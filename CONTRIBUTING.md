# Contributing

Start with the [Developer Guide](docs/DEVELOPER_GUIDE.md). Keep changes focused, format Rust code, and run the documented checks before opening a pull request.

- Do not commit recordings, student data, local calibrations, runtime caches, installers, or build output.
- Keep raw acquisition data authoritative; discuss any physiological-analysis feature before implementing it.
- Preserve the Arduino safety invariants and the production Tauri build path.
- Include tests for changed behavior and update current documentation when the user workflow changes.
