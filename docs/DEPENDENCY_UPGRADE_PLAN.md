# Dependency upgrade plan

**Status: plan only. Nothing has been upgraded.** Versions, advisories and upstream notes below were checked on **2026-11-19** against branch `release-1.1-prep` at HEAD `66af30a` (the firmware 0x00010007 commit, which landed during the check; counts first taken at `507673b` were redone at `66af30a`). Package registries move every week (`zip` 9.7.2 and 9.7.3 and `sv` 1.6.0 were all published on the day of this check), so re-run the inventory commands in [section 2.6](#26-refreshing-this-inventory) before starting any step and adjust the target versions.

Statements marked **Uncertain** could not be confirmed from upstream sources or local tests. Statements marked **Check** are things to look at while doing the step.

Contents:

1. [Purpose, principles and verification matrix](#1-purpose-principles-and-verification-matrix)
2. [Current inventory](#2-current-inventory-2026-11-19)
3. [Step 1: Tauri 2.11 to 2.12](#3-step-1-tauri-211-to-212)
4. [Step 2: zip 2.4 to 9.7](#4-step-2-zip-24-to-97)
5. [Step 3: frontend tooling on SvelteKit 2](#5-step-3-frontend-tooling-on-sveltekit-2)
6. [Step 4: SvelteKit 3 and adapter-static 4](#6-step-4-sveltekit-3-and-adapter-static-4)
7. [Optional step 5: chrono to jiff (or time)](#7-optional-step-5-chrono-to-jiff-or-time)
8. [Order, rollback, acceptance criteria and effort](#8-order-rollback-acceptance-criteria-and-effort)
9. [References](#9-references)

## 1. Purpose, principles and verification matrix

### Purpose

Several dependencies are one or more major versions behind. Some of them carry security advisories:

- `zip` 2.4.2 has six RustSec advisories. CI's blocking `cargo audit` currently ignores them by ID.
- `happy-dom` 20.14.6 and `@sveltejs/kit` 2.70.4 have npm advisories. Both are development dependencies.
- `chrono` 0.4.45 is flagged as unmaintained.

This plan gives the maintainer and future agents an order of work, the exact changes each step needs, how to verify each step, and how to back it out. It is written for someone who works on this project part time. Each step can be picked up on its own, weeks apart.

### Principles

- **One step per branch.** Create each step's branch from the current integration branch (`master` once `release-1.1-prep` is merged). A step may contain several commits, but every commit must pass the full gate on its own.
- **Never mix with feature work.** Do not upgrade on a branch that also changes firmware, the protocol, labs, the UI or release metadata. A failed hardware test must point at exactly one cause.
- **Run the full gate after every commit.** Use the gate from [AGENTS.md](../AGENTS.md) and [Developer Guide](DEVELOPER_GUIDE.md#setup-and-checks) (V1 below). Build the frontend before `npm test`, because `src/lib/release-config.test.ts` requires `build/index.html`.
- **Tauri npm packages and Rust crates move together.**
  - `scripts/check_release_consistency.ps1` fails when `@tauri-apps/api` and the `tauri` crate are on different major.minor releases, and likewise for `@tauri-apps/plugin-<name>` and `tauri-plugin-<name>`.
  - Update both lockfiles in the same commit. Dependabot opens separate npm and Cargo pull requests: close those and do the update by hand.
- **Re-sync the custom NSIS template on every Tauri CLI change.**
  - The consistency script also fails when `@tauri-apps/cli` or `@tauri-apps/cli-win32-x64-msvc` differs from the version recorded in [`src-tauri/nsis/README.md`](../src-tauri/nsis/README.md).
  - Follow [Re-syncing the NSIS template](DEVELOPER_GUIDE.md#re-syncing-the-nsis-template).
- **Keep the project invariants of [AGENTS.md](../AGENTS.md)** in every step. These include:
  - raw BMEG samples stay authoritative;
  - outputs stay safe LOW;
  - external processes keep `CREATE_NO_WINDOW`;
  - Tauri command argument names stay aligned with their frontend callers.
- **Use the real runtime only where a step needs it.**
  - `runtime_smoke`, production builds and hardware tests use the real hash-pinned `src-tauri/resources/arduino-runtime.zip`.
  - The CI placeholder ZIP is only for Cargo source checks.
- **Do not change the toolchain in the same step.** `rust-toolchain.toml` stays at `1.100.0`. It already satisfies every minimum Rust version below: Tauri 1.90, `zip` 1.88, `jiff` 1.85.
- **Timing (decided 2026-11-19).** Start after 1.1.0 is released, in the order of [section 8](#order): Tauri 2.12, `zip` 9.7, frontend tooling (`happy-dom` may go first), SvelteKit 3; chrono stays deferred.
  - Why: 1.1 is in hardware verification, and Step 1 changes the installer template whose 1.0.x upgrade tests are still open. Keeping the upgrades out of 1.1 means a failed 1.1 hardware or upgrade test points at the 1.1 changes only.
  - The advisories do not justify holding 1.1 for them. The app never calls `zip`'s `extract()` (the symlink and permission advisories): `arduino_runtime.rs` extracts with its own loop over `enclosed_name()` and plain file writes, only from the bundled runtime archive, and uses neither AES entries nor `ZipStreamReader`. The Tauri IPC fix matters for multi-webview apps; this app has one webview. The npm advisories are in development-only packages.
  - The alternative that was not chosen: Steps 1 and 2 before the 1.1 release, so the Sandbox upgrade tests and the 1.1 hardware checklist would cover them once instead of twice.

### Verification matrix

Run every command from the repository root in PowerShell 7. In a script, check `$LASTEXITCODE` after each native command; a later success can hide an earlier failure.

**V1 – Gate** (from [AGENTS.md](../AGENTS.md) and the Developer Guide):

```powershell
$ErrorActionPreference = 'Stop'
function Invoke-Step([string]$Name, [scriptblock]$Command) {
  & $Command
  if ($LASTEXITCODE -ne 0) { throw "$Name failed with exit code $LASTEXITCODE" }
}
Invoke-Step 'npm ci'      { npm ci }
Invoke-Step 'consistency' { pwsh -NoProfile -File scripts\check_release_consistency.ps1 }
Invoke-Step 'check'       { npm run check }
Invoke-Step 'build'       { npm run build }
Invoke-Step 'test'        { npm test }
Invoke-Step 'fmt'         { cargo fmt --manifest-path src-tauri\Cargo.toml -- --check }
Invoke-Step 'cargo check' { cargo check --manifest-path src-tauri\Cargo.toml --all-targets --locked }
Invoke-Step 'cargo test'  { cargo test --manifest-path src-tauri\Cargo.toml --locked }
Invoke-Step 'clippy'      { cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets --locked -- -D warnings }
```

Run `npm ci` only after the step's lockfile changes are written. The upgrade commands (`npm install`, `cargo update`) are what change the lockfiles.

**V2 – Audit.**

- `npm audit --omit=dev --audit-level=high` (blocking in CI).
- `npm audit --audit-level=high` (informational).
- `cargo audit` in `src-tauri`, with the same `--ignore` list as `.github/workflows/audit.yml` until Step 2 removes it. CI pins cargo-audit `0.23.0`.

**V3 – Sandbox UI check.**

- Run `pwsh -NoProfile -File scripts\start_dev_sandbox.ps1 -RemoteDebuggingPort 9222`, then use the Simulator:
  - start and stop a timed recording and an "until stopped" recording;
  - check that the live plots draw and that the Last recording card appears;
  - change the lab;
  - open Instructor **Manage Labs**, then use import (open dialog), export / "Save as course default file…" (save dialog) and a retire confirmation (confirm dialog);
  - open the calibration dialog;
  - use the Student "Install lab file…" button;
  - check that the header logo shows (`server.fs.allow`);
  - attach DevTools to the page target listed at `http://127.0.0.1:9222/json` and check that the console shows no errors or CSP violations.
- For a visual comparison, take before/after screenshots at 1366 × 690 as described in [Driving and screenshotting the window](DEVELOPER_GUIDE.md#driving-and-screenshotting-the-window).
- Never use real app data.

**V4 – Runtime smoke.** Run `cargo run --manifest-path src-tauri\Cargo.toml --example runtime_smoke -- C:\wvu-smoke` with the real ZIP. It must exit 0, print Arduino CLI `1.6.0` and core `1.6.0`, and download nothing. Delete `C:\wvu-smoke` afterwards (about 700 MB).

**V5 – Release build.**

- Run `npm run tauri build` with the real ZIP.
- Start `src-tauri\target\release\wvu_bioinstrumentation_studio.exe` with no Vite server running. The window must load, and no console window may appear.
- Do not distribute this build. Delete `src-tauri\target\release\bundle` afterwards.

**V6 – Upgrade tests.** Run [Upgrade from the previous release](RELEASE_CHECKLIST.md#upgrade-from-the-previous-release) in Windows Sandbox: NSIS to NSIS from the default folder and from a custom folder, MSI to MSI, and MSI to NSIS. Run them from 1.0.2, and also from 1.1.0 once it is released.

**V7 – Hardware smoke.** Use the real runtime, one UNO R4 WiFi, and one application and serial owner at a time. This is a subset of [Hardware acceptance](RELEASE_CHECKLIST.md#hardware-acceptance):

- board discovery and **Verify**;
- **Restore WVU Firmware**, offline;
- a short ECG recording, both timed and stopped with **Stop recording**;
- D4/D5/D6 LOW after Stop;
- no console windows during Verify, Restore or Start.

| Check | Step 1 Tauri | Step 2 zip | Step 3 tooling | Step 4 SvelteKit 3 | Step 5 chrono |
| --- | --- | --- | --- | --- | --- |
| V1 Gate | Required | Required | Required | Required | Required |
| V2 Audit | Required | Required (without ignores) | Required | Required | Required |
| V3 Sandbox UI | Required (dialogs) | Optional (Repair, short `-SandboxRoot`) | Required (visual) | Required (visual) | Required (metadata) |
| V4 Runtime smoke | No | **Required** | No | No | No |
| V5 Release build | Required (NSIS and MSI) | Required | Required | Required | Required |
| V6 Upgrade tests | **Required** | No | No | No | No |
| V7 Hardware smoke | Required | **Required** (Repair, then offline Restore) | Launch only | Required (one recording) | Required (one recording) |

## 2. Current inventory (2026-11-19)

The working tree versions are those in `package-lock.json` and `src-tauri/Cargo.lock`. "Latest" means the newest stable release on npm or crates.io on 2026-11-19.

### 2.1 npm direct dependencies

| Package | `package.json` range | Locked | Latest | Target | Step |
| --- | --- | --- | --- | --- | --- |
| `@tauri-apps/api` (prod) | `^2` | 2.11.1 | 2.12.5 | 2.12.5 | 1 |
| `@tauri-apps/plugin-dialog` (prod) | `^2.7.2` | 2.7.2 | 2.8.4 | 2.8.4 | 1 |
| `uplot` (prod) | `^1.6.32` | 1.6.32 | 1.6.32 | unchanged | – |
| `@tauri-apps/cli` | `^2` | 2.11.4 | 2.12.6 | 2.12.6 | 1 |
| `typescript` | `~5.6.2` | 5.6.3 | **7.0.2** | **6.0.3** (not 7, see Step 3) | 3 |
| `vite` | `^6.0.3` | 6.4.6 | 8.3.8 | 8.3.x | 3 |
| `@sveltejs/vite-plugin-svelte` | `^5.0.0` | 5.1.1 | 7.3.1 | 7.3.x | 3 |
| `vitest` | `^4.1.10` | 4.1.12 | 5.1.1 | 5.1.x | 3 |
| `happy-dom` | `^20.11.1` | 20.14.6 | 21.0.4 | 21.0.4 or later | 3 |
| `svelte` | `^5.0.0` | 5.56.8 | 5.58.0 | 5.58.x | 3 |
| `svelte-check` | `^4.0.0` | 4.7.4 | 4.7.8 | 4.7.8 | 3 |
| `@sveltejs/kit` | `^2.9.0` | 2.70.4 (latest 2.x) | 3.4.0 | 3.4.x | 4 |
| `@sveltejs/adapter-static` | `^3.0.6` | 3.0.11 | 4.0.1 | 4.0.x | 4 |

`engines.node` is `>=22`. The local Node is 22.23.2 with npm 10.9.8, and CI uses `node-version: 22`.

### 2.2 npm key transitive packages

- `@tauri-apps/cli-win32-x64-msvc` 2.11.4: its binary embeds the NSIS template.
- `esbuild` 0.25.12 and `rollup` 4.62.4: Vite 6's bundlers. Vite 8 replaces them with Rolldown and Oxc.
- `@sveltejs/vite-plugin-svelte-inspector` 4.0.1: merged into vite-plugin-svelte 7.
- `cookie` 0.6.0: has an advisory; SvelteKit 3 uses `cookie` 2.
- `@vitest/runner` and `@vitest/expect` 4.1.12.
- `@types/node` 26.1.2: transitive only, see TypeScript 6 below.
- `devalue` 5.9.4.

### 2.3 Cargo direct dependencies (`src-tauri/Cargo.toml`)

| Crate | Requirement | Locked | Latest | Target | Step |
| --- | --- | --- | --- | --- | --- |
| `tauri` | `2` | 2.11.5 | 2.12.7 | 2.12.x | 1 |
| `tauri-build` (build) | `2` | 2.6.3 | 2.7.6 | 2.7.x | 1 |
| `tauri-plugin-dialog` | `2` | 2.7.2 | 2.8.4 | 2.8.x | 1 |
| `zip` | `2`, `default-features = false`, `["deflate"]` | 2.4.2 | 9.7.3 | 9.7.x (at least 9.0.5) | 2 |
| `chrono` | `0.4.45`, `["serde"]` | 0.4.45 | 0.4.45 (unmaintained) | `jiff` 1.0.x or `time` 0.3.x | 5 (optional) |
| `serde` | `1` | 1.0.229 | 1.0.229 | – | – |
| `serde_json` | `1`, `["raw_value"]` | 1.0.151 | 1.0.154 | in range | not in plan |
| `thiserror` | `2.0.19` | 2.0.19 | 2.0.21 | in range | not in plan |
| `serialport` | `4.9.0` | 4.9.0 | 4.10.1 | in range | not in plan (needs V7) |
| `fs2` | `0.4.3` | 0.4.3 | 0.4.3 (last release 2018) | – | – |
| `sha2` | `0.10` | 0.10.9 | 0.11.1 | 0.x major | not in plan |
| `windows-sys` (Windows) | `0.61` | 0.61.2 | 0.61.2 | – | – |
| `tempfile` (dev) | `3.27.0` | 3.27.0 | 3.27.0 | – | – |

### 2.4 Cargo key transitive crates

- **Tauri runtime:** `tauri-runtime` 2.11.3, `tauri-runtime-wry` 2.11.4, `tauri-utils` 2.9.3, `tauri-codegen`/`tauri-macros`/`tauri-plugin` 2.6.3, `wry` 0.55.1, `tao` 0.35.3, `webview2-com` 0.38.2, `tauri-plugin-fs` 2.5.1 (through the dialog plugin), `rfd` 0.16.0.
- **zip stack:** `flate2` 1.1.9 with the `miniz_oxide` 0.8.9 backend, `zopfli` 0.8.3 and `crc32fast` 1.5.0.
- **`time` 0.3.55:** already in the tree through `cookie` (`tauri`, `wry`) and `plist` (`tauri-utils`). This matters for Step 5.
- **Size:** 450 crates in `Cargo.lock`.

### 2.5 Audit status

**npm audit** (2026-11-19):

- `--omit=dev`: **0 vulnerabilities**. The production dependencies (`@tauri-apps/api`, `@tauri-apps/plugin-dialog`, `uplot`) are clean.
- All dependencies: **4 packages, 2 high and 2 low**. All are development-only.

| Package | Advisory | Severity | Affected range | Fixed by |
| --- | --- | --- | --- | --- |
| `happy-dom` 20.14.6 | [GHSA-834v-8h66-3h79](https://github.com/advisories/GHSA-834v-8h66-3h79) (incomplete fix for CVE-2026-33943, host code execution) | high | >=17.0.0 <=21.0.3 | 21.0.4 (Step 3) |
| `happy-dom` 20.14.6 | [GHSA-c8p4-j97h-3vg8](https://github.com/advisories/GHSA-c8p4-j97h-3vg8) (SSRF via iframe and stylesheet) | high | <=20.14.6 | 21.0.x (Step 3) |
| `@sveltejs/kit` 2.70.4 | [GHSA-3r8g-jcqq-3p5j](https://github.com/advisories/GHSA-3r8g-jcqq-3p5j) (DoS in remote-function `form` parsing) | high | >=2.42.0 <=3.1.0 | 3.1.1 or later (Step 4) |
| `@sveltejs/kit` 2.70.4 | [GHSA-6cf5-gc4q-fm3x](https://github.com/advisories/GHSA-6cf5-gc4q-fm3x) (`query.batch` collision) | moderate | >=2.60.1 <=3.1.0 | 3.1.1 or later (Step 4) |
| `@sveltejs/kit` 2.70.4 | [GHSA-pfhg-q3qv-9q6r](https://github.com/advisories/GHSA-pfhg-q3qv-9q6r) (redirect via enhanced form) | low | <3.0.0 | 3.x (Step 4) |
| `@sveltejs/kit` 2.70.4 | [GHSA-27mw-82g8-phjj](https://github.com/advisories/GHSA-27mw-82g8-phjj) (XSS via cache-control in a prerendered page) | low | <3.0.0 | 3.x (Step 4) |
| `cookie` 0.6.0 (via Kit) | [GHSA-pxg6-pf52-xh8x](https://github.com/advisories/GHSA-pxg6-pf52-xh8x) | low | <0.7.0 | Kit 3 (Step 4) |
| `@sveltejs/adapter-static` 3.0.11 | via `@sveltejs/kit` | low | – | 4.0.1 (Step 4) |

The application does not use any of the affected SvelteKit features: remote functions, `query.batch`, form actions or `use:enhance`, prerendering. The test files do not parse untrusted HTML. Both remain good reasons to upgrade.

**cargo audit** (cargo-audit 0.23.0, `--no-fetch`, advisory database fetched 2026-11-18, 450 crates): **6 vulnerabilities, all in `zip` 2.4.2.** They are the six IDs that `audit.yml` ignores.

| ID | Title (short) | Severity | Patched |
| --- | --- | --- | --- |
| [RUSTSEC-2026-0367](https://rustsec.org/advisories/RUSTSEC-2026-0367) | SUID/SGID bits preserved during extraction | 7.3 high | >=9.0.0 |
| [RUSTSEC-2026-0368](https://rustsec.org/advisories/RUSTSEC-2026-0368) | `ZipArchive::extract` symlinks outside the destination | – | >=9.0.0 |
| [RUSTSEC-2026-0369](https://rustsec.org/advisories/RUSTSEC-2026-0369) | `extract()` applies Unix permissions through symlinks | 9.1 critical | >=9.0.0 |
| [RUSTSEC-2026-0429](https://rustsec.org/advisories/RUSTSEC-2026-0429) | `extract` aborts on a symlink with a large declared size | 7.5 high | >=9.0.0 |
| [RUSTSEC-2026-0545](https://rustsec.org/advisories/RUSTSEC-2026-0545) | AES entries returned without checking the authentication code | 6.5 medium | >=9.0.0 |
| [RUSTSEC-2026-0548](https://rustsec.org/advisories/RUSTSEC-2026-0548) | `ZipStreamReader` local-header name differs from the central directory | – | **>=9.0.5** |

The **8 warnings** do not fail cargo audit:

- `chrono` 0.4.45 unmaintained ([RUSTSEC-2026-0543](https://rustsec.org/advisories/RUSTSEC-2026-0543)). This is a direct dependency; see Step 5.
- `unic-char-property`, `unic-char-range`, `unic-common`, `unic-ucd-ident`, `unic-ucd-version` unmaintained (RUSTSEC-2025-0081, -0075, -0080, -0100, -0098). They come through `urlpattern` and `tauri-utils`. **Check** after Step 1 whether Tauri 2.12 still pulls them in.
- `proc-macro-error` 1.0.4 unmaintained (RUSTSEC-2024-0370) and `glib` 0.18.5 unsound (RUSTSEC-2024-0429). Both belong to the Linux GTK stack of Tauri and are not compiled for Windows.

### 2.6 Refreshing this inventory

Read-only commands. Prepend `$env:Path = "$env:USERPROFILE\.cargo\bin;C:\Program Files\nodejs;" + $env:Path` if the tools are not on the path.

```powershell
npm outdated --json
npm audit --omit=dev --json
npm audit --json
npm view @sveltejs/kit@latest peerDependencies engines --json   # repeat for vite, vitest, @sveltejs/vite-plugin-svelte, svelte-check
npm view typescript dist-tags --json
cd src-tauri
cargo audit                       # fetches the advisory database; --no-fetch uses the cached one
cargo tree --locked -i zip -e features
cargo tree --locked -i chrono
cd ..
```

For crate versions and minimum Rust versions, use `https://crates.io/api/v1/crates/<name>` (`max_stable_version`, and `rust_version` of the newest version) or `cargo info <name>`.

## 3. Step 1: Tauri 2.11 to 2.12

| Package | From | To |
| --- | --- | --- |
| `@tauri-apps/api` | 2.11.1 | 2.12.5 |
| `@tauri-apps/plugin-dialog` | 2.7.2 | 2.8.4 |
| `@tauri-apps/cli` and `@tauri-apps/cli-win32-x64-msvc` | 2.11.4 | 2.12.6 |
| `tauri` | 2.11.5 | 2.12.7 |
| `tauri-build` | 2.6.3 | 2.7.6 |
| `tauri-plugin-dialog` | 2.7.2 | 2.8.4 |

### What changes upstream (2.12.0 to 2.12.7)

Sources: [tauri-v2.12.0](https://github.com/tauri-apps/tauri/releases/tag/tauri-v2.12.0), [tauri-cli-v2.12.0](https://github.com/tauri-apps/tauri/releases/tag/tauri-cli-v2.12.0) and the later patch releases.

- **Security.** 2.12.0 binds channel IPC queue entries to the webview that created them ([GHSA-w28w-mhc8-qvjv](https://github.com/tauri-apps/tauri/security/advisories/GHSA-w28w-mhc8-qvjv)). This app has a single webview, so the risk is low, but this fix is the main reason for the step.
- **Minimum Rust 1.90; crates moved to edition 2024.** The unstable Tauri crates (for example `tauri-runtime` and `tauri-utils`) are now locked to minor versions. `windows` moves to 0.62 and `webview2-com` to 0.39, which drops Windows 7 (irrelevant here).
- **Breaking changes listed by 2.12.0:** only Android paths.
- **New, unused here:**
  - `bundle.windows.bundleVCRuntime`;
  - `build.windows.staticVCRuntime` (the `STATIC_VCRUNTIME` variable is deprecated; this repository does not use it);
  - `noRedirectionBitmap`;
  - `app.appDirectoriesOverride`;
  - `--no-binary-patching`.
- **NSIS** (template and bundler; versions are those of the Tauri CLI, which embeds the bundler):
  - CLI 2.12.2 (bundler 2.10.2) blocks downgrades in silent (`/S`), passive (`/P`) and update (`/UPDATE`) modes when `allowDowngrades` is `false`.
  - CLI 2.12.3 (bundler 2.10.3) stops leaving `uninstall.exe` and the folder behind after uninstalling a previous install, and updates the bundled NSIS to **3.13**.
  - CLI 2.12.4 (bundler 2.10.4) renames the `choowHowToInstall` language string to `chooseHowToInstall`, makes that downgrade check also compare with an MSI install, and quotes the executable path in file-association registry values.
  - CLI 2.12.5 (bundler 2.10.5) fixes MSI shortcut re-creation and an NSIS/MSI "error 0x2" bundling failure under the `SYSTEM` account, and shows a dedicated message when the running app cannot be closed.
- **Dialog plugin** ([dialog-v2.8.0](https://github.com/tauri-apps/plugins-workspace/releases/tag/dialog-v2.8.0) to 2.8.4):
  - minimum Rust 1.90;
  - the global API script (only with `withGlobalTauri`, which this app does not set) uses `window.__TAURI__`;
  - 2.8.2 fixes the browser `confirm` override;
  - 2.8.4 moves to `rfd` 0.17.
  - The JavaScript commands are unchanged: `plugin:dialog|open`, `|save` and `|message`, checked in the 2.8.4 package. The frontend uses only `confirm`, `open` and `save` from the plugin, and only `invoke` and `listen` from `@tauri-apps/api`.

### Procedure

1. Save the current permission sets for the review in step 5 of this procedure. `src-tauri/gen/schemas` is ignored by Git.

   ```powershell
   New-Item -ItemType Directory -Force C:\wvu-upgrade | Out-Null
   Copy-Item src-tauri\gen\schemas\acl-manifests.json C:\wvu-upgrade\acl-manifests.before.json
   ```

2. Update npm and Cargo together.

   ```powershell
   npm install @tauri-apps/api@^2.12.5 @tauri-apps/plugin-dialog@^2.8.4
   npm install -D @tauri-apps/cli@^2.12.6
   npm ls @tauri-apps/cli @tauri-apps/cli-win32-x64-msvc      # both must be 2.12.6
   cargo update --manifest-path src-tauri\Cargo.toml -p tauri -p tauri-build -p tauri-plugin-dialog
   ```

   Optional: raise the floors in `src-tauri/Cargo.toml` to `tauri = "2.12"`, `tauri-build = "2.7"` and `tauri-plugin-dialog = "2.8"`, so the manifest records the reviewed minimum.

3. **Re-sync the NSIS template.** Follow [Re-syncing the NSIS template](DEVELOPER_GUIDE.md#re-syncing-the-nsis-template), steps 1 to 8. What to expect this time (compared on 2026-11-19 from GitHub; step 2 of that procedure must still confirm that the installed binary embeds it):
   - Tag `tauri-cli-v2.12.6`, commit `9319e539f34237035cb4693bed9246660bc0a926`. The template has 1,022 lines, against 977 for 2.11.4, and its SHA-256 with LF line endings is `d00089d3b1a2c4a30d7b6e098079100cb8f25e980f84d4515aca4597b95e967d`. `diff` against `installer.upstream.nsi` reports 109 changed lines (77 added, 32 removed).
   - **The MSI search moved.** The loop that matches `DisplayName` and `Publisher` moved out of `PageReinstall` into a new `Function FindWixInstall`, which sets `$R6`.
     - Move the WVU region that maps Publisher `wvu` to `${MANUFACTURER}` into that function. It goes at the same place: between `ReadRegStr $R1 ... "Publisher"` and `StrCmp "$R0$R1" "${PRODUCTNAME}${MANUFACTURER}" 0 wix_loop`.
     - Step 5 of the Developer Guide's re-sync procedure names "the MSI search loop in `PageReinstall`"; change it to `FindWixInstall`.
     - `FindWixInstall` is also called by the downgrade check, which moved from `Section EarlyChecks` into `Function .onInit`. That check is compiled only when `allowDowngrades` is `false`; `tauri.conf.json` does not set it, and the Tauri default is `true` (confirmed in the `tauri-utils` config: "The default value of this flag is `true`").
   - **New cleanup after uninstalling.** After a successful NSIS uninstall, `PageLeaveReinstall` now runs `Delete "$4\uninstall.exe"` and `RMDir "$4"` when `$WixMode = 0` and `$4 != ""`.
     - The WVU region sets `$4` to the 1.0.x folder, so this cleanup now also removes a 1.0.x `uninstall.exe` and its empty folder.
     - Keep the region before the `ExecWait`, and keep `ClearErrors` and the second `UninstallString` read as its last lines. The consistency script requires them.
   - **Outside the regions** (take them unchanged):
     - `!include "Win\RestartManager.nsh"`;
     - `CheckIfAppIsRunning` now receives `$INSTDIR\${MAINBINARYNAME}.exe`;
     - the language-string fallback;
     - quoting in `APP_ASSOCIATE` (no file associations are configured).
   - **NSIS version.** The cached NSIS in `%LOCALAPPDATA%\tauri\NSIS` is 3.11. CLI 2.12.3 and later use NSIS 3.13. The 2.12.6 bundler checks the SHA-1 of the cached `NSIS.exe` on Windows and, when it differs, logs "NSIS.exe is outdated. Recreating it." and downloads `nsis-3.13.zip`. Allow network access for that first build.
   - Update the table in `src-tauri/nsis/README.md`: CLI version, tag, commit, source URL and SHA-256, and the sentence below the table that names the CLI 2.11.4 binary. The README "Why" section names no version. Also update the "Tauri CLI 2.11.4" wording in the 1.0.x-upgrade item of [the handoff](AGENT_HANDOFF.md#deferred-decisions). If Step 1 lands before 1.1.0 is released, also update the 1.1 entry in `CHANGELOG.md` that names Tauri CLI 2.11.4's template.

4. Run V1. Then run `npm run tauri build -- --bundles nsis` and `npm run tauri build -- --bundles msi` with the real ZIP. That is V5, and it compiles the template with makensis.

5. **Review capabilities and permissions.**
   - `src-tauri/capabilities/default.json` grants `core:default` plus `dialog:allow-open`, `dialog:allow-save` and `dialog:allow-message`. `src/lib/release-config.test.ts` ("grants only the dialog commands the frontend uses") pins the three dialog permissions.
   - `core:default` is a set that Tauri defines. Compare it before and after the upgrade. 2.12.0 added `core:app:allow-exit` and `core:window:allow-set-fullscreen-on-monitor`, and they must not have joined a default set silently.

     ```powershell
     node -e "for (const f of process.argv.slice(1)) { const m = require(f); console.log('== ' + f); for (const k of Object.keys(m)) if (k.startsWith('core') || k === 'dialog') console.log(k, (m[k].default_permission || {permissions: []}).permissions.join(',')); }" C:\wvu-upgrade\acl-manifests.before.json $PWD\src-tauri\gen\schemas\acl-manifests.json
     ```

     On 2026-11-19, `core:app` default was `allow-version, allow-name, allow-tauri-version, allow-identifier, allow-bundle-type, allow-register-listener, allow-remove-listener, allow-supports-multiple-windows`, and `dialog` default was `allow-message, allow-save, allow-open`.
   - If a default set grew, decide whether to list permissions explicitly instead of `core:default`. Extend `release-config.test.ts` accordingly.

6. Run V2, V3 (all three dialogs), V6 and V7.

7. Run `cargo audit` and record whether the `unic-*` warnings remain.

### Upgrade-path re-test (V6)

Every template change invalidates the earlier Sandbox results.

- Run the four 1.0.2 upgrades in [Release Checklist](RELEASE_CHECKLIST.md#upgrade-from-the-previous-release), including the run with `HKLM\SOFTWARE\wvu\WVU Bioinstrumentation Studio` deleted (the `UninstallString` fallback).
- Once 1.1.0 is released, also run 1.1.0 to candidate for NSIS to NSIS and MSI to MSI.
- Additionally:
  - Run one NSIS upgrade in passive mode (`setup.exe /P`).
  - After each NSIS upgrade, confirm that the old folder holds no stray `uninstall.exe`, that the app starts, and that recordings, presets and local labs are kept.

## 4. Step 2: zip 2.4 to 9.7

This clears all six blocking advisories. Target: the latest 9.x, and at least 9.0.5 for RUSTSEC-2026-0548. Upstream: [zip-rs/zip2 CHANGELOG](https://github.com/zip-rs/zip2/blob/master/CHANGELOG.md).

### API surface in use

All uses are in `src-tauri/src/arduino_runtime.rs`; line numbers are as of HEAD `66af30a` (the file is unchanged since `507673b`). They were compared with the 9.7.2 source; 9.7.3, published later the same day, changes only `src/read/extract.rs` (a 32-bit Android `st_mode` fix).

| Call | Where | In 9.7.2 |
| --- | --- | --- |
| `ZipArchive::new(File)`, `len()` | `archive_entries` (~1048), `extract_archive` (~1442), test `case_colliding_archive_entries_do_not_force_extraction_on_every_launch` (~1647) | Same signature. |
| `by_index_raw(i)` | `archive_entries` (~1053), the same test (~1656) | Same. |
| `by_index(i)` and `io::copy` from the entry | `extract_archive` (~1446-1458) | Same. Since 9.0.1, a read that exceeds the declared uncompressed size fails. CRC-32 is still checked. |
| `enclosed_name() -> Option<PathBuf>` | ~1055, ~1448 | Same signature, but **stricter on Windows** (below). |
| `size()`, `is_dir()` | ~1060-1061, ~1452 | Same. `is_dir` is still "raw name ends in `/` or `\`". |
| Tests: `ZipWriter::new`, `SimpleFileOptions::default()`, `add_directory`, `start_file`, `write_all`, `raw_copy_file`, `finish` | tests (~1470-1510, ~1651-1674) | Same names. `start_file` still takes `S: ToString`, as in 2.4.2, and `SimpleFileOptions` is still `Copy`. `raw_copy_file` now also preserves extra fields and the entry kind. |

The app does not use `ZipArchive::extract`, `ZipStreamReader`, AES, or `ZipFile::name()`, which now returns `ZipResult<Cow<str>>`. Those are where the advisories and most breaking changes are. Expected result: only `Cargo.toml` (and `Cargo.lock`) change. **Check** with `cargo check`; the 9.0.0-pre1 release also removed deprecated items.

### Feature flags

```toml
zip = { version = "9.7", default-features = false, features = ["deflate-flate2-zlib-rs"] }
```

- **Keep `default-features = false`.** The 9.x defaults add AES, bzip2, deflate64, lzma, ppmd, time, xz and zstd.
- **Why not `deflate`.** In 9.x, `deflate` means `deflate-zopfli` plus `deflate-flate2-zlib-rs`. `zopfli` is only a slow, high-ratio compressor for writing (levels 10 to 264), and the app never writes ZIPs outside tests.
- **`deflate-flate2` alone selects no back end** since zip 3.0. `deflate-zopfli` alone cannot decompress.
- **The decompressor changes** from `miniz_oxide` (flate2 `rust_backend`, today) to `zlib-rs`. Both are pure Rust.
  - `flate2` and `miniz_oxide` stay in `Cargo.lock`, because `png` also uses them: `png` 0.17 through `tauri-codegen` (and `ico`), and `png` 0.18 through `muda`, which depends on it only on macOS.
  - flate2 prefers `zlib-rs` over `miniz_oxide` when both are enabled. On Windows that affects only `zip`: `cargo tree -i flate2` shows `zip` as the only runtime user, because `png` 0.17 is reached only through `tauri-codegen` inside the `tauri-macros` proc-macro, whose features resolver 2 keeps separate, and `muda` does not use `png` on Windows.
- **Crate changes** (zip 9.7.2 makes `constant_time_eq`, `typed-path` and `zeroize` mandatory):
  - new: `typed-path` (MIT OR Apache-2.0), `zlib-rs` (Zlib license), `constant_time_eq` (CC0-1.0 OR MIT-0 OR Apache-2.0), `zeroize` and `zeroize_derive` (Apache-2.0 OR MIT);
  - removed: `zopfli` (Apache-2.0), and any of its dependencies that nothing else uses. `Cargo.lock` also drops `arbitrary` and `derive_arbitrary`, which only zip 2.4.2 lists (as a `cfg(fuzzing)` dependency that is never compiled here);
  - unchanged: zip 9.x also requires `getrandom` 0.4 on non-wasm targets, which is already locked (0.4.3, through `uuid`).
- **Licenses.** Record these in any crate-license inventory that exists by then (the handoff lists Rust crate license texts as a deferred release item).
- After the change, check that no extra codecs are compiled: `cargo tree --locked -i zip -e features`.

### Behavior changes to test with the real runtime ZIP

The app turns `enclosed_name() == None` into `RuntimeError::InvalidBundle`, which refuses the whole runtime. On Windows, zip 9.x `enclosed_name` rejects a path component that matches the rules below. 9.1.0 introduced them (`src/path.rs` `is_windows_invalid_component`), and the list is as of 9.7.2 (`path.rs` is identical in 9.7.3; `CONFIG$` and the superscripts `⁴`-`⁹` were added after 9.1.0). 9.0.4 added only the rejection of a separator inside one verbatim path component. A component is rejected when it:

- is a DOS device name: `CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9`, `LPT1`-`LPT9` (also with the superscript digits `¹`-`⁹`), `CONIN$`, `CONOUT$`, `CLOCK$` or `CONFIG$`. This applies even with an extension, so `aux.h` is rejected.
- contains `< > : " | ? *` or an ASCII control character (including DEL).
- ends in a dot or a space.
- is longer than 255 UTF-16 units.

**Pre-check.** It is read-only; run it on any runtime archive before relying on 9.x:

```powershell
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead((Resolve-Path src-tauri\resources\arduino-runtime.zip))
try {
  $reserved = '^(CON|PRN|AUX|NUL|CONIN\$|CONOUT\$|CLOCK\$|CONFIG\$|COM[1-9¹²³⁴-⁹]|LPT[1-9¹²³⁴-⁹])$'
  $bad = foreach ($e in $zip.Entries) {
    foreach ($c in ($e.FullName.TrimEnd('/') -split '/')) {
      $stem = ($c -split '[.:]')[0].TrimEnd(' ', '.')
      if ($c -eq '' -or $c -match '[<>:"|?*\x00-\x1f\x7f\\]' -or $c.EndsWith('.') -or $c.EndsWith(' ') -or
          $stem.ToUpperInvariant() -match $reserved -or $c.Length -gt 255) { $e.FullName }
    }
  }
  "entries: $($zip.Entries.Count); rejected by zip 9 on Windows: $(@($bad).Count)"; $bad
} finally { $zip.Dispose() }
```

Result on 2026-11-19 for runtime revision 4 (`5baa8a5f…`):

- 6,339 entries, **0 rejected**;
- 0 names with `\` (the script counts a `\` as rejected, which is stricter than zip, which splits on it);
- 2 directory entries: `data/packages/arduino/tools/arm-none-eabi-gcc/7-2017q4/arm-none-eabi/include/bits/` and `.../include/rpc/`;
- 0 symlink or Unix-mode entries;
- 0 names that differ only in case;
- longest name 171 characters.

**Check** the following:

- **Sizes.** After a **Repair Arduino tools** run, `application.log` has no `ARDUINO_RUNTIME_EXTRACT_INVALID` and no `ARDUINO_RUNTIME_EXTRACT_SIZE_MISMATCH`. That covers the 9.0.1 declared-size check and `inspect_runtime`'s size comparison.
- **Directories.** The 2 explicit directory entries (`.../include/bits` and `.../include/rpc`) are created, and the layout check passes: `data/packages/arduino/hardware/renesas_uno` and `data/packages/arduino/tools` exist.
- **Content.** V4 `runtime_smoke` compiles offline. That exercises `prepare_from_archive` and the new deflate back end end to end.
- **Optional test.** Add a `#[cfg(windows)]` test that an archive entry such as `data/aux.h` makes `prepare_from_archive` fail with `InvalidBundle`. It documents the new rule. `ZipWriter` 9.7.2 does not apply the Windows name rules when writing (only `enclosed_name` and extraction do), so `start_file("data/aux.h", ...)` can build the test archive.
- **Optional timing.** Time **Repair Arduino tools** before and after on the same machine.

### Audit workflow and documentation

- In `.github/workflows/audit.yml`:
  - remove the six `--ignore RUSTSEC-2026-…` lines and the comment block above them. The blocking step becomes `cargo audit`; unmaintained warnings such as `chrono` still do not fail it.
  - Keep the informational full-report step and the pinned cargo-audit `0.23.0`.
- Update the `cargo audit` sentence under [Continuous integration](DEVELOPER_GUIDE.md#continuous-integration) and the **Dependencies** item under [Deferred decisions](AGENT_HANDOFF.md#deferred-decisions).

### Minimum Rust version

`zip` 9.7.2 and 9.7.3 declare `rust-version = "1.88"` and edition 2024. The pinned toolchain is 1.100.0, so it needs no change.

## 5. Step 3: frontend tooling on SvelteKit 2

This step stays on SvelteKit 2.70.4, whose peer ranges already accept everything below. Step 4 then has only SvelteKit-specific changes.

### Targets and peer constraints

| Package | From | To | Constraints that matter (npm metadata, 2026-11-19) |
| --- | --- | --- | --- |
| `happy-dom` | 20.14.6 | 21.0.4 | Node >=20. Vitest accepts any `happy-dom`. Independent of everything else. |
| `svelte` | 5.56.8 | 5.58.0 | vite-plugin-svelte 7 needs >=5.46.4. SvelteKit 3 needs >=5.57.1 (`sv migrate` raises a lower range to `^5.57.2`; after this step, `^5.58.0` stays). |
| `svelte-check` | 4.7.4 | 4.7.8 | Peer `typescript ^5.0.0 \|\| ^6.0.0`. |
| `typescript` | 5.6.3 | **6.0.3** | Kit 2.70.4 accepts `^5.3.3 \|\| ^6.0.0`. Kit 3.4.0 requires `^6.0.0`. **TypeScript 7.0.2 (npm `latest`, the native Go port) is outside both the Kit 3 and svelte-check peer ranges: do not install it.** |
| `vite` | 6.4.6 | 8.3.x | Kit 2.70.4 accepts `^8.0.0`. Kit 3 needs `^8.0.12`. Engine `^20.19.0 \|\| >=22.12.0`. |
| `@sveltejs/vite-plugin-svelte` | 5.1.1 | 7.3.x | Peer `vite ^8`, `svelte ^5.46.4`. Kit 2.70.4 accepts `^7.0.0`. vite-plugin-svelte 5 accepts only Vite 6, so **Vite and vite-plugin-svelte move together**. |
| `vitest` | 4.1.12 | 5.1.x | Peer `vite ^6.4.0 \|\| ^7 \|\| ^8`. Engine `^22.12.0 \|\| ^24.0.0 \|\| >=26.0.0`. |

Suggested commits on the step branch, each passing V1:

1. **happy-dom** (closes both high advisories; it could even go first, before Step 1): `npm install -D happy-dom@^21.0.4`.
2. **svelte and svelte-check:** `npm install -D svelte@^5.58.0 svelte-check@^4.7.8`.
3. **TypeScript:** `npm install -D typescript@~6.0.3`.
4. **Vite, the plugin and Vitest:** `npm install -D vite@^8.3.8 @sveltejs/vite-plugin-svelte@^7.3.1 vitest@^5.1.1`. Also bump `engines.node` to `>=22.12`, Vitest 5's minimum; Step 4 raises it again.

### What the upstream guides change, and what applies here

- **Vite 8** ([migration from v7](https://vite.dev/guide/migration); for 6 to 7 see the [v7 guide](https://v7.vite.dev/guide/migration)):
  - Rolldown and Oxc replace esbuild and Rollup.
  - The Oxc minifier and Lightning CSS minify the build.
  - The default build target is Chrome/Edge 111. WebView2 is evergreen, so this is fine.
  - `build.rollupOptions` is renamed `rolldownOptions`.
  - Vite 7 dropped Node 18 and the Sass legacy API.
  - None of these options appear in `vite.config.js`. **Check:**
    - the CSS (Lightning CSS) and the UI in V3 screenshots;
    - the `build/` size;
    - that the header logo still loads in development (`server.fs.allow: ["assets"]`).
- **vite-plugin-svelte 6 and 7** ([changelog](https://github.com/sveltejs/vite-plugin-svelte/blob/main/packages/vite-plugin-svelte/CHANGELOG.md)):
  - 6.0 drops Node 18, requires Vite `^6.3.0 || ^7.0.0`, and drops CommonJS `svelte.config` files (ours is ESM);
  - 7.0 requires Vite 8 and Svelte 5.46.4, removes `vitePlugin.hot` (not used), stops overriding `cssHash`, and folds the inspector package in.
  - `vitePreprocess()` now uses `transformWithOxc`, and still preprocesses `<style>` only by default. The components use `<script lang="ts">` and plain `<style>`, with no `lang` on styles.
- **TypeScript 6** ([announcement](https://devblogs.microsoft.com/typescript/announcing-typescript-6-0/)). New defaults:
  - `strict: true` (already set);
  - `module: esnext`; `target` the latest ES;
  - `noUncheckedSideEffectImports: true`;
  - `types: []`;
  - `rootDir` = the tsconfig folder.

  `esModuleInterop` can no longer be `false` (ours is `true`). `moduleResolution: "bundler"` is unaffected.
  - **Check** `npm run check` for:
    - unresolved `node:fs`/`node:path` in the six test files that import them: `src/lib/release-config.test.ts`, `src/lib/messages.test.ts`, `src/lib/multichannel.test.ts`, `src/lib/components/LabManager.svelte.test.ts`, `src/lib/components/SessionCard.svelte.test.ts` and `src/routes/page.test.ts`;
    - the two side-effect imports: `import '$lib/styles/studio-form.css'` in `src/routes/+page.svelte` and `import 'uplot/dist/uPlot.min.css'` in `src/lib/components/LivePlot.svelte`.
  - **Uncertain:** whether `@types/node` (today only transitive) still loads with `types: []`. If not, add `@types/node` as a devDependency and `"types": ["node"]` to `tsconfig.json`. After Step 4 the list must also contain `"$app/types"`.
- **Vitest 5** ([migration guide](https://vitest.dev/guide/migration)):
  - `clearMocks` is `true` by default;
  - `vi.mock`/`vi.hoisted` must be top level;
  - unawaited `resolves`/`rejects` fail the test;
  - `toThrow('')` matches any message;
  - global assignments in happy-dom now update the DOM window;
  - reports and attachments go under `.vitest/`;
  - `vite` is a required peer;
  - Node >=22.12.
- **happy-dom 21** ([v21.0.0](https://github.com/capricorn86/happy-dom/releases/tag/v21.0.0)): CSS file and iframe loading are disabled by default. The tests use neither.

### Test suites likely affected (counts at HEAD `66af30a`)

- **Scope.** 38 test files; 11 declare `// @vitest-environment happy-dom`.
- **Mocks.** `vi.mock` appears 6 times, all at top level, in `src/routes/page.test.ts` and `src/lib/components/LabManager.svelte.test.ts`. `vi.hoisted` appears 3 times (2 in `page.test.ts`, 1 in `LabManager.svelte.test.ts`), all at top level. The fake backends count calls in plain objects, not in mock history.
- **Spies and stubs.**
  - `vi.fn`: 7 (5 files). `vi.spyOn`: 8 (4 files). `vi.stubGlobal`: 8 (4 files). `vi.restoreAllMocks`: 1.
  - `toHaveBeenCalledTimes` appears once, in `RecordingBar.svelte.test.ts`, on a spy created inside the test, so `clearMocks` does not affect it.
- **Async assertions.** `.resolves` appears 3 times in `clipboard.test.ts`, all awaited.
- **Not used at all:** fake timers, `setSystemTime`, `toThrow('')`, `expect.poll`, `beforeAll`, `test.sequential`.
- **Check** `src/lib/page-navigation.test.ts`. It stubs `innerHeight` under happy-dom, which Vitest 5 now propagates into the DOM window. `BoardPanel.svelte.test.ts` and `RecordingResultCard.svelte.test.ts` likewise stub `navigator` under happy-dom.

Expected impact: low. Fix a failing test only for a documented semantic change. Never weaken an assertion to make it pass.

### Config files

- `vite.config.js`: no required change.
- `vitest.config.ts`: no required change. Optionally add `test: { clearMocks: true }` to make the new default visible. Keep `resolve: { conditions: ['browser'] }`.
- `svelte.config.js`: unchanged; SvelteKit 2 still reads it.
- `tsconfig.json`: only if `npm run check` asks for it (see TypeScript 6 above).
- `.gitignore`: add `.vitest/`.
- `package.json` `engines.node`: `>=22.12`.

## 6. Step 4: SvelteKit 3 and adapter-static 4

Upstream: [Migrating to SvelteKit v3](https://svelte.dev/docs/kit/migrating-to-sveltekit-3), [`sv migrate`](https://svelte.dev/docs/cli/sv-migrate), [adapter-static changelog](https://github.com/sveltejs/kit/blob/main/packages/adapter-static/CHANGELOG.md) (4.0 only requires SvelteKit 3).

### Requirements

Kit 3.4.0 needs:

| Requirement | Minimum |
| --- | --- |
| Node | 22.17 |
| TypeScript | 6 |
| Svelte | 5.57.1 |
| Vite | 8.0.12 |
| `@sveltejs/vite-plugin-svelte` | 7 |

Step 3 provides all of these. Target `@sveltejs/kit` 3.4.x; 3.1.1 or later fixes GHSA-3r8g and GHSA-6cf5. Target `@sveltejs/adapter-static` 4.0.x.

### Scope in this repository (verified at HEAD `66af30a`)

The app uses very little of SvelteKit, so most of the migration guide does not apply.

- **Not used:**
  - `$app/*` imports (`$app/stores`, `$app/navigation`, `$app/paths`, `$app/environment`);
  - `$env/*` and `$service-worker`;
  - `use:enhance`;
  - server or remote modules, `src/params`, hooks;
  - `<a href>` links in components.
- **Used:**
  - one route (`src/routes/+page.svelte`) and `src/routes/+layout.ts` (`export const ssr = false;`);
  - `adapter-static` with `fallback: 'index.html'`, as the [Tauri SvelteKit guide](https://v2.tauri.app/start/frontend/sveltekit/) recommends. That guide is still written for SvelteKit 2.20 / `svelte.config.js`. **Uncertain:** how Tauri documents SvelteKit 3.
  - `$lib` in **103 import specifiers in 28 files** (19 source files, 9 test files):
    - 88 extensionless TypeScript module paths, such as `$lib/calibration` (every one names a `src/lib/<path>.ts` file);
    - 14 `.svelte` components;
    - 1 side-effect CSS import, `$lib/styles/studio-form.css` in `+page.svelte`.

    `vi.mock` targets only `@tauri-apps/*` and `uplot`, never `$lib`.

### Procedure

1. Start from a clean tree (`git status --short` empty) on the step branch. Run the pinned migration tool. `sv` 1.6.0 became the latest on 2026-11-19; its `sveltekit-3` tasks differ from 1.5.0 only in the `environment` task, which does not apply here:

   ```powershell
   npx sv@1.6.0 migrate sveltekit-3 --no-install
   npm install
   npm ls @sveltejs/kit @sveltejs/adapter-static     # 3.4.x and 4.0.x
   git status --short
   git diff --stat
   ```

   `sv` 1.6.0's `sveltekit-3` migration (the same list as 1.5.0) offers these tasks, in this order: `package-json` and `tsconfig` (prerequisites), then `svelte-config`, `environment`, `paths`, `external-redirects`, `shallow-routing`, `params`, `imports`, `lib-alias`, `app-state` and `collect-migration-instructions`. Without `--tasks`, it asks which tasks to run. It also asks for a final confirmation, and on a dirty Git tree it asks whether to continue (default: no).
   - The `package-json` task sets `@sveltejs/kit` to `^3.0.0` and `@sveltejs/adapter-static` to `^4.0.0`, and raises `vite`, `@sveltejs/vite-plugin-svelte`, `svelte`, `svelte-check` and `typescript` only where they are below Kit 3's minimums (after Step 3 none is). Without `--no-install`, it then asks for a package manager and installs the dependencies itself; `--no-install` leaves that to `npm install`.
   - When it finishes it deletes `.svelte-kit`, and runs `svelte-kit sync` only if it installed the dependencies itself; with `--no-install`, `npm run check` regenerates the folder.
   - The `collect-migration-instructions` task writes `MIGRATION_TASKS.md` at the repository root. Act on it, then delete it; never commit it.
   - `lib-alias` rewrites quoted `$lib` strings in every `.svelte`, `.ts`, `.js`, `.svx` and `.md` file, Markdown included. It would rewrite the `$lib` examples in this plan and in other documentation.
   - Revert anything it changed outside `src/`, the config files and `package.json`.

2. Review each expected change:
   - **`svelte.config.js` is deleted.** Its `preprocess` and `kit.adapter` become options of the Vite plugin in `vite.config.js`:

     ```js
     import adapter from '@sveltejs/adapter-static';
     import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';
     // ...
     plugins: [sveltekit({ preprocess: vitePreprocess(), adapter: adapter({ fallback: 'index.html' }) })],
     ```

     **Uncertain:** whether the tool handles this file's `defineConfig(async () => ({ ... }))` form. If it does not, edit by hand. Keep the Tauri comments and options: port 1420, `strictPort`, `TAURI_DEV_HOST`/HMR, `watch.ignored`, `fs.allow`.
   - **`vitest.config.ts`.** It calls `sveltekit()` separately. **Uncertain:** whether the migration updates it, and whether Kit 3 needs the adapter in a test-only config. To keep the two configs from drifting, export the options once and pass them in both files.
     - Put them in a small module such as `sveltekit-options.js`, with `export const kitOptions = { preprocess: vitePreprocess(), adapter: adapter({ fallback: 'index.html' }) };`.
     - Do not name it `svelte.config.*`, which the migration and Kit treat as the removed file.
     - Do not merge the Vitest settings into `vite.config.js`: `resolve.conditions: ['browser']` must not reach the production build.
   - **`package.json`** gains `"imports": { "#lib": "./src/lib/index.js", "#lib/*": "./src/lib/*" }`. There is no `src/lib/index.*`; the bare `#lib` entry is harmless and unused.
   - **`$lib` becomes `#lib`.** For an application, the tool appends the real extension when `src/lib/<path>.ts` exists, for example `#lib/calibration.ts`. `.svelte` and `.css` keep theirs. Afterwards, `git grep -n -F '$lib' -- src` must print nothing.
   - **`tsconfig.json`** extends `$app/tsconfig` instead of `./.svelte-kit/tsconfig.json`. `$app/tsconfig` sets no `include` or `exclude`, so the `tsconfig` task writes `"include": ["src", "vite.config.js"]` here (`src` plus the root config files it knows that exist; `vitest.config.ts` is not type-checked today either) and no `exclude` (the guide's example excludes only `src/service-worker`, which this app does not have). It also deletes the compiler options that `$app/tsconfig` already sets to the same value (`allowJs`, `checkJs`, `esModuleInterop`, `forceConsistentCasingInFileNames`, `resolveJsonModule`, `skipLibCheck`, `moduleResolution`), leaving `strict` and `sourceMap`. If you set `types`, it must contain `"$app/types"` (the task adds it to an existing list). `$app/tsconfig` provides `isolatedModules` and `verbatimModuleSyntax`; do not override them or add `paths`.
   - **`src/routes/+layout.ts`** stays as it is: `ssr = false`, no prerendering.
   - **The fallback stays `index.html`.** The Kit docs advise against `index.html` only when prerendering. `release-config.test.ts` requires `build/index.html`.
3. **Check** these Kit 3 behavior changes in V3 and V5:
   - **Version polling.** `version.pollInterval` now defaults to one hour, and `updated` is also checked when the window gains focus or becomes visible. In the packaged app that means requests for `_app/version.json` from the app's own origin (the CSP `connect-src 'self'` allows them). They are pointless in a desktop app. Consider `version: { pollInterval: 0 }`: the Kit 3.4.0 `Config` type documents that `0` disables polling, but the header- and event-based checks still run. Watch DevTools for failed requests.
   - **Sourcemaps.** "Sourcemaps are generated by default." In Kit 3.4.0 that default (`environments.ssr.build.sourcemap ?? build.sourcemap ?? true`) applies to the server build. The client build that adapter-static copies into `build/` keeps Vite's `build.sourcemap: false`, so expect no map files there. Confirm with `Get-ChildItem build -Recurse -Filter *.map`: map files in `build/` would be embedded in the executable.
   - **`app.html`.** `data-sveltekit-preload-data="hover"` is still valid; only `"off"` was removed.
4. **Node version.**
   - Set `package.json` `engines.node` to `^22.17.0 || ^24.0.0 || >=26.0.0`, the intersection of Kit 3 (>=22.17) and Vitest 5 (no 23 or 25), or simply `>=22.17`.
   - `.github/workflows/ci.yml` and `audit.yml` use `node-version: 22`. Without `check-latest`, `actions/setup-node` uses a matching 22.x from the runner's tool cache and downloads one only if none is cached. Hosted runners carry a recent 22.x; leave it, or pin a minimum such as `22.17`.
   - Update:
     - `scripts/check_environment.ps1` (it checks only major >= 22);
     - the message in `scripts/start_dev_sandbox.ps1`;
     - [Developer Guide prerequisites](DEVELOPER_GUIDE.md#prerequisites);
     - the handoff.
   - Node 22 has been in maintenance since 2025-10-21 and reaches end of life on 2027-04-30 (the [Node.js release schedule](https://github.com/nodejs/Release/blob/main/schedule.json); see also <https://nodejs.org/en/about/previous-releases>). Plan a separate step to move CI to Node 24.
5. Run V1. `npm run check` still runs `svelte-kit sync`; Kit 3.4.0 still ships the `svelte-kit` binary. Build before test, then run V2 (npm audit over all dependencies should now report 0), V3, V5 and V7.

## 7. Optional step 5: chrono to jiff (or time)

### Why

[RUSTSEC-2026-0543](https://rustsec.org/advisories/RUSTSEC-2026-0543) ([chrono#1768](https://github.com/chronotope/chrono/issues/1768)) marks `chrono` as unmaintained and recommends [`jiff`](https://crates.io/crates/jiff) (1.0.1, Unlicense OR MIT, minimum Rust 1.85) or [`time`](https://crates.io/crates/time) (0.3.55, already in `Cargo.lock` through Tauri, so it adds no new crate). It is an informational advisory, not a vulnerability. `cargo audit` reports it as a warning and does not fail.

### Scope

Counted at HEAD `66af30a` with `git grep -c -E 'chrono|Utc\b|Local\b|DateTime|ChronoDuration|to_rfc3339|SecondsFormat|\.format\("%' -- src-tauri/src`; a few hits are comments. The table lists the 7 files with real uses. The firmware commit `66af30a` already moved one count (`recording.rs`, 18 to 19), so recount before starting.

| File | Lines | Uses |
| --- | --- | --- |
| `src-tauri/src/session.rs` | 21 | `DateTime<Utc>`/`Local`, `ChronoDuration::from_std`, `with_timezone(&Local)`, `to_rfc3339_opts(Millis, true)`, `Local::now().format("%Y%m%d_%H%M%S")` |
| `src-tauri/src/recording.rs` | 19 | Serialized metadata fields `utc_start`, `local_start` (`DateTime<Local>`), `utc_stop`, `local_stop`, `session_opened_utc`, `acquisition_started_utc` |
| `src-tauri/src/firmware_workflow.rs` | 11 | Serialized job fields `started_utc`, `completed_utc`, `manual_reset_deadline_utc`; `started_utc.format("%Y%m%d_%H%M%S")` in a file name; `chrono::Duration::from_std` |
| `src-tauri/src/profiles.rs` | 10 | Serialized `captured_utc`; four `format("%Y%m%dT%H%M%S%.3fZ")` backup-name stamps; two `Utc::now().to_rfc3339()` audit-log lines |
| `src-tauri/src/calibration.rs` | 5 | Serialized preset `created_at: DateTime<Utc>` |
| `src-tauri/src/app_log.rs` | 2 | `Utc::now().to_rfc3339_opts(Secs, true)` |
| `src-tauri/src/project_paths.rs` | 1 | `format("%Y%m%dT%H%M%S%.3fZ")` |

### Hard requirement: byte-compatible output

Recordings, metadata, calibration presets, lab catalogs, firmware job records and logs are read by existing installations, course material and student analysis scripts. Every timestamp written after the change must have exactly the same text format. Every file written by 1.0.x and 1.1 must still parse. The formats today, confirmed in the chrono 0.4.45 source:

| Source | Example | Rule |
| --- | --- | --- |
| serde of `DateTime<Utc>` | `2026-11-19T14:03:07.123456700Z` | RFC 3339, `Z`. Fraction "AutoSi": 0, 3, 6 or 9 digits, the shortest group that holds every non-zero digit (Windows clocks give 100 ns, so usually 9). |
| serde of `DateTime<Local>` | `2026-11-19T09:03:07.123456700-05:00` | Same, with a numeric offset, or `Z` when the local offset is zero (serde passes `use_z = true`). **Never an IANA suffix** such as `[America/New_York]`. |
| `to_rfc3339()` | `2026-11-19T14:03:07.123456700+00:00` | `+00:00`, not `Z`, with the AutoSi fraction. |
| `to_rfc3339_opts(Secs, true)` | `2026-11-19T14:03:07Z` | No fraction. |
| `to_rfc3339_opts(Millis, true)` | `2026-11-19T14:03:07.123Z` | Always 3 digits. |
| `format("%Y%m%dT%H%M%S%.3fZ")` | `20261119T140307.123Z` | File-name stamp. |
| `format("%Y%m%d_%H%M%S")` | `20261119_140307` | File-name stamp, UTC or local. |

Pitfalls with `jiff`, from the [jiff 1.0.1 `DateTimePrinter` docs](https://docs.rs/jiff/latest/jiff/fmt/temporal/struct.DateTimePrinter.html):

- `Zoned` prints a bracketed IANA annotation. Use `Timestamp` with an explicit offset (`timestamp_with_offset_to_string`), not `Zoned`, for `local_*` fields.
- That printer writes `+00:00`, not `Z`, for a UTC offset.
- The default ("automatic") fraction precision is not AutoSi's 3/6/9 grouping. The jiff 1.0.1 source (`write_fraction`) strips every trailing zero, so 123,456,700 ns prints `.1234567` where chrono prints `.123456700`. Expect a custom formatter.
- `Zoned` parsing rejects an offset-only string from old files with "failed to find time zone annotation in square brackets" (jiff 1.0.1 `fmt::temporal` docs and source). Parse into `Timestamp` and keep the offset separately.

`time` has the same class of problem; its RFC 3339 fraction rule must also be checked. The `zip` crate has its own `jiff-02`, `chrono` and `time` features; keep them off.

### Recommendation

**Defer.** Revisit only if one of these happens:

- RustSec publishes a vulnerability (not just "unmaintained") for `chrono`;
- a dependency forces chrono out;
- serde or Rust changes break chrono.

If you do it, work in two commits on one branch:

1. **Golden tests first, still on chrono.** Add tests that serialize fixed instants with zero, millisecond, microsecond and 100 ns fractions, in UTC and in a fixed negative offset, through every row of the table above. Add tests that deserialize sample metadata, preset, catalog and firmware-job JSON written by 1.0.2 and 1.1.
2. **Swap the crate** without touching those tests. Then make a Simulator recording before and after (V3), and confirm that the timestamp fields of the `.metadata.json` match in format. Run one hardware recording (V7).

Estimated effort is 2 to 3 days. The risk is in data compatibility, not in compiling.

## 8. Order, rollback, acceptance criteria and effort

### Order

| Order | Step | Depends on | Why this position |
| --- | --- | --- | --- |
| 1 | Tauri 2.12 | – | Shipped code and installer. Its NSIS re-sync and Sandbox tests are the most manual work; do them while the 1.0.x regions are fresh. |
| 2 | zip 9.7 | – | Removes the only blocking audit ignores. Independent of Step 1; the two can be swapped. |
| 3 | Tooling | – | Development only. `happy-dom` can go first of all. Vite and vite-plugin-svelte must move in one commit. |
| 4 | SvelteKit 3 | Step 3 | Requires TypeScript 6, Vite 8, vite-plugin-svelte 7, Svelte 5.57.1 and Node 22.17. |
| 5 | chrono (optional) | – | Deferred; touches persisted data formats. |

### Rollback

- **Before merging:** do not merge; delete the branch.
- **After merging:** `git revert <step commit(s)>`, then `npm ci` and `cargo build --manifest-path src-tauri\Cargo.toml --locked`. The reverted lockfiles bring back the old packages.
- Revert a whole step, never half of it:
  - Step 1's lockfiles, NSIS files and README table belong together.
  - Step 4's `svelte.config.js` deletion, config moves and import rewrite belong together.
- **Step 1.** The CLI 2.11.4 bundler checks only that the required NSIS files exist and the hash of `nsis_tauri_utils.dll`, not the NSIS version. After a revert it therefore keeps building with the cached NSIS 3.13. To return to the 3.11 that 2.11.4 was tested with, delete the bundler's tool cache `%LOCALAPPDATA%\tauri\NSIS`, and the next build downloads 3.11 again. **Uncertain:** whether the 2.11.4 template compiles with 3.13.
- **No build from a reverted step may be distributed.** If a step's build was already released, roll forward with a new version instead. The installers block nothing by default (`allowDowngrades` is not set), but a newer version is still the clean path.
- **Steps 1 to 4 change no data formats,** so an app built after a revert reads everything the upgraded build wrote. Step 5 must keep that property; its golden tests enforce it.

### Acceptance criteria

| Step | Done when |
| --- | --- |
| 1 Tauri | Lockfiles: `@tauri-apps/api` 2.12.x, `plugin-dialog` 2.8.x, `cli` and `cli-win32-x64-msvc` the same 2.12.x; `tauri` 2.12.x, `tauri-build` 2.7.x, `tauri-plugin-dialog` 2.8.x. The consistency script passes, including the NSIS template and README table. V1 passes. NSIS and MSI build with the real ZIP. The default-permission diff is reviewed and `release-config.test.ts` passes. V3 dialogs work. All V6 upgrades pass. V7 passes. `cargo audit` shows no new vulnerability. |
| 2 zip | `zip` 9.7.x (at least 9.0.5) with only `deflate-flate2-zlib-rs`, checked with `cargo tree -e features`. Rust tests pass unchanged. The pre-check reports 0 rejected entries for the shipped runtime ZIP. `runtime_smoke` exits 0 offline. Repair followed by an offline Restore succeeds on hardware with no extract warnings in `application.log`. `audit.yml` has no `--ignore`, and `cargo audit` exits 0. Developer Guide and handoff are updated. |
| 3 Tooling | Versions as in the table (TypeScript 6.0.x, not 7). V1 passes with no weakened assertions. `npm audit` shows no `happy-dom` advisory. V3 screenshots match the previous ones. V5: the production executable loads without Vite. `engines` is >=22.12 and `.vitest/` is ignored. |
| 4 SvelteKit 3 | Kit 3.4.x and adapter-static 4.0.x. `svelte.config.js` is gone. `vite.config.js` and `vitest.config.ts` share the same Kit options. `git grep -F '$lib' -- src` is empty. `build/index.html` exists and `release-config.test.ts` passes. `npm audit` reports 0 vulnerabilities, development dependencies included. V3, V5 and V7 pass. Node 22.17 is required in `engines`, `check_environment.ps1` and the docs. `MIGRATION_TASKS.md` is resolved and not committed. |
| 5 chrono | The golden tests from the first commit pass unchanged after the swap. 1.0.2 and 1.1 files load. Simulator and hardware metadata timestamps keep their format. `cargo audit` has no chrono warning. |

### Effort

Estimates are in focused hours for one person, excluding CI wait time. They are rough.

| Step | Estimate | Where the time goes |
| --- | --- | --- |
| 1 Tauri | 4-8 h | Update 1 h, NSIS re-sync 1-2 h, two release builds 1 h, Sandbox upgrade tests 2-3 h, hardware smoke 1 h |
| 2 zip | 3-5 h | Manifest and gate 1 h, `runtime_smoke` 0.5 h, Repair and offline Restore on hardware 1 h, audit workflow and docs 0.5-1 h |
| 3 Tooling | 4-8 h (up to 1.5 days if TypeScript 6 or Vitest 5 report many errors) | Four commits with a gate each, visual comparison, production build |
| 4 SvelteKit 3 | 1-2 days | Migration tool and review of 28 rewritten files, config sharing, sourcemap/polling decisions, Node bump across scripts, CI and docs, V3/V5/V7 |
| 5 chrono (optional) | 2-3 days | Golden tests first, then the swap and a data-compatibility review |

### Not covered by this plan

- **In-range Cargo updates.** `serde_json` 1.0.154 and `thiserror` 2.0.21 can follow Dependabot's grouped minor/patch pull requests. `serialport` 4.10.1 also qualifies, but it is the serial path: run V7 before merging it.
- **`sha2` 0.10 to 0.11.** A pre-1.0 major with likely API changes. Its own small step; the firmware and runtime hash tests cover it.
- **`fs2` 0.4.3.** No release since 2018, but no advisory.
- **Node 24 for CI and development.** A separate step after Step 4.
- **The `rust-toolchain.toml` pin** and the cargo-audit `0.23.0` pin in `audit.yml`. Change them deliberately, on their own.
- **Svelte 5 legacy syntax.** The 111 `$:` reactive statements in `.svelte` files at HEAD `66af30a` (113 occurrences of `$:`, two of them in comments) still work in Svelte 5. A future Svelte major may change that; out of scope.

## 9. References

- Tauri:
  - [tauri-v2.12.0](https://github.com/tauri-apps/tauri/releases/tag/tauri-v2.12.0) and [tauri-cli-v2.12.0](https://github.com/tauri-apps/tauri/releases/tag/tauri-cli-v2.12.0) release notes;
  - [tauri-cli-v2.12.3](https://github.com/tauri-apps/tauri/releases/tag/tauri-cli-v2.12.3) (NSIS 3.13, uninstall cleanup);
  - [GHSA-w28w-mhc8-qvjv](https://github.com/tauri-apps/tauri/security/advisories/GHSA-w28w-mhc8-qvjv);
  - [dialog plugin 2.8.0](https://github.com/tauri-apps/plugins-workspace/releases/tag/dialog-v2.8.0);
  - [Tauri SvelteKit guide](https://v2.tauri.app/start/frontend/sveltekit/);
  - upstream NSIS template: `https://raw.githubusercontent.com/tauri-apps/tauri/tauri-cli-v<version>/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi`.
- zip:
  - [CHANGELOG](https://github.com/zip-rs/zip2/blob/master/CHANGELOG.md);
  - advisories [RUSTSEC-2026-0367](https://rustsec.org/advisories/RUSTSEC-2026-0367), [-0368](https://rustsec.org/advisories/RUSTSEC-2026-0368), [-0369](https://rustsec.org/advisories/RUSTSEC-2026-0369), [-0429](https://rustsec.org/advisories/RUSTSEC-2026-0429), [-0545](https://rustsec.org/advisories/RUSTSEC-2026-0545), [-0548](https://rustsec.org/advisories/RUSTSEC-2026-0548).
- Vite:
  - [migration from v7 (Vite 8)](https://vite.dev/guide/migration);
  - [migration from v6 (Vite 7)](https://v7.vite.dev/guide/migration).
- [vite-plugin-svelte CHANGELOG](https://github.com/sveltejs/vite-plugin-svelte/blob/main/packages/vite-plugin-svelte/CHANGELOG.md).
- [Vitest 5 migration guide](https://vitest.dev/guide/migration).
- [happy-dom v21.0.0](https://github.com/capricorn86/happy-dom/releases/tag/v21.0.0).
- [Announcing TypeScript 6.0](https://devblogs.microsoft.com/typescript/announcing-typescript-6-0/).
- SvelteKit:
  - [Migrating to SvelteKit v3](https://svelte.dev/docs/kit/migrating-to-sveltekit-3);
  - [`sv migrate`](https://svelte.dev/docs/cli/sv-migrate);
  - [single-page apps](https://svelte.dev/docs/kit/single-page-apps);
  - [adapter-static CHANGELOG](https://github.com/sveltejs/kit/blob/main/packages/adapter-static/CHANGELOG.md).
- chrono and alternatives:
  - [RUSTSEC-2026-0543](https://rustsec.org/advisories/RUSTSEC-2026-0543);
  - [jiff `DateTimePrinter`](https://docs.rs/jiff/latest/jiff/fmt/temporal/struct.DateTimePrinter.html).
