# Security policy

WVU Bioinstrumentation Studio is teaching software for biomedical-instrumentation laboratory courses. It is not a medical device and must not be used for diagnosis, treatment, or clinical decisions.

## Supported versions

Security fixes are made for the latest release line only: the newest `MAJOR.MINOR` version on the [Releases](https://github.com/danielduqueurrego/wvu-bioinstrumentation-studio/releases) page. A fix ships as a new release; older releases are not updated.

| Version | Supported |
| --- | --- |
| Latest release line (for example `1.0.x` while 1.0.2 is the newest release) | Yes |
| Older releases | No. Update to the latest release. |

## Reporting a vulnerability

Report vulnerabilities privately through GitHub private vulnerability reporting:

1. Open the repository's **Security** tab and select **Report a vulnerability**, or go directly to <https://github.com/danielduqueurrego/wvu-bioinstrumentation-studio/security/advisories/new>.
2. Describe the problem in the form and submit it. Only you and the maintainers can see the report.

**Do not put vulnerability details in public issues, pull requests, or discussions.** If you are unsure whether something is a security problem, report it privately anyway.

Please include:

- The application version and installer type (NSIS setup or MSI), and the Windows version.
- The firmware build shown in **Advanced details**, if the board is involved.
- The affected part: the desktop application, the WVU reference firmware, a lab definition, or a build or release script.
- What an attacker or a faulty input could do, and what it would need (for example local access, a crafted lab file, or a crafted recording).
- Steps to reproduce, with a minimal proof of concept if you have one.
- Any fix or mitigation you suggest.

Never include participant data, physiological recordings, calibration data, or student-identifying information. Remove personal paths and names from logs before attaching them.

## What to expect

This is a teaching project maintained on a best-effort basis. The maintainer aims to acknowledge reports, but there is no guaranteed response or fix time, and replies can be slower between academic terms. The conversation stays in the private advisory. Once a fix is released, the advisory may be published, and it may credit the reporter.

## Scope

In scope:

- The Windows desktop application (`src/`, `src-tauri/`), including lab-file import, recording files, and how the application bundles, extracts, and runs the Arduino tools.
- The WVU reference firmware (`firmware/reference_unor4wifi/`) and the USB protocol.
- The release and build scripts (`scripts/`), the installers they produce, and the GitHub workflows.

Hardware-safety problems are welcome too, even when they are not classic security bugs: for example a D4, D5, or D6 output that stays HIGH after Stop, a fault, or the board watchdog, or the red and IR LEDs driven HIGH at the same time.

Out of scope: vulnerabilities in third-party components themselves (Arduino CLI and tools, Tauri, WebView2, Rust crates, npm packages). Report those to their upstream projects. Do tell us if the way this project bundles or uses them makes a problem worse.
