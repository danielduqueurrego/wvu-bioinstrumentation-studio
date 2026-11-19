# Contributing

WVU Bioinstrumentation Studio is teaching software for BMEG 420L. It is not a medical device: keep the teaching-use language, and do not add features that diagnose, treat, or make clinical decisions.

Start with the [Developer Guide](docs/DEVELOPER_GUIDE.md). Keep changes focused, format Rust code, and run the [documented checks](docs/DEVELOPER_GUIDE.md#setup-and-checks) before opening a pull request.

- Do not commit recordings, student data, local calibrations, runtime caches, installers, or build output.
- Keep raw acquisition data authoritative; discuss any physiological-analysis feature before implementing it.
- Preserve the Arduino safety invariants and the production Tauri build path.
- Include tests for changed behavior and update current documentation when the user workflow changes.

Report security problems privately as described in [SECURITY.md](SECURITY.md), never in a public issue or pull request. To ask for a new lab or a change to a built-in lab, use the **Lab request** issue form.

## License of contributions

The project is licensed under the [Apache License, Version 2.0](LICENSE). Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in the project is licensed under the Apache License, Version 2.0, as described in Section 5 of the license, without any additional terms or conditions. Only submit work that you have the right to license this way.
