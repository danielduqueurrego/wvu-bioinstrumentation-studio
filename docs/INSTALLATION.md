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
- **Arduino tools.** The writable runtime (about 700 MB per Windows account) is in `%LOCALAPPDATA%\WVU-BMEG\rt`, or, for an account with a profile folder name longer than 24 characters, in `%ProgramData%\WVU-BMEG-<id>\rt` (see [Included tools](#included-tools)).
- **Uninstalling.** Uninstalling the application does not delete student Project folders or recordings. It does not remove the Arduino tools either. A `%ProgramData%\WVU-BMEG-<id>` folder (about 700 MB, one for each account with a long profile folder name) also stays when the Windows profile is deleted, so on shared lab computers that remove student profiles these folders build up. An administrator can delete them once the account is gone or the application is no longer used; if the account uses the application again, it extracts the tools again.

## Offline use

After installation, normal board discovery, firmware verification and restoration, acquisition, calibration, and export work without Internet access. The application does not automatically update Arduino packages.

## Update from version 1.0.x

Run the new installer of the same type as before: the NSIS setup over an NSIS installation, the MSI over an MSI installation. The setup is designed to remove 1.0.x before installing (for the NSIS setup, with its default choice **Uninstall before installing**), so **Installed apps** keeps one entry.

- **Publisher.** From version 1.1.0, **Installed apps** names the publisher West Virginia University Department of Chemical and Biomedical Engineering instead of `wvu`.
- **Installation folder.** The NSIS setup proposes and keeps the 1.0.x folder, including a custom one. The MSI package installs into the default Program Files folder, even when 1.0.x was installed elsewhere.
- **If the setup says "Unable to uninstall".** Uninstall 1.0.x from **Installed apps**, then run the setup again.
- **Your data.** Project folders, recordings, calibration presets, and local labs are kept: they are stored per user, not in the installation folder.

The updated app may show **Preparing Arduino tools…** once while it sets up its new Arduino tools. Then update the boards as described below.

## Boards after an update

This version uses WVU firmware build `0x00010007`. A board restored by version 1.0.x shows **Older WVU firmware** and needs one **Restore WVU Firmware**. Roll the update out in this order:

1. Update the application on **every** computer that connects to the course boards: lab computers, instructors' laptops, and students' own installations. Ask students to install this version before the boards are restored.
2. Then restore each board once, from any updated computer.

Do not restore a board from a computer still on 1.0.x. That version reports a restored board as **Firmware update required** and suggests **Restore WVU Firmware**, but restoring there puts its older firmware (build `0x00010006`) back, and the next updated computer asks for a restore again. Update that computer instead (see [Troubleshooting](TROUBLESHOOTING.md#older-or-newer-wvu-firmware)).

## Uninstall

Use Windows **Installed apps** or the Start-menu uninstall entry. Remove a Project folder manually only when its recordings are no longer needed. The Arduino tools folders are not removed either, including the `%ProgramData%\WVU-BMEG-<id>` folders of accounts with long profile folder names (see [User data](#user-data)).

## Installer status

The installers are unsigned unless the repository owner supplies an approved Windows code-signing workflow. Windows may therefore display a trust warning.

Teaching use only — not a medical device.
