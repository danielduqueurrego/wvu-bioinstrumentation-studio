# Installation

## Install on Windows

Use the WVU Bioinstrumentation Studio NSIS setup executable supplied by the course. It is the primary installer.

1. Run the setup executable.
2. Approve the Windows administrator prompt for the system-wide installation.
3. Complete setup, then launch WVU Bioinstrumentation Studio from the Start menu.

The application installs in the normal Program Files location and is available to users of that computer. It runs as a standard user after installation.

## Included tools

The distribution bundles its tested Arduino CLI, Arduino UNO R4 core, compiler/upload tools, and an offline WebView2 installer. Arduino IDE, Arduino CLI, Node.js, Rust, Git, and development tools are not required for normal course use.

On first use, the app may show **Preparing Arduino tools…** while it prepares a per-user writable runtime. This does not modify an existing Arduino IDE installation or its global configuration.

## User data

Project folders, recordings, calibration presets, local instructor customizations, logs, and the writable Arduino runtime are stored in per-user locations. They are not written into Program Files. Uninstalling the application does not delete student Project folders or recordings by default.

## Offline use

After installation, normal board discovery, firmware verification/restoration, acquisition, calibration, and export work without Internet access. The application does not automatically update Arduino packages.

## Uninstall

Use Windows **Installed apps** or the Start-menu uninstall entry. Remove a Project folder manually only when its recordings are no longer needed.

## Installer status

The installer is unsigned unless the repository owner supplies an approved Windows code-signing workflow. Windows may therefore display a trust warning.
