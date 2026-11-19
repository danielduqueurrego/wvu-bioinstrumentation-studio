# Third-party components and notices

The Apache-2.0 license in the repository root applies to the first-party WVU Bioinstrumentation Studio project; the root `NOTICE` gives its copyright line. The installers place copies of both (`LICENSE.txt`, `NOTICE.txt`) in the application's `resources\licenses` folder. The license and NOTICE do not change, replace, or grant licenses for third-party software.

The Windows distribution bundles or uses components including Tauri, Rust crates, Svelte, Vite, uPlot, WebView2 installation support, Arduino CLI, and the Arduino UNO R4/Renesas core and toolchain.

The Node dependency lockfile and Rust lockfile identify the exact frontend and Rust dependencies used to build the application. Their upstream licenses remain applicable. The distribution uses WebView2 installation support according to Microsoft's applicable terms.

## Offline Arduino runtime

The Arduino runtime is packaged separately as `src-tauri/resources/arduino-runtime.zip` for release builds. Runtime **revision 4** (from version 1.1.0) holds:

| Component | Version | License | Path in the runtime |
| --- | --- | --- | --- |
| Arduino CLI | 1.6.0 | GPL-3.0-only | `arduino-cli.exe` (license `arduino-cli-LICENSE.txt`) |
| Arduino serial-discovery, mdns-discovery, serial-monitor | 1.5.2, 1.1.0, 0.15.0 | GPL-3.0-only | `data/packages/builtin/tools/…` |
| Arduino dfu-discovery (with dfu-util 0.11 files and libusb 1.0.26) | 0.1.2 | GPL-3.0-only (GPL-2.0-or-later, LGPL-2.1-or-later parts) | `data/packages/builtin/tools/dfu-discovery/0.1.2/` |
| Exuberant Ctags, Arduino build | 5.8-arduino12 | GPL-2.0-or-later | `data/packages/builtin/tools/ctags/5.8-arduino12/` |
| GNU Arm Embedded Toolchain (GCC 7.2.1, Binutils, Newlib, GDB) with statically linked GMP, MPFR, MPC, isl, libiconv, Expat, zlib | 7-2017q4 | GPL-3.0-or-later and others (`share/doc/gcc-arm-none-eabi/license.txt`) | `data/packages/arduino/tools/arm-none-eabi-gcc/7-2017q4/` |
| Arduino UNO R4 core (ArduinoCore-renesas, with ArduinoCore-API 1.5.2) | 1.6.0 | MIT, LGPL-2.1-or-later and others | `data/packages/arduino/hardware/renesas_uno/1.6.0/` |
| BOSSA uploader (bossac), Arduino build | 1.9.1-arduino5 | BSD-3-Clause | `data/packages/arduino/tools/bossac/1.9.1-arduino5/` |

The archive root also carries the Arduino CLI release's own `LICENSE.txt` as `arduino-cli-LICENSE.txt` (GPLv3) and the GNU `GPL-2.0.txt` and `LGPL-2.1.txt`; the other upstream license and notice files stay in place inside the archive. OpenOCD 0.11.0-arduino2 and dfu-util 0.11.0-arduino5, which revision 3 (versions 1.0.0 to 1.0.2, Arduino CLI `1.5.2-rc.1`) carried, were removed in revision 4: the UNO R4 WiFi uploads with bossac and never uses them.

The installed application includes `src-tauri/resources/licenses/BUNDLED_COMPONENTS.txt`, which gives each component's version, license, path and upstream source, and the upstream BOSSA license (`bossac-LICENSE.txt`), because the Arduino BOSSA package does not carry that license file. `release/arduino-runtime-entries.json` pins the license files and their SHA-256 values; `release/arduino-runtime-4.inventory.txt` lists every license file in the archive.

## Corresponding source

Every release that carries the runtime publishes `WVU-Bioinstrumentation-Studio-<version>-third-party-sources.zip` and its `.sha256` beside its installers. `scripts/build_third_party_sources.ps1` builds it from `release/third-party-sources.json`; it holds the source of every GPL and LGPL component (with the Go modules of the Go programs, vendored, and the toolchain's statically linked host libraries), the BOSSA license, the GNU license texts, `SHA256SUMS.txt`, and a `README.txt` with clear build directions and a written offer valid for at least three years. Requests go through the repository's "Request corresponding source" issue form.

`release/third-party-sources-runtime3.json` describes the bundle for 1.0.0 to 1.0.2. For the OpenOCD and dfu-util builds in that runtime, Arduino has not published the exact source and the maintainers do not hold it: that bundle contains the closest upstream releases and Arduino's build scripts and sources as found, says so, and states that requests for the exact build sources are forwarded to Arduino and anything received is published. The 1.0.x bundle is not attached to the published 1.0.0-1.0.2 releases; attaching it changes those releases and needs the maintainer's explicit go-ahead.

## Before publishing

Before publishing or updating a public distribution, maintainers must verify the exact Arduino CLI license copy, the component-specific BOSSA notice, and the retained upstream notices for the core, compiler, discovery tools, and WebView2 redistribution artifact. Run `scripts/audit_bundled_runtime_notices.ps1`; it fails when a reviewed license file or required runtime entry is absent or changed, when the archive holds a tool folder without a reviewed notice, or when `BUNDLED_COMPONENTS.txt` does not name the bundled versions. Run `scripts/check_third_party_sources.ps1`, and publish the source bundle with the installers. This document does not grant or infer third-party licenses.
