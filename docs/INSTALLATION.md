# Installation

## Install on Windows

Use the WVU Bioinstrumentation Studio NSIS setup executable supplied by the course (`WVU_Bioinstrumentation_Studio_<version>_x64-setup.exe`). It is the primary installer.

1. Run the setup executable.
2. Approve the Windows administrator prompt for the system-wide installation.
3. Complete setup, then launch WVU Bioinstrumentation Studio from the Start menu.

The application installs in the normal Program Files location and is available to users of that computer. It runs as a standard user after installation.

### MSI package

The course distribution folder and ZIP also contain an MSI package (`WVU_Bioinstrumentation_Studio_<version>_x64.msi`). It is the same application in an MSI package, for managed IT deployment (for example Intune or Group Policy).

**Use only one installer type per computer.** Students should use the NSIS setup executable. Installing the MSI on a computer that already has the NSIS installation (or the reverse) can leave two uninstall entries for the same Program Files application. Uninstall one before installing the other.

`SHA256SUMS.txt` in the distribution folder lists the checksums of both installers.

## Included tools

The distribution bundles its tested Arduino CLI, Arduino UNO R4 core, compiler/upload tools, and an offline WebView2 installer. Arduino IDE, Arduino CLI, Node.js, Rust, Git, and development tools are not required for normal course use.

On first use, the app may show **Preparing Arduino tools…** while it prepares a per-user writable runtime (about 700 MB) in `%LOCALAPPDATA%\WVU-BMEG\rt`. On a Windows account whose profile folder name is longer than 24 characters, that path is too long for the included compiler, so the app uses `%ProgramData%\WVU-BMEG-<id>\rt` instead (`<id>` is derived from the account): a folder it creates with a protected access list that only that account, SYSTEM and Administrators can open. IT policies that restrict `%ProgramData%` must let standard users create that folder ([Troubleshooting](TROUBLESHOOTING.md#arduino-tools-cannot-be-set-up-for-this-windows-account)). This does not modify an existing Arduino IDE installation or its global configuration. If preparation fails, the app says that the Arduino tools need attention (in the message under the recording bar while Source is **Hardware**, otherwise in the Board card) and, when it can help, offers **Repair Arduino tools**; Simulator recordings still work (see [Troubleshooting](TROUBLESHOOTING.md#arduino-tools-problem)).

## User data

Project folders, recordings, calibration presets, local instructor customizations, logs, and the writable Arduino runtime are stored in per-user locations; none is written to Program Files.

- **Default Project folder.** The default is `Documents\BMEG 420L` in the user's Documents folder, found through Windows, so a OneDrive-redirected Documents folder works.
- **App data.** Calibration presets, local labs, logs, and the Project folder setting are under `%LOCALAPPDATA%\WVU Bioinstrumentation Studio`.
- **Uninstalling.** Uninstalling the application does not delete student Project folders or recordings.

## Offline use

After installation, normal board discovery, firmware verification and restoration, acquisition, calibration, and export work without Internet access. The application does not automatically update Arduino packages.

## Uninstall

Use Windows **Installed apps** or the Start-menu uninstall entry. Remove a Project folder manually only when its recordings are no longer needed.

## Installer status

The installers are unsigned unless the repository owner supplies an approved Windows code-signing workflow. Windows may therefore display a trust warning.

Teaching use only — not a medical device.
