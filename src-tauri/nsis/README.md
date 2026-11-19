# Custom NSIS installer template

`src-tauri/tauri.conf.json` sets `bundle.windows.nsis.template` to `nsis/installer.nsi` (relative to `src-tauri`, where `tauri build` runs the bundler). The bundler still writes its own `utils.nsh`, `FileAssociation.nsh`, and language files beside the rendered template, so they are not copied here.

| File | Contents |
| --- | --- |
| `installer.upstream.nsi` | Tauri's NSIS template for the CLI version below, unmodified. Do not edit it. |
| `installer.nsi` | The same file plus regions between `; WVU-LEGACY-PUBLISHER BEGIN` and `; WVU-LEGACY-PUBLISHER END` lines. |

Both files are Handlebars templates: the bundler fills in `{{...}}` placeholders, so never write two opening braces in a comment.

## Why

Versions 1.0.0 to 1.0.2 set no `bundle.publisher`, so Tauri used `wvu`, the second segment of the identifier `edu.wvu.bioinstrumentationstudio`, as the publisher. From 1.1 the publisher is the department. The upstream template keys three things on the publisher, and without the marked regions a 1.1 setup would mishandle a 1.0.x install:

- **Uninstalling the previous NSIS install.** The template reads the install folder from `SHCTX\Software\<publisher>\<product>` and passes it to the old uninstaller as `_?=`. The new key is empty for a 1.0.x install, so the old uninstaller would start with an empty folder and fail. The region falls back to `Software\wvu\<product>`, then to the folder of the registered `UninstallString` if the app binary is there. If neither gives a folder, it shows the template's "Unable to uninstall" message and aborts, as the template's own failure path does; it never starts the uninstaller with an empty `_?=`. In the GUI the abort returns to the reinstall page. In passive mode (`/P`) the page is skipped, so setup installs without uninstalling 1.0.x, as upstream also does after "Unable to uninstall". When a folder is found, the region ends with `ClearErrors` and reads `UninstallString` again: the failed read of the new key sets NSIS's error flag, which stays set until something clears it, and the template would then report the successful uninstall as failed. The second read leaves the flag set if `UninstallString` is missing, as upstream does.
- **Reusing the install folder.** `RestorePreviousInstallLocation` falls back to `Software\wvu\<product>`, so a custom 1.0.x folder is kept.
- **Detecting an MSI install.** The template finds an MSI install by `DisplayName` and `Publisher`. The region also accepts Publisher `wvu`, so a 1.0.x MSI install is detected and removed instead of leaving two Installed-apps entries.

Everything else is upstream. The MSI needs no template: Tauri's WiX template has `MajorUpgrade`, and `bundle.windows.wix.upgradeCode` is pinned to the code the 1.0.x MSIs used, so a 1.1 MSI removes a 1.0.x MSI install. The WiX template looks for the previous folder under `HKCU\Software\<publisher>\<product>`, so an MSI upgrade from 1.0.x installs into the default folder, not a custom one.

## Upstream record

`scripts/check_release_consistency.ps1` reads this table. It fails in any of these cases:

- The installed or locked version of `@tauri-apps/cli`, or of `@tauri-apps/cli-win32-x64-msvc`, differs from the recorded one. The platform package is the one whose binary embeds the template.
- `installer.upstream.nsi` no longer has the recorded SHA-256 (computed with LF line endings).
- `installer.nsi` differs from it outside the marked regions.
- The regions lack a 1.0.x fallback, or the `ClearErrors` / `UninstallString` lines at the end of the `PageLeaveReinstall` region.

| Field | Value |
| --- | --- |
| Tauri CLI version | `2.11.4` |
| Source tag | `tauri-cli-v2.11.4` (commit `8909f221d1515955fc843808032bdc5d62209c96`; `@tauri-apps/cli-v2.11.4` serves the same file) |
| Source URL | https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi |
| SHA-256 of `installer.upstream.nsi` (LF line endings) | `20f4ecc730defb71f1342eaeaec4021df13be3d843abba0effe88ea5835fa079` |

The file was also compared with the template embedded in `node_modules/@tauri-apps/cli-win32-x64-msvc/cli.win32-x64-msvc.node` 2.11.4: the binary contains it byte for byte, with CRLF line endings.

When the Tauri CLI changes, follow [Re-syncing the NSIS template](../../docs/DEVELOPER_GUIDE.md#re-syncing-the-nsis-template).
