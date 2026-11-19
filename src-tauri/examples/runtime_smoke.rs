//! Offline smoke test of the bundled Arduino runtime (maintainer tool).
//!
//! Extracts `src-tauri/resources/arduino-runtime.zip` into a scratch folder
//! with the application's own code (`arduino_runtime::prepare_from_archive`,
//! which also applies the native-USB core shim and writes the application's
//! `arduino-cli.yaml`), then compiles the controlled reference sketch with
//! `ArduinoCli::compile_to`, the application's compile command, while all
//! HTTP(S) traffic goes to a dead proxy. It never uploads anything and never
//! touches the real `%LOCALAPPDATA%\WVU-BMEG\rt` or application data:
//! LOCALAPPDATA, APPDATA, TEMP and TMP point into the scratch folder, so the
//! Arduino build cache and the application log stay there too.
//!
//! Usage (from the repository root):
//!   cargo run --manifest-path src-tauri/Cargo.toml --example runtime_smoke -- <scratch folder>
//!       [--root <runtime folder>] [--archive <arduino-runtime.zip>]
//!       [--manifest <arduino-runtime-manifest.json>]
//!
//! The runtime is extracted to `<scratch folder>\rt`, or to `--root`. The
//! bundled compiler needs a runtime root shorter than
//! `RUNTIME_ROOT_LENGTH_LIMIT` (60) characters; a longer one is refused before
//! anything is extracted (this tool never uses the application's
//! `%ProgramData%` fallback). A long scratch folder with a short
//! `--root` reproduces that fallback: long job and build-cache paths, as in a
//! long Windows profile, with a short runtime root.
//!
//! Exit codes: 0 the runtime compiled the sketch offline, 1 it did not,
//! 2 usage error. Docs: docs/DEVELOPER_GUIDE.md, "Rebuilding the Arduino
//! runtime".
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::atomic::AtomicBool,
    time::Instant,
};
use wvu_bioinstrumentation_studio_lib::{
    arduino_cli::{
        parse_compile_usage, ArduinoCli, NATIVE_USB_REFERENCE_BUILD_DEFINES, UNO_R4_WIFI_FQBN,
    },
    arduino_runtime::{
        path_length, prepare_from_archive, resolve_runtime_root, RuntimeError,
        RUNTIME_ROOT_LENGTH_LIMIT,
    },
    reference_firmware::{controlled_reference_source, source_hash, REFERENCE_SOURCE_SHA256},
};

const USAGE: &str = "\
Offline smoke test of the bundled Arduino runtime (WVU Bioinstrumentation Studio, teaching use only).

Usage:
  runtime_smoke <scratch folder> [--root <runtime folder>] [--archive <arduino-runtime.zip>]
                [--manifest <arduino-runtime-manifest.json>]

Extracts the runtime to <scratch folder>\\rt (or --root) with the application's
code and compiles the reference sketch offline with the application's compile
command. The runtime folder must be shorter than 60 characters.
Defaults: src-tauri/resources/arduino-runtime.zip and
src-tauri/resources/arduino-runtime-manifest.json.";

/// Nothing listens here; any download attempt fails at once.
const DEAD_PROXY: &str = "http://127.0.0.1:9";

struct Options {
    scratch: PathBuf,
    root: Option<PathBuf>,
    archive: PathBuf,
    manifest: PathBuf,
}

fn parse_options() -> Result<Options, String> {
    let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
    let mut scratch = None;
    let mut root = None;
    let mut archive = resources.join("arduino-runtime.zip");
    let mut manifest = resources.join("arduino-runtime-manifest.json");
    let mut arguments = env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--root") => {
                root = Some(
                    arguments
                        .next()
                        .map(PathBuf::from)
                        .ok_or("--root needs a path")?,
                );
            }
            Some("--archive") => {
                archive = arguments
                    .next()
                    .map(PathBuf::from)
                    .ok_or("--archive needs a path")?;
            }
            Some("--manifest") => {
                manifest = arguments
                    .next()
                    .map(PathBuf::from)
                    .ok_or("--manifest needs a path")?;
            }
            Some("-h" | "--help") => return Err(String::new()),
            _ if scratch.is_none() => scratch = Some(PathBuf::from(argument)),
            _ => {
                return Err(format!(
                    "unexpected argument {}",
                    argument.to_string_lossy()
                ))
            }
        }
    }
    let scratch = scratch.ok_or("the scratch folder is required")?;
    let absolute = |path: &Path| {
        std::path::absolute(path).map_err(|error| format!("{}: {error}", path.display()))
    };
    Ok(Options {
        scratch: absolute(&scratch)?,
        root: root.as_deref().map(absolute).transpose()?,
        archive,
        manifest,
    })
}

/// The real application folders this tool must never write to.
fn refuse_real_application_folders(scratch: &Path) -> Result<(), String> {
    let Some(local) = env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
        return Ok(());
    };
    let lower = |path: &Path| path.to_string_lossy().to_lowercase();
    let scratch_text = lower(scratch);
    for protected in [
        local.join("WVU-BMEG"),
        local.join("WVU Bioinstrumentation Studio"),
        local.join("Arduino15"),
    ] {
        let protected_text = lower(&protected);
        if scratch_text == protected_text
            || scratch_text.starts_with(&format!("{protected_text}\\"))
        {
            return Err(format!(
                "{} is inside {}; use a separate scratch folder.",
                scratch.display(),
                protected.display()
            ));
        }
    }
    Ok(())
}

fn run(options: &Options) -> Result<(), String> {
    refuse_real_application_folders(&options.scratch)?;
    if let Some(root) = &options.root {
        refuse_real_application_folders(root)?;
    }
    let home = options.scratch.join("home");
    let local = home.join("Local");
    let roaming = home.join("Roaming");
    let temp = home.join("Temp");
    let job = options.scratch.join("job");
    let sketch_dir = job.join("reference_unor4wifi");
    let output_dir = job.join("compile");
    if job.exists() {
        // A fresh job folder each run: Arduino CLI compiles every file in it.
        fs::remove_dir_all(&job).map_err(|error| format!("{}: {error}", job.display()))?;
    }
    for folder in [&local, &roaming, &temp, &sketch_dir, &output_dir] {
        fs::create_dir_all(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    }
    // Before any thread starts. Every Arduino CLI process inherits these.
    for (name, value) in [
        ("LOCALAPPDATA", local.as_os_str()),
        ("APPDATA", roaming.as_os_str()),
        ("TEMP", temp.as_os_str()),
        ("TMP", temp.as_os_str()),
        ("HTTP_PROXY", DEAD_PROXY.as_ref()),
        ("HTTPS_PROXY", DEAD_PROXY.as_ref()),
        ("ALL_PROXY", DEAD_PROXY.as_ref()),
        ("ARDUINO_NETWORK_PROXY", DEAD_PROXY.as_ref()),
    ] {
        env::set_var(name, value);
    }
    env::remove_var("NO_PROXY");
    println!("Scratch folder: {}", options.scratch.display());
    println!("Archive: {}", options.archive.display());
    println!(
        "Network: HTTP(S) proxy {DEAD_PROXY} (offline); LOCALAPPDATA, APPDATA, TEMP in the scratch folder"
    );

    let preferred = options
        .root
        .clone()
        .unwrap_or_else(|| options.scratch.join("rt"));
    // Never the %ProgramData% fallback: a too-long root is refused.
    let root = resolve_runtime_root(&preferred, false).map_err(|error| match error {
        RuntimeError::PathTooLong { path, length } => format!(
            "the runtime folder {} is {length} characters long; the included compiler needs fewer than {RUNTIME_ROOT_LENGTH_LIMIT}. This tool does not use the application's %ProgramData% fallback: give a shorter scratch folder or --root.",
            path.display()
        ),
        other => other.to_string(),
    })?;
    println!(
        "Runtime root: {} ({} characters; the compiler needs fewer than {RUNTIME_ROOT_LENGTH_LIMIT})",
        root.path().display(),
        path_length(root.path())
    );

    let started = Instant::now();
    let runtime = prepare_from_archive(&options.archive, &options.manifest, root.path())
        .map_err(|error| format!("runtime preparation failed: {error}"))?;
    println!(
        "Prepared runtime: Arduino CLI {}, UNO R4 core {} in {:.1} s",
        runtime.cli_version,
        runtime.core_version,
        started.elapsed().as_secs_f64()
    );

    let cli = ArduinoCli::for_runtime(&runtime);
    let version = cli
        .version()
        .map_err(|error| format!("arduino-cli version failed: {error}"))?;
    println!("arduino-cli version: {}", version.stdout.trim());
    let core = cli
        .uno_r4_core_version()
        .map_err(|error| format!("arduino-cli core list failed: {error}"))?;
    println!("core list: arduino:renesas_uno {core}");
    if core != runtime.core_version {
        return Err(format!(
            "core list reports {core}, the manifest names {}",
            runtime.core_version
        ));
    }

    let source = controlled_reference_source();
    let source_sha256 = source_hash(source);
    if source_sha256 != REFERENCE_SOURCE_SHA256 {
        return Err(format!(
            "embedded sketch SHA-256 {source_sha256} differs from REFERENCE_SOURCE_SHA256"
        ));
    }
    let repository_sketch = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../firmware/reference_unor4wifi/reference_unor4wifi.ino");
    match fs::read(&repository_sketch) {
        Ok(bytes) if bytes == source => {
            println!(
                "firmware/reference_unor4wifi/reference_unor4wifi.ino matches the embedded sketch"
            )
        }
        Ok(_) => {
            return Err(format!(
                "{} differs from the sketch compiled into this build; rebuild the example",
                repository_sketch.display()
            ))
        }
        Err(error) => println!("note: {} not read: {error}", repository_sketch.display()),
    }
    let sketch = sketch_dir.join("reference_unor4wifi.ino");
    fs::write(&sketch, source).map_err(|error| format!("{}: {error}", sketch.display()))?;
    println!("reference_unor4wifi.ino SHA-256: {source_sha256}");
    println!(
        "compile --fqbn {UNO_R4_WIFI_FQBN} --build-property \"build.defines={NATIVE_USB_REFERENCE_BUILD_DEFINES}\" --output-dir {} {}",
        output_dir.display(),
        sketch_dir.display()
    );

    let started = Instant::now();
    let log = cli
        .compile_to(&sketch_dir, &output_dir, &AtomicBool::new(false))
        .map_err(|error| format!("compile could not run: {error}"))?;
    if !log.succeeded() {
        let tail = |text: &str| {
            let lines: Vec<&str> = text.lines().collect();
            lines[lines.len().saturating_sub(25)..].join("\n")
        };
        return Err(format!(
            "compile failed (exit {:?})\n--- stdout ---\n{}\n--- stderr ---\n{}",
            log.exit_code,
            tail(&log.stdout),
            tail(&log.stderr)
        ));
    }
    let usage = parse_compile_usage(&log.stdout);
    let binary = output_dir.join("reference_unor4wifi.ino.bin");
    let binary_bytes =
        fs::read(&binary).map_err(|error| format!("{}: {error}", binary.display()))?;
    println!(
        "Compiled in {:.1} s: flash {} bytes ({}%), RAM {} bytes ({}%)",
        started.elapsed().as_secs_f64(),
        usage
            .sketch_bytes
            .map_or("?".into(), |value| value.to_string()),
        usage
            .sketch_percent
            .map_or("?".into(), |value| value.to_string()),
        usage
            .ram_bytes
            .map_or("?".into(), |value| value.to_string()),
        usage
            .ram_percent
            .map_or("?".into(), |value| value.to_string()),
    );
    println!(
        "reference_unor4wifi.ino.bin: {} bytes, SHA-256 {} (it embeds source paths, so it differs per folder)",
        binary_bytes.len(),
        source_hash(&binary_bytes)
    );
    if usage.sketch_bytes.is_none() || usage.ram_bytes.is_none() {
        return Err("the compile output has no flash/RAM usage lines".into());
    }
    if !log.stderr.trim().is_empty() {
        println!("compile stderr:\n{}", log.stderr.trim());
    }
    let downloads = runtime.root.join("downloads");
    let downloaded = fs::read_dir(&downloads)
        .map(|entries| entries.count())
        .unwrap_or(0);
    if downloaded > 0 {
        return Err(format!(
            "{} has {downloaded} entries: something was downloaded",
            downloads.display()
        ));
    }
    println!("Nothing was downloaded ({} is empty).", downloads.display());
    Ok(())
}

fn main() -> ExitCode {
    let options = match parse_options() {
        Ok(options) => options,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("error: {message}\n");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&options) {
        Ok(()) => {
            println!("PASS: the bundled runtime compiled the reference sketch offline.");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("FAIL: {message}");
            ExitCode::FAILURE
        }
    }
}
