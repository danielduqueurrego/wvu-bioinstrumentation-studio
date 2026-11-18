use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    embed_git_revision();
    tauri_build::build()
}

/// Build provenance: `GIT_COMMIT` (read with `option_env!` and recorded as
/// `git_revision` in firmware restore logs) is the source commit when the
/// build runs inside a Git work tree with `git` available. An explicit
/// `GIT_COMMIT` from the environment wins. Without either, the value stays
/// unset (`None`): a missing Git never fails the build.
fn embed_git_revision() {
    println!("cargo:rerun-if-env-changed=GIT_COMMIT");
    if std::env::var("GIT_COMMIT").is_ok_and(|value| !value.trim().is_empty()) {
        return;
    }
    let Some(manifest_dir) = std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from) else {
        return;
    };
    let Some(commit) = git(&manifest_dir, &["rev-parse", "HEAD"]) else {
        return;
    };
    if !matches!(commit.len(), 40 | 64) || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
        return;
    }
    println!("cargo:rustc-env=GIT_COMMIT={commit}");
    // Rebuild when HEAD moves: HEAD itself, the branch it names, and the
    // packed refs. Only existing files are watched, because Cargo reruns a
    // build script on every build when a watched path is missing.
    let mut watched = vec!["HEAD".to_owned(), "packed-refs".to_owned()];
    if let Some(reference) = git(&manifest_dir, &["symbolic-ref", "-q", "HEAD"]) {
        watched.push(reference);
    }
    for name in watched {
        if let Some(path) = git(&manifest_dir, &["rev-parse", "--git-path", &name]) {
            let path = manifest_dir.join(path);
            if path.is_file() {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
}

fn git(directory: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}
