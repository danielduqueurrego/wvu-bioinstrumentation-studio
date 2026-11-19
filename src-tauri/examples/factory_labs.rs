//! Maintainer tool for the factory (built-in) course labs in `profiles/`.
//! Run it through `scripts/update_factory_labs.ps1`; the logic lives in
//! `profiles.rs` (`factory_lab_check`, `rehash_factory_file`,
//! `promote_lab_file`), where unit tests cover it. Docs:
//! docs/LAB_CONFIGURATION.md, "How to change a built-in lab".
//!
//! Exit codes: 0 success, 1 problems found or the change was refused,
//! 2 usage error.
use std::{
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};
use wvu_bioinstrumentation_studio_lib::profiles::{
    factory_lab_check, factory_lab_source_line, factory_lab_sources, promote_lab_file,
    rehash_factory_file, FACTORY_LAB_FILE_SUFFIX, FACTORY_LAB_LOCK_FILE,
};

const USAGE: &str = "\
Factory lab file tool (WVU Bioinstrumentation Studio, teaching use only).

Usage:
  factory_labs check
      Validate every profiles/*.profile.json (and profiles/legacy/), its
      canonical hash, the MAJOR.MINOR.0 / built_in / locked rules, that each
      file is compiled into the application, profiles/factory-labs.lock.json,
      and that every published version is still held by a file.
  factory_labs fix <file> [--bump-minor]
      Re-hash a factory lab file in profiles/ after a hand edit and record
      its version in the lock. A version already in the lock never changes:
      --bump-minor publishes the change as the next MINOR version (X.Y+1.0).
  factory_labs promote <exported.lab.json> [--name <file-name>]
      Turn a lab exported from Lab Manager into a factory lab file with the
      next factory version (one MINOR above the published versions and the
      export's own version: a new lab exported as 1.0.x becomes 1.1.0), add
      it to the lock, and print the line that compiles it in. --name chooses
      the file name for a new lab.

fix and promote keep the published version a file held in
profiles/legacy/<name>_<version>.profile.json, taken from the file or, when
it has already changed, from the copy committed in Git (HEAD).

Exit codes: 0 success, 1 problems found or change refused, 2 usage error.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let profiles_dir = repository_profiles_dir();
    let lock_path = profiles_dir.join(FACTORY_LAB_LOCK_FILE);
    match args.first().map(String::as_str) {
        Some("check") if args.len() == 1 => check(&profiles_dir),
        Some("fix") => match parse_options(&args[1..], &["--bump-minor"], &[]) {
            Some((path, flags, _)) => fix(&profiles_dir, &lock_path, &path, !flags.is_empty()),
            None => usage_error(),
        },
        Some("promote") => match parse_options(&args[1..], &[], &["--name"]) {
            Some((path, _, name)) => promote(&profiles_dir, &lock_path, &path, name.as_deref()),
            None => usage_error(),
        },
        Some("help" | "--help" | "-h") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => usage_error(),
    }
}

fn usage_error() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

/// The repository's `profiles/` folder (the one compiled into this build).
fn repository_profiles_dir() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .unwrap_or(manifest_dir)
        .join("profiles")
}

/// The committed (HEAD) copy of `profiles/<file_name>`, byte for byte, or
/// `None` when Git, the commit, or the file is not available. Used only to
/// keep a published version whose file has already been changed.
fn committed_profile_text(profiles_dir: &Path, file_name: &str) -> Option<String> {
    let repository = profiles_dir.parent()?;
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["cat-file", "blob", &format!("HEAD:profiles/{file_name}")])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn print_kept_files(kept_files: &[PathBuf]) {
    for path in kept_files {
        println!(
            "  kept the published version it replaced as {}",
            path.display()
        );
    }
}

/// One path argument plus the allowed flags and at most one `--name` value.
fn parse_options(
    args: &[String],
    allowed_flags: &[&str],
    allowed_values: &[&str],
) -> Option<(PathBuf, Vec<String>, Option<String>)> {
    let mut path = None;
    let mut flags = Vec::new();
    let mut value = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if allowed_flags.contains(&arg.as_str()) {
            flags.push(arg.clone());
        } else if allowed_values.contains(&arg.as_str()) {
            if value.is_some() {
                return None;
            }
            value = Some(iter.next()?.clone());
        } else if arg.starts_with("--") || path.is_some() {
            return None;
        } else {
            path = Some(PathBuf::from(arg));
        }
    }
    Some((path?, flags, value))
}

fn check(profiles_dir: &Path) -> ExitCode {
    println!("Checking {}", profiles_dir.display());
    let report = factory_lab_check(profiles_dir, factory_lab_sources());
    for line in &report.labs {
        println!("  {line}");
    }
    if report.is_ok() {
        println!(
            "OK: {} factory lab file(s) checked; all are compiled in and recorded in {FACTORY_LAB_LOCK_FILE}.",
            report.labs.len()
        );
        ExitCode::SUCCESS
    } else {
        eprintln!("{} problem(s):", report.problems.len());
        for problem in &report.problems {
            eprintln!("  - {problem}");
        }
        ExitCode::from(1)
    }
}

fn fix(profiles_dir: &Path, lock_path: &Path, path: &Path, bump_minor: bool) -> ExitCode {
    // Only a current factory lab file is re-hashed; legacy files are frozen.
    let in_profiles_dir = path
        .parent()
        .map(|parent| {
            if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            }
        })
        .and_then(|parent| parent.canonicalize().ok())
        .zip(profiles_dir.canonicalize().ok())
        .is_some_and(|(parent, profiles)| parent == profiles);
    let named_like_a_lab = path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().ends_with(FACTORY_LAB_FILE_SUFFIX));
    if !in_profiles_dir || !named_like_a_lab {
        eprintln!(
            "error: {} is not a *{FACTORY_LAB_FILE_SUFFIX} file directly in {}",
            path.display(),
            profiles_dir.display()
        );
        return ExitCode::from(1);
    }
    let committed = |file_name: &str| committed_profile_text(profiles_dir, file_name);
    match rehash_factory_file(path, lock_path, bump_minor, &committed) {
        Ok(outcome) => {
            if outcome.profile_version != outcome.previous_version {
                println!(
                    "{}: {} -> {} (new MINOR version)",
                    outcome.profile_id, outcome.previous_version, outcome.profile_version
                );
            } else if bump_minor {
                println!(
                    "{} {}: content matches the recorded version or the version is not published yet, so it was not bumped.",
                    outcome.profile_id, outcome.profile_version
                );
            }
            println!(
                "{} {} canonical hash {}",
                outcome.profile_id, outcome.profile_version, outcome.canonical_hash
            );
            println!(
                "  file: {}",
                if outcome.file_changed {
                    "updated"
                } else {
                    "already up to date"
                }
            );
            println!(
                "  lock: {}",
                if outcome.lock_appended {
                    "new entry appended"
                } else {
                    "already recorded"
                }
            );
            print_kept_files(&outcome.kept_files);
            if outcome.file_changed || outcome.lock_appended || !outcome.kept_files.is_empty() {
                println!("Next: run `scripts/update_factory_labs.ps1 check` (it rebuilds with the new file), then the checks in docs/DEVELOPER_GUIDE.md. Commit the lab file, the lock, and any file kept in profiles/legacy/ together.");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

fn promote(profiles_dir: &Path, lock_path: &Path, path: &Path, name: Option<&str>) -> ExitCode {
    let committed = |file_name: &str| committed_profile_text(profiles_dir, file_name);
    match promote_lab_file(path, profiles_dir, lock_path, name, &committed) {
        Ok(outcome) => {
            println!("Wrote {}", outcome.path.display());
            println!(
                "  {} {} canonical hash {}",
                outcome.profile_id, outcome.profile_version, outcome.canonical_hash
            );
            if let Some(replaced) = &outcome.replaced_version {
                println!("  replaces factory version {replaced} (still recorded in the lock)");
            }
            print_kept_files(&outcome.kept_files);
            println!("  appended to {}", lock_path.display());
            println!("Next steps:");
            let compiled_in = factory_lab_sources()
                .iter()
                .any(|(file_name, _)| *file_name == outcome.file_name);
            if compiled_in {
                println!("  1. The file is already compiled in (FACTORY_LAB_FILES); a rebuild picks up the new version.");
            } else {
                println!("  1. Compile the new lab in: add this line at the end of FACTORY_LAB_FILES in src-tauri/src/profiles.rs (the list built_in_profiles() loads):");
                println!("         {}", factory_lab_source_line(&outcome.file_name));
                println!(
                    "     It expands to include_str!(\"../../profiles/{}\").",
                    outcome.file_name
                );
            }
            println!("  2. Run `scripts/update_factory_labs.ps1 check`.");
            println!("  3. Update the Factory labs table in docs/LAB_CONFIGURATION.md and CHANGELOG.md, then run the checks in docs/DEVELOPER_GUIDE.md.");
            println!("  4. Commit the lab file, the lock, and any file kept in profiles/legacy/ together.");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}
