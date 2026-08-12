# Third-party components and notices

The Apache-2.0 license in the repository root applies to the first-party WVU Bioinstrumentation Studio project. It does not change, replace, or grant licenses for third-party software.

The Windows distribution bundles or uses components including Tauri, Rust crates, Svelte, Vite, uPlot, WebView2 installation support, Arduino CLI, and the Arduino UNO R4/Renesas core and toolchain.

The Node dependency lockfile and Rust lockfile identify the exact frontend and Rust dependencies used to build the application. Their upstream licenses remain applicable. The distribution uses WebView2 installation support according to Microsoft's applicable terms.

The Arduino runtime is packaged separately as `src-tauri/resources/arduino-runtime.zip` for release builds. Its pinned manifest names Arduino CLI `1.5.2-rc.1` and the UNO R4 core `1.6.0`. The reviewed archive contains upstream license files for the bundled Renesas core, Arduino discovery tools, and GCC toolchain; it must retain all upstream license and notice files supplied by its components.

Before public distribution, maintainers must verify that the runtime archive includes all upstream notices required for the exact Arduino CLI, core, compiler, uploader, and WebView2 redistribution artifacts. This document does not grant or infer third-party licenses.
