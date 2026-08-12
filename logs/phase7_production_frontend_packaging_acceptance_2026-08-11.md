# Phase 7 production frontend packaging acceptance — 2026-08-11

## Status

Corrected production build pending focused manual launch and installed-application verification.
No final distribution hashes or release artifacts were regenerated for this investigation.

## Root cause

The executable launched for the Board-recovery check was produced with:

```powershell
cargo build --manifest-path src-tauri\Cargo.toml --release
```

That direct Cargo command compiled the Rust binary but bypassed the Tauri production build pipeline
that packages the static Svelte frontend. The resulting executable contained and attempted to load
the development URL `http://localhost:1420`, causing `ERR_CONNECTION_REFUSED` when no Vite server
was running.

## Reviewed configuration

| Setting | Resolved value |
| --- | --- |
| `build.devUrl` | `http://localhost:1420` — development only |
| `build.frontendDist` | `../build` relative to `src-tauri`, resolving to repository `build/` |
| `build.beforeBuildCommand` | `npm run build` |
| Canonical production command | `npm run tauri build` |

The static `build/` directory contains `index.html` and compiled Svelte JavaScript, CSS, and asset
files. Tauri must package that directory for a student release; direct Cargo artifacts are not
accepted release executables.

## Guard

`scripts/build_student_release.ps1` now rejects a URL-valued `frontendDist`, requires the frontend
build command, and verifies that the resolved local frontend distribution contains `index.html`
before building installers. The frontend test suite also verifies the production configuration.

## Required manual result

After a Tauri production build, launch the exact `src-tauri/target/release` executable with no
Vite/Node development server running. Confirm the WVU UI loads from bundled assets, never requests
localhost, and the installed NSIS application behaves identically.
