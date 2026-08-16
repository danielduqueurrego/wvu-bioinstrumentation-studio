//! App-owned, offline Arduino tool runtime. Production never relies on a
//! student's Arduino IDE installation, PATH, or global Arduino15 directory.
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::OnceLock,
};
use tauri::Manager;
use zip::ZipArchive;

static ACTIVE_RUNTIME: OnceLock<RuntimePaths> = OnceLock::new();

#[derive(Clone, Debug)]
pub struct RuntimePaths {
    pub executable: PathBuf,
    pub config_file: PathBuf,
    pub root: PathBuf,
    pub cli_version: String,
    pub core_version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct RuntimeManifest {
    pub runtime_schema: u32,
    pub arduino_cli: String,
    pub renesas_uno_core: String,
    pub bundle_revision: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct RuntimeStatus {
    pub ready: bool,
    pub cli_version: String,
    pub core_version: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("Bundled Arduino tools are missing. Reinstall WVU Bioinstrumentation Studio or contact your instructor.")]
    MissingBundle,
    #[error("Could not prepare the bundled Arduino tools: {0}")]
    Io(#[from] std::io::Error),
    #[error("Bundled Arduino runtime metadata is invalid: {0}")]
    Manifest(#[from] serde_json::Error),
    #[error("Bundled Arduino runtime is incompatible or incomplete. Reinstall WVU Bioinstrumentation Studio.")]
    InvalidBundle,
}

pub fn active_runtime() -> Option<&'static RuntimePaths> {
    ACTIVE_RUNTIME.get()
}

pub fn prepare_runtime(app: &tauri::AppHandle) -> Result<RuntimeStatus, RuntimeError> {
    if let Some(paths) = active_runtime() {
        return Ok(RuntimeStatus {
            ready: true,
            cli_version: paths.cli_version.clone(),
            core_version: paths.core_version.clone(),
            message: "Arduino tools are ready.".into(),
        });
    }
    let resource_root = app
        .path()
        .resource_dir()
        .map_err(|_| RuntimeError::MissingBundle)?
        .join("resources");
    let archive = resource_root.join("arduino-runtime.zip");
    let manifest = resource_root.join("arduino-runtime-manifest.json");
    // The bundled Renesas GCC toolchain has path-length-sensitive C++ multilib
    // discovery on Windows. Keep its app-owned extraction root deliberately
    // short while ordinary application data stays in Tauri's normal directory.
    let local_data_root = app
        .path()
        .app_local_data_dir()
        .map_err(|_| RuntimeError::MissingBundle)?
        .parent()
        .map(Path::to_path_buf)
        .ok_or(RuntimeError::InvalidBundle)?;
    let destination = local_data_root.join("WVU-BMEG").join("rt");
    let paths = prepare_from_archive(&archive, &manifest, &destination)?;
    let _ = ACTIVE_RUNTIME.set(paths.clone());
    Ok(RuntimeStatus {
        ready: true,
        cli_version: paths.cli_version,
        core_version: paths.core_version,
        message: "Arduino tools are ready.".into(),
    })
}

pub fn prepare_from_archive(
    archive: &Path,
    manifest_path: &Path,
    destination: &Path,
) -> Result<RuntimePaths, RuntimeError> {
    let source_manifest = read_manifest_file(manifest_path)?;
    let destination_manifest = read_manifest(destination).ok();
    if destination_manifest.as_ref() != Some(&source_manifest) {
        if destination.exists() {
            fs::remove_dir_all(destination)?;
        }
        extract_archive(archive, destination)?;
    }
    let executable = destination.join("arduino-cli.exe");
    let data = destination.join("data");
    if !executable.is_file()
        || !data.join("packages/arduino/hardware/renesas_uno").is_dir()
        || !data.join("packages/arduino/tools").is_dir()
    {
        return Err(RuntimeError::InvalidBundle);
    }
    apply_native_usb_core_compatibility_patch(destination, &source_manifest)?;
    let config_file = destination.join("arduino-cli.yaml");
    let normalized_data = data.to_string_lossy().replace('\\', "/");
    let normalized_downloads = destination
        .join("downloads")
        .to_string_lossy()
        .replace('\\', "/");
    let normalized_user = destination
        .join("sketchbook")
        .to_string_lossy()
        .replace('\\', "/");
    fs::create_dir_all(destination.join("downloads"))?;
    fs::create_dir_all(destination.join("sketchbook"))?;
    fs::write(
        &config_file,
        format!(
            "directories:\n  data: {normalized_data}\n  downloads: {normalized_downloads}\n  user: {normalized_user}\n"
        ),
    )?;
    Ok(RuntimePaths {
        executable,
        config_file,
        root: destination.to_path_buf(),
        cli_version: source_manifest.arduino_cli,
        core_version: source_manifest.renesas_uno_core,
    })
}

/// The pinned Arduino Renesas UNO core routes the C-library `_write` hook through
/// `Serial.write_raw`, but its native-USB `SerialUSB` class omits that member.
/// The standard UNO R4 WiFi build defines `NO_USB`, so the upstream omission is
/// normally hidden. WVU's controlled firmware deliberately uses the RA4M1 native
/// USB CDC path to avoid the proven intermittent RA4M1-to-ESP32 bridge stall.
///
/// Apply the smallest possible, version-pinned compatibility shim to the
/// app-owned extracted runtime. It is idempotent, never modifies an Arduino IDE
/// installation, and rejects an unexpected upstream core layout rather than
/// silently patching unknown third-party source.
fn apply_native_usb_core_compatibility_patch(
    destination: &Path,
    manifest: &RuntimeManifest,
) -> Result<(), RuntimeError> {
    if manifest.renesas_uno_core != "1.6.0" {
        return Err(RuntimeError::InvalidBundle);
    }
    let header = destination
        .join("data/packages/arduino/hardware/renesas_uno")
        .join(&manifest.renesas_uno_core)
        .join("cores/arduino/usb/SerialUSB.h");
    let source = fs::read_to_string(&header).map_err(|_| RuntimeError::InvalidBundle)?;
    if source.contains("size_t write_raw(uint8_t *p, size_t len)") {
        return Ok(());
    }
    let marker = "    virtual size_t write(const uint8_t *p, size_t len) override;";
    if !source.contains(marker) {
        return Err(RuntimeError::InvalidBundle);
    }
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let replacement = format!(
        "{marker}{newline}    // WVU native-USB compatibility shim for the pinned core's libc _write hook.{newline}    size_t write_raw(uint8_t *p, size_t len) {{ return write(p, len); }}"
    );
    fs::write(header, source.replacen(marker, &replacement, 1))?;
    Ok(())
}

fn read_manifest(root: &Path) -> Result<RuntimeManifest, RuntimeError> {
    read_manifest_file(&root.join("runtime-manifest.json"))
}

fn read_manifest_file(path: &Path) -> Result<RuntimeManifest, RuntimeError> {
    if !path.is_file() {
        return Err(RuntimeError::MissingBundle);
    }
    let manifest: RuntimeManifest = serde_json::from_slice(&fs::read(path)?)?;
    if manifest.runtime_schema != 1
        || manifest.arduino_cli.trim().is_empty()
        || manifest.renesas_uno_core.trim().is_empty()
    {
        return Err(RuntimeError::InvalidBundle);
    }
    Ok(manifest)
}

fn extract_archive(archive: &Path, destination: &Path) -> Result<(), RuntimeError> {
    let archive = fs::File::open(archive).map_err(|_| RuntimeError::MissingBundle)?;
    let mut archive = ZipArchive::new(archive).map_err(|_| RuntimeError::InvalidBundle)?;
    fs::create_dir_all(destination)?;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|_| RuntimeError::InvalidBundle)?;
        let Some(relative) = file.enclosed_name() else {
            return Err(RuntimeError::InvalidBundle);
        };
        let output = destination.join(relative);
        if file.is_dir() {
            fs::create_dir_all(output)?;
        } else {
            let parent = output.parent().ok_or(RuntimeError::InvalidBundle)?;
            fs::create_dir_all(parent)?;
            let mut destination_file = fs::File::create(output)?;
            io::copy(&mut file, &mut destination_file)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use zip::{write::SimpleFileOptions, ZipWriter};

    #[test]
    fn runtime_bootstrap_is_idempotent_and_generates_an_app_owned_config() {
        let source = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let destination = tempdir().unwrap_or_else(|error| panic!("{error}"));
        let manifest = r#"{"runtime_schema":1,"arduino_cli":"1.5.2-rc.1","renesas_uno_core":"1.6.0","bundle_revision":1}"#;
        let manifest_path = source.path().join("arduino-runtime-manifest.json");
        fs::write(&manifest_path, manifest).unwrap_or_else(|error| panic!("{error}"));
        let archive_path = source.path().join("arduino-runtime.zip");
        let mut archive = ZipWriter::new(
            fs::File::create(&archive_path).unwrap_or_else(|error| panic!("{error}")),
        );
        let options = SimpleFileOptions::default();
        for (name, bytes) in [
            ("runtime-manifest.json", manifest.as_bytes()),
            ("arduino-cli.exe", b"test".as_slice()),
            (
                "data/packages/arduino/hardware/renesas_uno/1.6.0/.keep",
                b"".as_slice(),
            ),
            (
                "data/packages/arduino/hardware/renesas_uno/1.6.0/cores/arduino/usb/SerialUSB.h",
                b"class SerialUSB {\n    virtual size_t write(const uint8_t *p, size_t len) override;\n    using Print::write;\n};\n".as_slice(),
            ),
            ("data/packages/arduino/tools/.keep", b"".as_slice()),
        ] {
            archive
                .start_file(name, options)
                .unwrap_or_else(|error| panic!("{error}"));
            archive
                .write_all(bytes)
                .unwrap_or_else(|error| panic!("{error}"));
        }
        archive.finish().unwrap_or_else(|error| panic!("{error}"));
        let root = destination.path().join("runtime");
        let first = prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        let second = prepare_from_archive(&archive_path, &manifest_path, &root)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(first.config_file, second.config_file);
        assert!(first.config_file.is_file());
        assert!(fs::read_to_string(first.config_file)
            .unwrap_or_default()
            .contains("directories:"));
        let serial_usb = root
            .join("data/packages/arduino/hardware/renesas_uno/1.6.0/cores/arduino/usb/SerialUSB.h");
        assert!(fs::read_to_string(serial_usb)
            .unwrap_or_default()
            .contains("size_t write_raw(uint8_t *p, size_t len)"));
    }
}
