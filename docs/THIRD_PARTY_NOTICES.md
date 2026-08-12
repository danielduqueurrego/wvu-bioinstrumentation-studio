# Third-party components and notices

The Windows distribution bundles or uses components including Tauri, Rust crates, Svelte, Vite, uPlot, WebView2 installation support, Arduino CLI, and the Arduino UNO R4/Renesas core and toolchain.

The Arduino runtime is packaged separately as `src-tauri/resources/arduino-runtime.zip` for release builds. Its pinned manifest names Arduino CLI `1.5.2-rc.1` and the UNO R4 core `1.6.0`. The runtime archive must retain the upstream license and notice files supplied by its components; the bundled Renesas core contains its own `LICENSE` file.

Before public distribution, maintainers must verify that the runtime archive includes all upstream notices required for the exact Arduino CLI, core, compiler, uploader, and WebView2 redistribution artifacts. This document does not grant or infer third-party licenses.
