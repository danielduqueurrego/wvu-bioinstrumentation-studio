# Third-party components and notices

The Apache-2.0 license in the repository root applies to the first-party WVU Bioinstrumentation Studio project. It does not change, replace, or grant licenses for third-party software.

The Windows distribution bundles or uses components including Tauri, Rust crates, Svelte, Vite, uPlot, WebView2 installation support, Arduino CLI, and the Arduino UNO R4/Renesas core and toolchain.

The Node dependency lockfile and Rust lockfile identify the exact frontend and Rust dependencies used to build the application. Their upstream licenses remain applicable. The distribution uses WebView2 installation support according to Microsoft's applicable terms.

The Arduino runtime is packaged separately as `src-tauri/resources/arduino-runtime.zip` for release builds. Its pinned manifest names Arduino CLI `1.5.2-rc.1` and the UNO R4 core `1.6.0`. The reviewed archive contains upstream license files for the bundled Renesas core, Arduino discovery tools, and GCC toolchain; it must retain all upstream license and notice files supplied by its components. The installed application also includes `src-tauri/resources/licenses/BUNDLED_COMPONENTS.txt` and the upstream BOSSA license because the Arduino BOSSA binary package does not carry that license file inside the assembled runtime archive.

Before publishing or updating a public distribution, maintainers must verify the exact Arduino CLI license copy, the component-specific BOSSA notice, and the retained upstream notices for the core, compiler, discovery tools, and WebView2 redistribution artifact. Run `scripts/audit_bundled_runtime_notices.ps1`; it fails when the reviewed component notices or required runtime entries are absent. This document does not grant or infer third-party licenses.
