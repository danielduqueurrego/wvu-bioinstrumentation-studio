<#
.SYNOPSIS
Builds the offline Arduino runtime archive (arduino-runtime.zip) that WVU
Bioinstrumentation Studio bundles, and proves that it compiles the reference
firmware without network access.

.DESCRIPTION
The application never uses a student's Arduino IDE, PATH or Arduino15 folder.
It extracts src-tauri/resources/arduino-runtime.zip to
%LOCALAPPDATA%\WVU-BMEG\rt, writes its own arduino-cli.yaml there, applies the
native-USB SerialUSB.h compatibility shim and runs Arduino CLI from that folder
(src-tauri/src/arduino_runtime.rs). This script assembles that archive from
pinned upstream inputs. It never changes the repository: it reads the
reference sketch and src-tauri/src/arduino_cli.rs, and writes only under
-WorkRoot and beside -OutputZip.

 1. Downloads Arduino CLI for Windows x64 into its own folder and checks the
    archive's SHA-256 against the value pinned below and against the release's
    published checksums file. Only that verified arduino-cli.exe is ever run.
 2. Writes a build-only CLI configuration. Its data, downloads, user and build
    cache folders are all under -WorkRoot, never the maintainer's Arduino15.
 3. Installs the pinned core (Arduino CLI checks every package against the
    signed package index) and updates the library index. Arduino CLI also
    installs its built-in tools (discovery, monitor, ctags) at the versions
    the current package index names, so the script requires exactly the tool
    versions pinned below and stops on any difference: a new tool version
    needs a new license and source review.
 4. Offline proof: copies the staged runtime into a simulated
    %LOCALAPPDATA%\WVU-BMEG\rt whose length is -ProofRootLength, applies the
    application's SerialUSB.h shim, writes the application's arduino-cli.yaml
    (the same text as cli_config_text in src-tauri/src/arduino_runtime.rs),
    sends all HTTP(S) traffic to a dead proxy, compiles the reference sketch
    with the application's exact arguments, runs board details, and fails if
    any file appears or changes under data/packages or downloads.
 5. Unless -KeepUnusedTools is given, removes the openocd and dfu-util
    tools, which the UNO R4 WiFi never uses (it uploads with bossac; openocd
    is for debugging, dfu-util for the UNO R4 Minima), and repeats the offline
    proof. If that proof fails, the tools are put back and kept.
 6. Leaves out everything the application must not inherit: downloads, the
    sketchbook, the build cache, Arduino CLI's per-installation
    data/inventory.yaml (it holds an installation ID and secret) and any
    arduino-cli.yaml. Adds runtime-manifest.json, arduino-cli-LICENSE.txt and
    the GPL-2.0 and LGPL-2.1 texts.
 7. Writes the ZIP deterministically: ordinal-sorted entry names, '/'
    separators, one fixed timestamp, no wrapper folder. Writes <zip>.sha256
    (sha256sum layout) and arduino-runtime-<revision>.inventory.txt beside it.
 8. Extracts the finished ZIP into a fresh simulated profile and repeats the
    offline proof. When release/arduino-runtime-entries.json describes the
    same bundle revision, the archive must contain its required entries and
    license files (with their SHA-256), exactly its tool and platform folders,
    and none of its forbidden entries; for another revision the differences
    are only reported, so that file can be updated after review.

What a rebuild reproduces: the ZIP is byte-identical when written twice from
the same staged folder with the same PowerShell/.NET version (whose Deflate
encoder writes the bytes; see -CheckDeterminism). Two builds are not: Arduino
CLI writes the core's installed.json with its "tools" list in random order (Go
map iteration; same content, same size), installs the newest built-in
discovery tools listed in the package index, and saves the current package and
library indexes. Each build therefore has its own SHA-256, and the reviewed
archive itself, not this script, is what the release pins.

After review, a maintainer copies the ZIP to
src-tauri/resources/arduino-runtime.zip and updates both runtime manifests, the
reviewed SHA-256 in release/release-manifest.json,
release/arduino-runtime-entries.json, release/third-party-sources.json and
the bundled-component notices (docs/DEVELOPER_GUIDE.md, "Rebuilding the
Arduino runtime"). Runtime revision 4 (SHA-256 5baa8a5f...) was built on
2026-11-18 by the version of this script before its review fixes (default
work root and -Force marker, -KeepUnusedTools, relative paths, pinned tool
versions, the entries file and the 59-character proof); the build steps and
their inputs are the same.

Path length: GCC 7-2017q4 builds its C++ multilib include paths from its own
location with unresolved '..' segments. Measured on 2026-11-18, the reference
sketch compiles when the runtime root is at most 59 characters long and fails
from 60 characters on with a missing header, such as "bits/c++config.h: No
such file or directory" (bits/c++allocator.h and bits/cpu_defines.h were also
seen). The application's root,
C:\Users\<profile folder>\AppData\Local\WVU-BMEG\rt, is 35 characters plus the
profile folder name; when it is 60 characters or longer, the application uses
a private per-user folder, %ProgramData%\WVU-BMEG-<id>\rt, or reports
runtime_path_too_long (resolve_runtime_root and RUNTIME_ROOT_LENGTH_LIMIT in
src-tauri/src/arduino_runtime.rs). An 8.3 short path does not help: Arduino
CLI expands it back to the long path before it runs GCC. The proofs therefore
run at 59 characters, the longest root the application uses.

.PARAMETER WorkRoot
Folder for all intermediate files. Default: C:\wvu-rtbuild. Keep it short
(at most 28 characters with the default -ProofRootLength): the offline proofs
place a 59-character runtime root under it. The script marks the folder it
uses (file .wvu-runtime-build); -Force deletes the contents of a non-empty
folder only when that marker is present.

.PARAMETER OutputZip
Path of the archive to write. Default: <WorkRoot>\output\arduino-runtime.zip.
The .sha256 and inventory files are written beside it.

.PARAMETER BundleRevision
bundle_revision written to runtime-manifest.json. Increase it whenever the
archive content changes, so installed copies extract the new runtime.

.PARAMETER KeepUnusedTools
Keep openocd and dfu-util in the archive. By default they are removed when
the offline proof passes without them.

.PARAMETER ReferenceSketch
The reference .ino to compile. Default:
<ProjectRoot>\firmware\reference_unor4wifi\reference_unor4wifi.ino.

.PARAMETER ProjectRoot
Repository root, read for the reference sketch, the application's compile
arguments and the release scripts' expectations. Default: the parent of this
script's folder.

.PARAMETER DownloadCache
Optional folder for Arduino CLI's package downloads, outside -WorkRoot, so a
-Force rebuild does not download the toolchain again. Arduino CLI checks every
cached package against the signed package index before using it.

.PARAMETER ProofRootLength
Length, in characters, of the runtime root used by the offline proofs.
Default: 59, the longest root the application uses (one less than
RUNTIME_ROOT_LENGTH_LIMIT in src-tauri/src/arduino_runtime.rs).

.PARAMETER CheckDeterminism
Write the ZIP a second time from the same folder and require the same SHA-256.

.PARAMETER Force
Empty a non-empty -WorkRoot that carries this script's marker, and replace an
existing -OutputZip.

.EXAMPLE
pwsh -NoProfile -File scripts\build_arduino_runtime.ps1 -OutputZip C:\wvu-rtout\arduino-runtime.zip -BundleRevision 4 -DownloadCache C:\wvu-arduino-downloads
#>
#Requires -Version 7.2
[CmdletBinding()]
param(
  [string]$WorkRoot = 'C:\wvu-rtbuild',
  [string]$OutputZip,
  [ValidateRange(1, 1000000)][int]$BundleRevision = 4,
  [switch]$KeepUnusedTools,
  [string]$ReferenceSketch,
  [string]$ProjectRoot,
  [string]$DownloadCache,
  [ValidateRange(20, 259)][int]$ProofRootLength = 59,
  [switch]$CheckDeterminism,
  [switch]$Force
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
# Native exit codes are checked explicitly below.
$PSNativeCommandUseErrorActionPreference = $false

# --- Pinned inputs --------------------------------------------------------------
# Changing any of these needs a review of the new upstream component, its
# licenses and the release metadata.

$CliVersion = '1.6.0'
$CliArchiveName = "arduino-cli_${CliVersion}_Windows_64bit.zip"
$CliArchiveUrl = "https://downloads.arduino.cc/arduino-cli/$CliArchiveName"
$CliChecksumsUrl = "https://github.com/arduino/arduino-cli/releases/download/v$CliVersion/$CliVersion-checksums.txt"
$CliArchiveSha256 = '2769b9b282ce597db9679d8ec14d726d57004ca4205ef4d7a795bf36b6cdc9d8'

$CorePackager = 'arduino'
$CoreArchitecture = 'renesas_uno'
$CoreVersion = '1.6.0'
# Tool versions that arduino:renesas_uno@1.6.0 depends on and the app needs.
$GccVersion = '7-2017q4'
$BossacVersion = '1.9.1-arduino5'
# Tools of arduino:renesas_uno that the UNO R4 WiFi never runs.
$UnusedTools = @('openocd', 'dfu-util')
# Every tool Arduino CLI installs, at the reviewed version. The core pins the
# arduino:* tools; Arduino CLI installs the newest builtin:* tools that the
# current package index lists, so those can change without any change here.
# The build stops on any difference: a new tool version needs a new license
# and source review (release/third-party-sources.json, BUNDLED_COMPONENTS.txt,
# release/arduino-runtime-entries.json) before its pin is updated.
$PinnedTools = @(
  [pscustomobject]@{ Package = 'arduino'; Name = 'arm-none-eabi-gcc'; Version = $GccVersion },
  [pscustomobject]@{ Package = 'arduino'; Name = 'bossac'; Version = $BossacVersion },
  [pscustomobject]@{ Package = 'arduino'; Name = 'dfu-util'; Version = '0.11.0-arduino5' },
  [pscustomobject]@{ Package = 'arduino'; Name = 'openocd'; Version = '0.11.0-arduino2' },
  [pscustomobject]@{ Package = 'builtin'; Name = 'ctags'; Version = '5.8-arduino12' },
  [pscustomobject]@{ Package = 'builtin'; Name = 'dfu-discovery'; Version = '0.1.2' },
  [pscustomobject]@{ Package = 'builtin'; Name = 'mdns-discovery'; Version = '1.1.0' },
  [pscustomobject]@{ Package = 'builtin'; Name = 'serial-discovery'; Version = '1.5.2' },
  [pscustomobject]@{ Package = 'builtin'; Name = 'serial-monitor'; Version = '0.15.0' }
)
# Written into -WorkRoot; -Force empties only a folder that has it.
$WorkRootMarker = '.wvu-runtime-build'

# GNU license texts placed at the archive root. gnu.org publishes no checksums;
# these are the SHA-256 values reviewed on 2026-11-18. A change stops the
# build so the new text is reviewed before the pins are updated.
$LicenseTexts = @(
  [pscustomobject]@{
    Name = 'GPL-2.0.txt'
    Url = 'https://www.gnu.org/licenses/old-licenses/gpl-2.0.txt'
    Sha256 = 'edaef632cbb643e4e7a221717a6c441a4c1a7c918e6e4d56debc3d8739b233f6'
    Heading = 'GNU GENERAL PUBLIC LICENSE'
    Version = 'Version 2, June 1991'
  },
  [pscustomobject]@{
    Name = 'LGPL-2.1.txt'
    Url = 'https://www.gnu.org/licenses/old-licenses/lgpl-2.1.txt'
    Sha256 = '20e50fe7aae3e56378ebf0417d9de904f55a0e61e4df315333e632a4d3555d95'
    Heading = 'GNU LESSER GENERAL PUBLIC LICENSE'
    Version = 'Version 2.1, February 1999'
  }
)

# Every archive entry gets this timestamp (the Arduino CLI release date).
$ZipTimestamp = [DateTimeOffset]::new(2026, 11, 10, 0, 0, 0, [TimeSpan]::Zero)
# Nothing listens here; any download attempt fails at once.
$DeadProxy = 'http://127.0.0.1:9'

# The application's SerialUSB.h shim. Must equal
# apply_native_usb_core_compatibility_patch in src-tauri/src/arduino_runtime.rs.
$ShimMarker = '    virtual size_t write(const uint8_t *p, size_t len) override;'
$ShimComment = "    // WVU native-USB compatibility shim for the pinned core's libc _write hook."
$ShimMember = '    size_t write_raw(uint8_t *p, size_t len) { return write(p, len); }'
$ShimSignature = 'size_t write_raw(uint8_t *p, size_t len)'

# The only files kept at the root of data/ (plus data/packages).
$DataRootFiles = @('package_index.json', 'package_index.json.sig', 'library_index.json', 'library_index.json.sig')

# --- Helpers ----------------------------------------------------------------------

function Write-Step {
  param([Parameter(Mandatory = $true)][string]$Message)
  Write-Host ''
  Write-Host "==> $Message" -ForegroundColor Cyan
}

function Get-Sha256 {
  param([Parameter(Mandatory = $true)][string]$Path)
  return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

# Relative paths are resolved against PowerShell's current location, not the
# .NET process directory (which `& script.ps1` from another folder keeps).
function Resolve-FullPath {
  param([Parameter(Mandatory = $true)][string]$Path)
  return $PSCmdlet.GetUnresolvedProviderPathFromPSPath($Path).TrimEnd('\')
}

function Assert-Sha256 {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][string]$Expected,
    [Parameter(Mandatory = $true)][string]$Description
  )
  $actual = Get-Sha256 $Path
  if ($actual -cne $Expected.ToLowerInvariant()) {
    throw "$Description has SHA-256 $actual, expected $Expected. Stopped before using it."
  }
  Write-Host "SHA-256 verified: $Description ($actual)"
}

function Format-CommandLine {
  param([string[]]$Arguments)
  return ($Arguments | ForEach-Object {
      if ($_ -eq '' -or $_ -match '[\s"]') { '"' + ($_ -replace '"', '\"') + '"' } else { $_ }
    }) -join ' '
}

function New-EmptyDirectory {
  param([Parameter(Mandatory = $true)][string]$Path)
  if (Test-Path -LiteralPath $Path) {
    if (@(Get-ChildItem -LiteralPath $Path -Force).Count -gt 0) {
      throw "Folder is not empty: $Path"
    }
  } else {
    New-Item -ItemType Directory -Path $Path | Out-Null
  }
}

# Downloads one file into its own new, empty folder.
function Save-Download {
  param(
    [Parameter(Mandatory = $true)][string]$Url,
    [Parameter(Mandatory = $true)][string]$Folder,
    [Parameter(Mandatory = $true)][string]$FileName
  )
  New-EmptyDirectory $Folder
  $path = Join-Path $Folder $FileName
  Write-Host "Downloading $Url"
  Invoke-WebRequest -Uri $Url -OutFile $path -MaximumRedirection 5
  return $path
}

$script:LogIndex = 0
$script:CommandLog = [System.Collections.Generic.List[string]]::new()

# Runs the verified arduino-cli.exe with an explicit configuration file. Any
# ARDUINO_* variable of the calling session is removed for the call, and the
# given variables are set; all are restored afterwards. Every command's output
# is saved under <WorkRoot>\logs.
function Invoke-ArduinoCli {
  param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$ConfigFile,
    [Parameter(Mandatory = $true)][string[]]$Arguments,
    [Parameter(Mandatory = $true)][hashtable]$Environment,
    [Parameter(Mandatory = $true)][string]$WorkingDirectory,
    [Parameter(Mandatory = $true)][string]$Label,
    [switch]$AllowFailure,
    [switch]$Quiet
  )
  $argv = @('--config-file', $ConfigFile) + $Arguments
  $commandLine = 'arduino-cli ' + (Format-CommandLine $argv)
  Write-Host "> $commandLine"
  $script:CommandLog.Add($commandLine)
  $names = @(Get-ChildItem env: | Where-Object Name -Like 'ARDUINO_*' | ForEach-Object Name) + @($Environment.Keys)
  $saved = @{}
  foreach ($name in ($names | Sort-Object -Unique)) {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name)
  }
  $watch = [Diagnostics.Stopwatch]::StartNew()
  $previousPreference = $ErrorActionPreference
  Push-Location -LiteralPath $WorkingDirectory
  try {
    foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $null) }
    foreach ($name in $Environment.Keys) { [Environment]::SetEnvironmentVariable($name, $Environment[$name]) }
    # Arduino CLI reports progress on stderr; only the exit code decides
    # success. The relaxed preference covers this native call only.
    $ErrorActionPreference = 'Continue'
    $output = @(& $Executable @argv 2>&1)
    $exitCode = $LASTEXITCODE
  } finally {
    $ErrorActionPreference = $previousPreference
    foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
    Pop-Location
  }
  $watch.Stop()
  $stdout = (@($output | Where-Object { $_ -isnot [Management.Automation.ErrorRecord] } | ForEach-Object { [string]$_ }) -join "`n")
  $stderr = (@($output | Where-Object { $_ -is [Management.Automation.ErrorRecord] } | ForEach-Object { $_.ToString() }) -join "`n")
  $script:LogIndex++
  $logPath = Join-Path $script:LogRoot ('{0:D2}-{1}.txt' -f $script:LogIndex, $Label)
  [IO.File]::WriteAllText($logPath,
    "> $commandLine`nexit code: $exitCode`nseconds: $([math]::Round($watch.Elapsed.TotalSeconds, 1))`n`n--- stdout ---`n$stdout`n`n--- stderr ---`n$stderr`n",
    [Text.UTF8Encoding]::new($false))
  if (-not $Quiet) {
    if ($stdout) { Write-Host $stdout }
    if ($stderr) { Write-Host $stderr -ForegroundColor Yellow }
  }
  Write-Host ("  exit code {0} after {1:N1} s (log: {2})" -f $exitCode, $watch.Elapsed.TotalSeconds, $logPath)
  if ($exitCode -ne 0 -and -not $AllowFailure) {
    throw "arduino-cli $($Arguments -join ' ') failed with exit code $exitCode. See $logPath."
  }
  return [pscustomobject]@{
    Command = $commandLine
    ExitCode = $exitCode
    StdOut = $stdout
    StdErr = $stderr
    Seconds = [math]::Round($watch.Elapsed.TotalSeconds, 1)
    Log = $logPath
  }
}

# A JSON string is also a YAML double-quoted scalar; the application writes
# its paths the same way (yaml_path in arduino_runtime.rs).
function ConvertTo-YamlPath {
  param([Parameter(Mandatory = $true)][string]$Path)
  return (ConvertTo-Json -InputObject ($Path.Replace('\', '/')) -Compress)
}

# Byte-for-byte the configuration the application writes (cli_config_text).
function Get-AppCliConfigText {
  param([Parameter(Mandatory = $true)][string]$RuntimeRoot)
  return "directories:`n  data: $(ConvertTo-YamlPath (Join-Path $RuntimeRoot 'data'))`n  downloads: $(ConvertTo-YamlPath (Join-Path $RuntimeRoot 'downloads'))`n  user: $(ConvertTo-YamlPath (Join-Path $RuntimeRoot 'sketchbook'))`n" +
    "updater:`n  enable_notification: false`nnetwork:`n  cloud_api:`n    skip_board_detection_calls: true`n"
}

function Get-SerialUsbHeaderPath {
  param([Parameter(Mandatory = $true)][string]$RuntimeRoot)
  return Join-Path $RuntimeRoot "data\packages\$CorePackager\hardware\$CoreArchitecture\$CoreVersion\cores\arduino\usb\SerialUSB.h"
}

# The application's shim, applied the same way: idempotent, first occurrence
# of the marker only, the file's own line ending, UTF-8 without a BOM change.
function Add-NativeUsbShim {
  param([Parameter(Mandatory = $true)][string]$RuntimeRoot)
  $header = Get-SerialUsbHeaderPath $RuntimeRoot
  $utf8 = [Text.UTF8Encoding]::new($false, $true)
  $source = $utf8.GetString([IO.File]::ReadAllBytes($header))
  if ($source.Contains($ShimSignature)) { return 'already present' }
  $index = $source.IndexOf($ShimMarker, [StringComparison]::Ordinal)
  if ($index -lt 0) {
    throw "The pinned core's SerialUSB.h no longer contains the line the application patches: $header"
  }
  $newline = if ($source.Contains("`r`n")) { "`r`n" } else { "`n" }
  $replacement = $ShimMarker + $newline + $ShimComment + $newline + $ShimMember
  $patched = $source.Substring(0, $index) + $replacement + $source.Substring($index + $ShimMarker.Length)
  [IO.File]::WriteAllBytes($header, $utf8.GetBytes($patched))
  return 'applied'
}

# Relative path ('/' separators) -> "length|last write ticks" for every file.
function Get-TreeSnapshot {
  param([Parameter(Mandatory = $true)][string]$Root)
  $rootPath = [IO.Path]::GetFullPath($Root).TrimEnd('\')
  $map = [System.Collections.Generic.Dictionary[string, string]]::new([StringComparer]::OrdinalIgnoreCase)
  foreach ($file in [IO.Directory]::EnumerateFiles($rootPath, '*', [IO.SearchOption]::AllDirectories)) {
    $info = [IO.FileInfo]::new($file)
    $map[$file.Substring($rootPath.Length + 1).Replace('\', '/')] = "$($info.Length)|$($info.LastWriteTimeUtc.Ticks)"
  }
  return $map
}

function Compare-TreeSnapshot {
  param(
    [Parameter(Mandatory = $true)]$Before,
    [Parameter(Mandatory = $true)]$After
  )
  $added = [System.Collections.Generic.List[string]]::new()
  $changed = [System.Collections.Generic.List[string]]::new()
  $removed = [System.Collections.Generic.List[string]]::new()
  foreach ($key in $After.Keys) {
    if (-not $Before.ContainsKey($key)) { $added.Add($key) }
    elseif ($Before[$key] -cne $After[$key]) { $changed.Add($key) }
  }
  foreach ($key in $Before.Keys) {
    if (-not $After.ContainsKey($key)) { $removed.Add($key) }
  }
  $added.Sort([StringComparer]::Ordinal)
  $changed.Sort([StringComparer]::Ordinal)
  $removed.Sort([StringComparer]::Ordinal)
  return [pscustomobject]@{ Added = @($added); Changed = @($changed); Removed = @($removed) }
}

# Copies the files a runtime archive would hold: arduino-cli.exe, the indexes
# and data/packages. Nothing else from the staging folder.
function Copy-RuntimeContent {
  param(
    [Parameter(Mandatory = $true)][string]$From,
    [Parameter(Mandatory = $true)][string]$To
  )
  New-Item -ItemType Directory -Force -Path (Join-Path $To 'data') | Out-Null
  Copy-Item -LiteralPath (Join-Path $From 'arduino-cli.exe') -Destination $To
  foreach ($name in $DataRootFiles) {
    $source = Join-Path $From "data\$name"
    if (Test-Path -LiteralPath $source -PathType Leaf) {
      Copy-Item -LiteralPath $source -Destination (Join-Path $To 'data')
    }
  }
  & robocopy.exe (Join-Path $From 'data\packages') (Join-Path $To 'data\packages') /E /COPY:DAT /DCOPY:DAT /R:0 /W:0 /NFL /NDL /NJH /NJS /NP | Out-Null
  if ($LASTEXITCODE -ge 8) {
    throw "robocopy of data\packages failed with exit code $LASTEXITCODE."
  }
}

# A simulated Windows profile, laid out like the application's folders:
#   <Base>\<user>\AppData\Local\WVU-BMEG\rt                 extracted runtime
#   <Base>\<user>\AppData\Local\Temp                        TEMP and TMP
#   <Base>\<user>\AppData\Local\WVU Bioinstrumentation Studio\firmware\logs\jobs\job_1
#                                                           sketch and output folders
# The <user> folder name is sized so that the runtime root has exactly
# -ProofRootLength characters, the length of the real root for a profile
# folder name of that size.
function New-ProofProfile {
  param([Parameter(Mandatory = $true)][string]$Base)
  New-EmptyDirectory $Base
  $padding = $ProofRootLength - '\AppData\Local\WVU-BMEG\rt'.Length - $Base.Length - 1
  if ($padding -lt 1) {
    throw "-WorkRoot is too long for a $ProofRootLength-character runtime root under $Base. Use a shorter -WorkRoot."
  }
  $userProfile = Join-Path $Base ('student' + ('x' * [Math]::Max(0, $padding - 7))).Substring(0, $padding)
  $local = Join-Path $userProfile 'AppData\Local'
  $job = Join-Path $local 'WVU Bioinstrumentation Studio\firmware\logs\jobs\job_1'
  $proofProfile = [pscustomobject]@{
    Base = $Base
    UserProfile = $userProfile
    Roaming = Join-Path $userProfile 'AppData\Roaming'
    Local = $local
    Temp = Join-Path $local 'Temp'
    RuntimeRoot = Join-Path $local 'WVU-BMEG\rt'
    SketchDir = Join-Path $job 'reference_unor4wifi'
    OutputDir = Join-Path $job 'compile'
  }
  foreach ($folder in @($proofProfile.Roaming, $proofProfile.Temp, $proofProfile.SketchDir)) {
    New-Item -ItemType Directory -Force -Path $folder | Out-Null
  }
  if ($proofProfile.RuntimeRoot.Length -ne $ProofRootLength) { throw "Internal error: proof root length $($proofProfile.RuntimeRoot.Length)." }
  return $proofProfile
}

# Prepares the runtime in $ProofProfile.RuntimeRoot the way the application
# does after extraction, then compiles the reference sketch and queries the
# board with all network access sent to a dead proxy. Returns the evidence and
# a list of problems (empty when the proof passed).
function Invoke-OfflineProof {
  param(
    [Parameter(Mandatory = $true)][string]$Label,
    [Parameter(Mandatory = $true)]$ProofProfile
  )
  $root = $ProofProfile.RuntimeRoot
  $problems = [System.Collections.Generic.List[string]]::new()
  $cli = Join-Path $root 'arduino-cli.exe'

  # What the application does after extracting the archive.
  $shim = Add-NativeUsbShim $root
  Write-Host "SerialUSB.h shim: $shim"
  New-Item -ItemType Directory -Force -Path (Join-Path $root 'downloads'), (Join-Path $root 'sketchbook') | Out-Null
  $config = Join-Path $root 'arduino-cli.yaml'
  [IO.File]::WriteAllText($config, (Get-AppCliConfigText $root), [Text.UTF8Encoding]::new($false))

  # The application writes the embedded source into a fresh job folder.
  $sketchFile = Join-Path $ProofProfile.SketchDir 'reference_unor4wifi.ino'
  [IO.File]::WriteAllBytes($sketchFile, [IO.File]::ReadAllBytes($ReferenceSketch))
  if ((Get-Sha256 $sketchFile) -cne $script:ReferenceSketchSha256) { throw 'The copied reference sketch differs from its source.' }

  $before = Get-TreeSnapshot $root
  $environment = @{
    HTTP_PROXY = $DeadProxy
    HTTPS_PROXY = $DeadProxy
    ALL_PROXY = $DeadProxy
    NO_PROXY = $null
    ARDUINO_NETWORK_PROXY = $DeadProxy
    TEMP = $ProofProfile.Temp
    TMP = $ProofProfile.Temp
    LOCALAPPDATA = $ProofProfile.Local
    APPDATA = $ProofProfile.Roaming
    USERPROFILE = $ProofProfile.UserProfile
  }
  $common = @{
    Executable = $cli
    ConfigFile = $config
    Environment = $environment
    WorkingDirectory = $ProofProfile.Temp
  }

  # The application's own queries: `version` and `core list --format json`.
  $version = Invoke-ArduinoCli @common -Label "$Label-version" -Arguments @('version') -AllowFailure
  if ($version.ExitCode -ne 0 -or $version.StdOut -notmatch "Version:\s*$([regex]::Escape($CliVersion))\b") {
    $problems.Add("arduino-cli version did not report $CliVersion.")
  }
  $coreList = Invoke-ArduinoCli @common -Label "$Label-core-list" -Arguments @('core', 'list', '--format', 'json') -AllowFailure -Quiet
  $installedCore = $null
  if ($coreList.ExitCode -eq 0) {
    $platform = @(($coreList.StdOut | ConvertFrom-Json).platforms) | Where-Object id -EQ "${CorePackager}:$CoreArchitecture" | Select-Object -First 1
    $installedCore = $platform.installed_version
  }
  Write-Host "  core list: ${CorePackager}:$CoreArchitecture installed_version=$installedCore"
  if ($installedCore -cne $CoreVersion) { $problems.Add("core list did not report ${CorePackager}:$CoreArchitecture $CoreVersion.") }

  # The application's compile command (ArduinoCli::compile_to).
  $compile = Invoke-ArduinoCli @common -Label "$Label-compile" -AllowFailure -Arguments @(
    'compile', '--fqbn', $script:Fqbn,
    '--build-property', "build.defines=$($script:BuildDefines)",
    '--output-dir', $ProofProfile.OutputDir,
    $ProofProfile.SketchDir
  )
  $usage = [ordered]@{}
  $flash = [regex]::Match($compile.StdOut, 'Sketch uses (\d+) bytes \((\d+)%\) of program storage space\. Maximum is (\d+) bytes\.')
  $ram = [regex]::Match($compile.StdOut, 'Global variables use (\d+) bytes \((\d+)%\) of dynamic memory, leaving (\d+) bytes for local variables\. Maximum is (\d+) bytes\.')
  if ($flash.Success) {
    $usage.FlashBytes = [long]$flash.Groups[1].Value; $usage.FlashPercent = [int]$flash.Groups[2].Value; $usage.FlashMax = [long]$flash.Groups[3].Value
  }
  if ($ram.Success) {
    $usage.RamBytes = [long]$ram.Groups[1].Value; $usage.RamPercent = [int]$ram.Groups[2].Value; $usage.RamFree = [long]$ram.Groups[3].Value; $usage.RamMax = [long]$ram.Groups[4].Value
  }
  $binary = Join-Path $ProofProfile.OutputDir 'reference_unor4wifi.ino.bin'
  $binaryInfo = $null
  if ($compile.ExitCode -ne 0) {
    $problems.Add("compile failed with exit code $($compile.ExitCode).")
  } elseif (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
    $problems.Add("compile did not write $binary.")
  } else {
    $binaryInfo = [pscustomobject]@{ Bytes = (Get-Item -LiteralPath $binary).Length; Sha256 = Get-Sha256 $binary }
  }
  if (-not $flash.Success -or -not $ram.Success) { $problems.Add('compile output has no flash/RAM usage lines.') }

  # Upload tool resolution, without uploading anything.
  $details = Invoke-ArduinoCli @common -Label "$Label-board-details" -Quiet -AllowFailure -Arguments @(
    'board', 'details', '-b', $script:Fqbn, '--show-properties'
  )
  $properties = @{}
  foreach ($line in ($details.StdOut -split "`r?`n")) {
    $separator = $line.IndexOf('=')
    if ($separator -gt 0) { $properties[$line.Substring(0, $separator)] = $line.Substring($separator + 1) }
  }
  $bossacPath = $properties['tools.bossac.path']
  $uploadTool = if ($properties.ContainsKey('upload.tool.default')) { $properties['upload.tool.default'] } else { $properties['upload.tool'] }
  $bossacOk = $false
  if ($details.ExitCode -ne 0) {
    $problems.Add("board details failed with exit code $($details.ExitCode).")
  } elseif ([string]::IsNullOrWhiteSpace($bossacPath)) {
    $problems.Add('board details has no tools.bossac.path.')
  } else {
    $bossacFull = [IO.Path]::GetFullPath($bossacPath)
    $bossacOk = $bossacFull.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase) -and
      (Test-Path -LiteralPath (Join-Path $bossacFull 'bossac.exe') -PathType Leaf)
    if (-not $bossacOk) { $problems.Add("tools.bossac.path does not resolve to bossac.exe inside the runtime: $bossacPath") }
  }
  if ($uploadTool -cne 'bossac') { $problems.Add("upload tool is '$uploadTool', expected bossac.") }
  Write-Host "  tools.bossac.path=$bossacPath (bossac.exe present: $bossacOk); upload tool: $uploadTool"

  # Nothing may have been downloaded or changed in the runtime.
  $difference = Compare-TreeSnapshot -Before $before -After (Get-TreeSnapshot $root)
  foreach ($path in @($difference.Added) + @($difference.Changed) + @($difference.Removed)) {
    if ($path -like 'data/packages/*' -or $path -like 'downloads/*' -or $path -like 'data/*index.json*' -or $path -ceq 'arduino-cli.exe') {
      $problems.Add("runtime file added, changed or removed during the offline proof: $path")
    }
  }
  $inventoryFile = Join-Path $root 'data\inventory.yaml'
  $inventoryKeys = @()
  if (Test-Path -LiteralPath $inventoryFile -PathType Leaf) {
    # Key names only (top.child); the values include a per-installation secret.
    $section = ''
    $inventoryKeys = @(foreach ($line in Get-Content -LiteralPath $inventoryFile) {
        if ($line -match '^([A-Za-z_]+):\s*$') { $section = $Matches[1] }
        elseif ($line -match '^([A-Za-z_]+):') { $section = ''; $Matches[1] }
        elseif ($line -match '^\s+([A-Za-z_]+):' -and $section) { "$section.$($Matches[1])" }
      })
  }
  $buildCache = Join-Path $ProofProfile.Local 'arduino'
  $buildCacheFolders = if (Test-Path -LiteralPath $buildCache) { @(Get-ChildItem -LiteralPath $buildCache -Directory | ForEach-Object Name) } else { @() }

  $warnings = @(@($compile, $details, $coreList, $version) | ForEach-Object { $_.StdErr } | Where-Object { $_ } | ForEach-Object { $_ -split "`n" } | Where-Object { $_.Trim() })
  $result = [pscustomobject]@{
    Label = $Label
    RuntimeRoot = $root
    Passed = ($problems.Count -eq 0)
    Problems = @($problems)
    VersionText = $version.StdOut.Trim()
    InstalledCore = $installedCore
    Usage = $usage
    Binary = $binaryInfo
    CompileSeconds = $compile.Seconds
    BossacPath = $bossacPath
    UploadTool = $uploadTool
    Added = $difference.Added
    Changed = $difference.Changed
    Removed = $difference.Removed
    InventoryKeys = $inventoryKeys
    BuildCacheFolders = $buildCacheFolders
    Warnings = $warnings
    ToolProperties = @($properties.Keys | Where-Object { $_ -match '^(runtime\.)?tools\.[^.]+\.path$' } | Sort-Object | ForEach-Object { "$_=$($properties[$_])" })
  }
  if ($result.Passed) {
    Write-Host ("Offline proof '{0}' passed: flash {1} bytes ({2}%), RAM {3} bytes ({4}%)." -f $Label, $usage.FlashBytes, $usage.FlashPercent, $usage.RamBytes, $usage.RamPercent) -ForegroundColor Green
  } else {
    Write-Host "Offline proof '$Label' FAILED:" -ForegroundColor Red
    $problems | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
  }
  return $result
}

# Ordinal-sorted entries, '/' separators, one timestamp, no wrapper folder.
# Files imply their folders; only empty folders get their own entry.
function New-DeterministicZip {
  param(
    [Parameter(Mandatory = $true)][string]$SourceRoot,
    [Parameter(Mandatory = $true)][string]$Destination
  )
  $root = [IO.Path]::GetFullPath($SourceRoot).TrimEnd('\')
  $sources = [System.Collections.Generic.Dictionary[string, string]]::new([StringComparer]::Ordinal)
  foreach ($item in Get-ChildItem -LiteralPath $root -Recurse -Force) {
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Links are not allowed in the runtime: $($item.FullName)" }
    $relative = $item.FullName.Substring($root.Length + 1).Replace('\', '/')
    if ($item.PSIsContainer) {
      if (@(Get-ChildItem -LiteralPath $item.FullName -Force).Count -eq 0) { $sources["$relative/"] = '' }
    } else {
      $sources[$relative] = $item.FullName
    }
  }
  $names = [System.Collections.Generic.List[string]]::new($sources.Keys)
  $names.Sort([StringComparer]::Ordinal)
  $stream = [IO.File]::Open($Destination, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
  try {
    $zip = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create, $false, [Text.UTF8Encoding]::new($false))
    try {
      foreach ($name in $names) {
        $entry = $zip.CreateEntry($name, [IO.Compression.CompressionLevel]::Optimal)
        $entry.LastWriteTime = $ZipTimestamp
        # Names ending in '/' are empty folders: an entry without content.
        if (-not $name.EndsWith('/')) {
          $reader = [IO.File]::OpenRead($sources[$name])
          try {
            $writer = $entry.Open()
            try { $reader.CopyTo($writer) } finally { $writer.Dispose() }
          } finally {
            $reader.Dispose()
          }
        }
      }
    } finally {
      $zip.Dispose()
    }
  } finally {
    $stream.Dispose()
  }
  return $names.Count
}

# Format checks on the finished archive. Returns a list of problems.
function Test-RuntimeArchive {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][string[]]$RequiredEntries
  )
  $problems = [System.Collections.Generic.List[string]]::new()
  $archive = [IO.Compression.ZipFile]::OpenRead($Path)
  try {
    $names = @($archive.Entries | ForEach-Object FullName)
    $sorted = [System.Collections.Generic.List[string]]::new([string[]]$names)
    $sorted.Sort([StringComparer]::Ordinal)
    if (($names -join "`n") -cne ($sorted -join "`n")) { $problems.Add('entries are not in ordinal order.') }
    $seen = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($entry in $archive.Entries) {
      $name = $entry.FullName
      if (-not $seen.Add($name)) { $problems.Add("duplicate entry (ignoring case): $name") }
      if ($name.Contains('\') -or $name.StartsWith('/') -or $name.Contains(':') -or ($name -split '/') -contains '..') {
        $problems.Add("unsafe or non-'/' entry name: $name")
      }
      if ($entry.LastWriteTime.DateTime -ne $ZipTimestamp.DateTime) { $problems.Add("entry timestamp is not fixed: $name") }
      if ($name -ceq 'arduino-cli.yaml' -or $name -ceq 'data/inventory.yaml' -or $name -match '^(downloads|sketchbook|user|cache|build-cache)/' -or $name -match '(^|/)(arduino-cli\.yaml|inventory\.yaml)$') {
        $problems.Add("entry must not be shipped: $name")
      }
    }
    foreach ($required in $RequiredEntries) {
      if ($names -cnotcontains $required) { $problems.Add("required entry missing: $required") }
    }
    $topLevel = @($names | ForEach-Object { ($_ -split '/')[0] } | Sort-Object -Unique)
    if ($topLevel.Count -eq 1) { $problems.Add("archive has a single wrapper folder: $($topLevel[0])") }
  } finally {
    $archive.Dispose()
  }
  return @($problems)
}

# Differences between the archive and release/arduino-runtime-entries.json:
# missing required entries, license files with another SHA-256, tool or
# platform folders that differ from the listed ones, and forbidden entries.
function Compare-ArchiveWithEntries {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)]$Entries
  )
  $problems = [System.Collections.Generic.List[string]]::new()
  $archive = [IO.Compression.ZipFile]::OpenRead($Path)
  try {
    $names = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($entry in $archive.Entries) { [void]$names.Add($entry.FullName) }
    foreach ($required in @($Entries.required_entries)) {
      if (-not $names.Contains($required)) { $problems.Add("required entry missing: $required") }
    }
    foreach ($license in @($Entries.license_entries)) {
      $actual = Get-ArchiveEntrySha256 -Path $Path -Entry $license.path
      if ($null -eq $actual) { $problems.Add("license file missing: $($license.path)") }
      elseif ($actual -cne $license.sha256) { $problems.Add("license file $($license.path) has SHA-256 $actual, the entries file pins $($license.sha256)") }
    }
    $expectedFolders = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($tool in @($Entries.tools)) { [void]$expectedFolders.Add("data/packages/$($tool.package)/tools/$($tool.name)/$($tool.version)/") }
    foreach ($platform in @($Entries.platforms)) { [void]$expectedFolders.Add("data/packages/$($platform.package)/hardware/$($platform.architecture)/$($platform.version)/") }
    $actualFolders = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($name in $names) {
      $match = [regex]::Match($name, '^data/packages/[^/]+/(tools|hardware)/[^/]+/[^/]+/')
      if ($match.Success) { [void]$actualFolders.Add($match.Value) }
    }
    foreach ($folder in $actualFolders) { if (-not $expectedFolders.Contains($folder)) { $problems.Add("tool or platform folder not in the entries file: $folder") } }
    foreach ($folder in $expectedFolders) { if (-not $actualFolders.Contains($folder)) { $problems.Add("tool or platform folder missing: $folder") } }
    foreach ($prefix in @($Entries.forbidden_prefixes)) {
      foreach ($name in $names) {
        if ($name.StartsWith($prefix, [StringComparison]::Ordinal)) { $problems.Add("forbidden entry: $name"); break }
      }
    }
  } finally {
    $archive.Dispose()
  }
  return @($problems)
}

function Get-ArchiveEntrySha256 {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][string]$Entry
  )
  $archive = [IO.Compression.ZipFile]::OpenRead($Path)
  try {
    $item = $archive.GetEntry($Entry)
    if ($null -eq $item) { return $null }
    $stream = $item.Open()
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
      return ([Convert]::ToHexString($sha.ComputeHash($stream))).ToLowerInvariant()
    } finally {
      $sha.Dispose()
      $stream.Dispose()
    }
  } finally {
    $archive.Dispose()
  }
}

# --- Parameters and safety checks -------------------------------------------------

$scriptWatch = [Diagnostics.Stopwatch]::StartNew()
$previousOutputEncoding = [Console]::OutputEncoding
# Arduino CLI writes UTF-8.
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
try {

if ([string]::IsNullOrWhiteSpace($ProjectRoot)) { $ProjectRoot = Join-Path $PSScriptRoot '..' }
$ProjectRoot = (Resolve-Path -LiteralPath (Resolve-FullPath $ProjectRoot)).Path
if ([string]::IsNullOrWhiteSpace($ReferenceSketch)) {
  $ReferenceSketch = Join-Path $ProjectRoot 'firmware\reference_unor4wifi\reference_unor4wifi.ino'
}
$ReferenceSketch = (Resolve-Path -LiteralPath (Resolve-FullPath $ReferenceSketch)).Path
$script:ReferenceSketchSha256 = Get-Sha256 $ReferenceSketch

$WorkRoot = Resolve-FullPath $WorkRoot
if ([IO.Path]::GetPathRoot($WorkRoot).TrimEnd('\') -eq $WorkRoot) { throw "-WorkRoot must not be a drive root: $WorkRoot" }
# Never build inside the repository or on top of a real runtime or Arduino installation.
$localAppData = [Environment]::GetFolderPath('LocalApplicationData')
foreach ($protected in @($ProjectRoot, (Join-Path $localAppData 'WVU-BMEG'), (Join-Path $localAppData 'WVU Bioinstrumentation Studio'), (Join-Path $localAppData 'Arduino15'), $env:USERPROFILE, $env:ProgramFiles, $env:windir)) {
  if ([string]::IsNullOrWhiteSpace($protected)) { continue }
  $protectedFull = [IO.Path]::GetFullPath($protected).TrimEnd('\')
  if ($WorkRoot -ieq $protectedFull -or $WorkRoot.StartsWith("$protectedFull\", [StringComparison]::OrdinalIgnoreCase) -or $protectedFull.StartsWith("$WorkRoot\", [StringComparison]::OrdinalIgnoreCase)) {
    throw "-WorkRoot $WorkRoot overlaps $protectedFull. Use a separate short folder such as C:\wvu-rtbuild."
  }
}
# The proofs need <WorkRoot>\pN\<user>\AppData\Local\WVU-BMEG\rt, with a
# user folder of at least one character, to fit in -ProofRootLength.
if ($WorkRoot.Length + '\p1\u\AppData\Local\WVU-BMEG\rt'.Length -gt $ProofRootLength) {
  throw "-WorkRoot is $($WorkRoot.Length) characters long; the offline proofs need a $ProofRootLength-character runtime root under it. Use a shorter folder such as C:\wvu-rtbuild."
}

if ([string]::IsNullOrWhiteSpace($OutputZip)) { $OutputZip = Join-Path $WorkRoot 'output\arduino-runtime.zip' }
$OutputZip = Resolve-FullPath $OutputZip
$outputFolder = Split-Path -Parent $OutputZip
if ($DownloadCache) {
  $DownloadCache = Resolve-FullPath $DownloadCache
  # -Force empties -WorkRoot; the cache must survive that.
  if ($DownloadCache -ieq $WorkRoot -or $DownloadCache.StartsWith("$WorkRoot\", [StringComparison]::OrdinalIgnoreCase) -or $WorkRoot.StartsWith("$DownloadCache\", [StringComparison]::OrdinalIgnoreCase)) {
    throw "-DownloadCache $DownloadCache must be outside -WorkRoot $WorkRoot."
  }
}
if ($OutputZip.StartsWith("$WorkRoot\", [StringComparison]::OrdinalIgnoreCase) -and
    -not $OutputZip.StartsWith((Join-Path $WorkRoot 'output') + '\', [StringComparison]::OrdinalIgnoreCase)) {
  throw '-OutputZip inside -WorkRoot must be under <WorkRoot>\output.'
}
if ($OutputZip.StartsWith("$ProjectRoot\", [StringComparison]::OrdinalIgnoreCase)) {
  throw '-OutputZip must be outside the repository. Copy the reviewed archive into src-tauri\resources yourself.'
}
$checksumPath = "$OutputZip.sha256"
$inventoryPath = Join-Path $outputFolder "arduino-runtime-$BundleRevision.inventory.txt"

$markerPath = Join-Path $WorkRoot $WorkRootMarker
if ((Test-Path -LiteralPath $WorkRoot) -and @(Get-ChildItem -LiteralPath $WorkRoot -Force).Count -gt 0) {
  if (-not $Force) { throw "-WorkRoot $WorkRoot is not empty. Rerun with -Force to delete its contents and build again." }
  # Only a folder this script set up is ever emptied.
  if (-not (Test-Path -LiteralPath $markerPath -PathType Leaf)) {
    throw "-WorkRoot $WorkRoot is not empty and has no $WorkRootMarker file, so this script did not create it. -Force deletes only its own work folders; choose an empty or new folder."
  }
  Write-Host "Deleting the contents of $WorkRoot (-Force)."
  Get-ChildItem -LiteralPath $WorkRoot -Force | Where-Object Name -NE $WorkRootMarker | Remove-Item -Recurse -Force
}
foreach ($existing in @($OutputZip, $checksumPath, $inventoryPath)) {
  if (Test-Path -LiteralPath $existing) {
    if (-not $Force) { throw "$existing exists. Rerun with -Force to replace it." }
    Remove-Item -LiteralPath $existing -Force
  }
}
New-Item -ItemType Directory -Force -Path $WorkRoot, $outputFolder | Out-Null
[IO.File]::WriteAllText($markerPath, "Work folder of scripts/build_arduino_runtime.ps1 (WVU Bioinstrumentation Studio). Safe to delete.`n", [Text.UTF8Encoding]::new($false))

# What the reviewed archive must contain (scripts/build_student_release.ps1
# and scripts/audit_bundled_runtime_notices.ps1 read the same file).
$entriesPath = Join-Path $ProjectRoot 'release\arduino-runtime-entries.json'
$script:Entries = $null
if (Test-Path -LiteralPath $entriesPath -PathType Leaf) {
  $script:Entries = Get-Content -LiteralPath $entriesPath -Raw | ConvertFrom-Json
}

# The application's board and compile arguments, read from its source.
$cliSourcePath = Join-Path $ProjectRoot 'src-tauri\src\arduino_cli.rs'
$runtimeSourcePath = Join-Path $ProjectRoot 'src-tauri\src\arduino_runtime.rs'
foreach ($required in @($cliSourcePath, $runtimeSourcePath)) {
  if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "Application source not found: $required. Pass -ProjectRoot." }
}
$cliSource = [IO.File]::ReadAllText($cliSourcePath)
$script:BuildDefines = [regex]::Match($cliSource, 'pub const NATIVE_USB_REFERENCE_BUILD_DEFINES:\s*&str\s*=\s*"([^"]+)"').Groups[1].Value
$script:Fqbn = [regex]::Match($cliSource, 'pub const UNO_R4_WIFI_FQBN:\s*&str\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $script:BuildDefines -or -not $script:Fqbn) { throw "Could not read UNO_R4_WIFI_FQBN and NATIVE_USB_REFERENCE_BUILD_DEFINES from $cliSourcePath." }
$runtimeSource = [IO.File]::ReadAllText($runtimeSourcePath)
foreach ($fragment in @($ShimMarker, $ShimSignature, $ShimComment.Trim(), 'enable_notification: false', 'skip_board_detection_calls: true')) {
  if (-not $runtimeSource.Contains($fragment)) { throw "src-tauri/src/arduino_runtime.rs no longer contains '$fragment'. Update this script's shim and arduino-cli.yaml text to match the application." }
}

$paths = [ordered]@{
  Downloads = Join-Path $WorkRoot 'dl'
  CliExtract = Join-Path $WorkRoot 'cli'
  Stage = Join-Path $WorkRoot 'stage'
  Config = Join-Path $WorkRoot 'config\build-config.yaml'
  ArduinoDownloads = if ($DownloadCache) { $DownloadCache } else { Join-Path $WorkRoot 'arduino-downloads' }
  User = Join-Path $WorkRoot 'user'
  BuildCache = Join-Path $WorkRoot 'build-cache'
  Temp = Join-Path $WorkRoot 'tmp'
  LocalAppData = Join-Path $WorkRoot 'localappdata'
  RemovedTools = Join-Path $WorkRoot 'removed-tools'
  Logs = Join-Path $WorkRoot 'logs'
}
foreach ($folder in @($paths.Downloads, $paths.CliExtract, $paths.Stage, (Split-Path -Parent $paths.Config), $paths.ArduinoDownloads, $paths.User, $paths.BuildCache, $paths.Temp, $paths.LocalAppData, $paths.Logs)) {
  New-Item -ItemType Directory -Force -Path $folder | Out-Null
}
$script:LogRoot = $paths.Logs

Write-Host 'WVU Bioinstrumentation Studio - Arduino runtime build'
Write-Host "  Arduino CLI $CliVersion, ${CorePackager}:$CoreArchitecture@$CoreVersion, bundle revision $BundleRevision"
Write-Host "  Work root: $WorkRoot"
Write-Host "  Output: $OutputZip"
Write-Host "  Reference sketch: $ReferenceSketch (SHA-256 $($script:ReferenceSketchSha256))"
Write-Host "  Compile: --fqbn $($script:Fqbn) --build-property `"build.defines=$($script:BuildDefines)`""

# --- 1. Arduino CLI --------------------------------------------------------------

Write-Step "1. Download and verify Arduino CLI $CliVersion"
$checksumsFile = Save-Download -Url $CliChecksumsUrl -Folder (Join-Path $paths.Downloads 'cli-checksums') -FileName "$CliVersion-checksums.txt"
$publishedLine = @(Get-Content -LiteralPath $checksumsFile | Where-Object { $_ -match "^([0-9a-fA-F]{64})\s+\*?$([regex]::Escape($CliArchiveName))$" })
if ($publishedLine.Count -ne 1) { throw "$CliVersion-checksums.txt does not list $CliArchiveName exactly once." }
$publishedSha256 = ($publishedLine[0] -split '\s+')[0].ToLowerInvariant()
if ($publishedSha256 -cne $CliArchiveSha256) {
  throw "The published checksum $publishedSha256 for $CliArchiveName differs from the pinned $CliArchiveSha256."
}
Write-Host "Published checksum matches the pinned value: $publishedSha256"
$cliArchive = Save-Download -Url $CliArchiveUrl -Folder (Join-Path $paths.Downloads 'cli') -FileName $CliArchiveName
Assert-Sha256 -Path $cliArchive -Expected $CliArchiveSha256 -Description $CliArchiveName

$cliEntries = @()
$cliZip = [IO.Compression.ZipFile]::OpenRead($cliArchive)
try { $cliEntries = @($cliZip.Entries | ForEach-Object FullName) } finally { $cliZip.Dispose() }
Write-Host "Archive entries: $($cliEntries -join ', ')"
foreach ($required in @('arduino-cli.exe', 'LICENSE.txt')) {
  if ($cliEntries -cnotcontains $required) { throw "$CliArchiveName has no $required at its root." }
}
Expand-Archive -LiteralPath $cliArchive -DestinationPath $paths.CliExtract
$extractedCli = Join-Path $paths.CliExtract 'arduino-cli.exe'
$cliLicense = Join-Path $paths.CliExtract 'LICENSE.txt'
$cli = Join-Path $paths.Stage 'arduino-cli.exe'
Copy-Item -LiteralPath $extractedCli -Destination $cli
Copy-Item -LiteralPath $cliLicense -Destination (Join-Path $paths.Stage 'arduino-cli-LICENSE.txt')
$cliExeSha256 = Get-Sha256 $cli
if ($cliExeSha256 -cne (Get-Sha256 $extractedCli)) { throw 'The staged arduino-cli.exe differs from the verified archive.' }
$cliLicenseSha256 = Get-Sha256 $cliLicense
$signature = Get-AuthenticodeSignature -LiteralPath $cli
$signatureText = "$($signature.Status)$(if ($signature.SignerCertificate) { ', ' + $signature.SignerCertificate.Subject })"
Write-Host "arduino-cli.exe SHA-256 $cliExeSha256; Authenticode: $signatureText"
Write-Host "LICENSE.txt SHA-256 $cliLicenseSha256 (saved as arduino-cli-LICENSE.txt)"

# --- 2. Build-only configuration ---------------------------------------------------

Write-Step '2. Write the build-only Arduino CLI configuration'
$buildConfig = @"
directories:
  data: $(ConvertTo-YamlPath (Join-Path $paths.Stage 'data'))
  downloads: $(ConvertTo-YamlPath $paths.ArduinoDownloads)
  user: $(ConvertTo-YamlPath $paths.User)
build_cache:
  path: $(ConvertTo-YamlPath $paths.BuildCache)
updater:
  enable_notification: false
network:
  cloud_api:
    skip_board_detection_calls: true

"@
[IO.File]::WriteAllText($paths.Config, $buildConfig.Replace("`r`n", "`n"), [Text.UTF8Encoding]::new($false))
Write-Host $buildConfig
$buildEnvironment = @{
  TEMP = $paths.Temp
  TMP = $paths.Temp
  LOCALAPPDATA = $paths.LocalAppData
}
$build = @{
  Executable = $cli
  ConfigFile = $paths.Config
  Environment = $buildEnvironment
  WorkingDirectory = $paths.Temp
}
$versionJson = (Invoke-ArduinoCli @build -Label 'version' -Arguments @('version', '--format', 'json')).StdOut | ConvertFrom-Json
if ($versionJson.VersionString -cne $CliVersion) { throw "arduino-cli reports version $($versionJson.VersionString), expected $CliVersion." }
# ConvertFrom-Json turns the ISO date into a DateTime; print it as ISO again.
$cliBuildDate = if ($versionJson.Date -is [DateTime]) { $versionJson.Date.ToUniversalTime().ToString("yyyy-MM-dd'T'HH:mm:ss'Z'") } else { [string]$versionJson.Date }
foreach ($key in @('directories.data', 'build_cache.path', 'updater.enable_notification', 'network.cloud_api.skip_board_detection_calls')) {
  Invoke-ArduinoCli @build -Label "config-get-$key" -Arguments @('config', 'get', $key) | Out-Null
}

# --- 3. Core and indexes -----------------------------------------------------------

Write-Step "3. Install ${CorePackager}:$CoreArchitecture@$CoreVersion and the indexes"
Invoke-ArduinoCli @build -Label 'core-update-index' -Arguments @('core', 'update-index') | Out-Null
Invoke-ArduinoCli @build -Label 'core-install' -Arguments @('core', 'install', "${CorePackager}:$CoreArchitecture@$CoreVersion", '--skip-post-install') | Out-Null
Invoke-ArduinoCli @build -Label 'lib-update-index' -Arguments @('lib', 'update-index') | Out-Null
$coreListJson = (Invoke-ArduinoCli @build -Label 'core-list' -Quiet -Arguments @('core', 'list', '--format', 'json')).StdOut | ConvertFrom-Json
$corePlatform = @($coreListJson.platforms) | Where-Object id -EQ "${CorePackager}:$CoreArchitecture" | Select-Object -First 1
if ($corePlatform.installed_version -cne $CoreVersion) { throw "core list reports ${CorePackager}:$CoreArchitecture $($corePlatform.installed_version), expected $CoreVersion." }
Write-Host "core list: $($corePlatform.id) installed_version=$($corePlatform.installed_version) (latest_version=$($corePlatform.latest_version))"

$stageData = Join-Path $paths.Stage 'data'
function Get-InstalledTools {
  param([Parameter(Mandatory = $true)][string]$DataRoot)
  $packagesRoot = Join-Path $DataRoot 'packages'
  foreach ($package in Get-ChildItem -LiteralPath $packagesRoot -Directory | Sort-Object Name) {
    $toolsRoot = Join-Path $package.FullName 'tools'
    if (-not (Test-Path -LiteralPath $toolsRoot)) { continue }
    foreach ($tool in Get-ChildItem -LiteralPath $toolsRoot -Directory | Sort-Object Name) {
      foreach ($release in Get-ChildItem -LiteralPath $tool.FullName -Directory | Sort-Object Name) {
        $files = @(Get-ChildItem -LiteralPath $release.FullName -Recurse -File -Force)
        [pscustomobject]@{
          Package = $package.Name
          Name = $tool.Name
          Version = $release.Name
          Files = $files.Count
          Bytes = ($files | Measure-Object -Property Length -Sum).Sum
          Path = "data/packages/$($package.Name)/tools/$($tool.Name)/$($release.Name)"
        }
      }
    }
  }
}
$installedTools = @(Get-InstalledTools $stageData)
Write-Host 'Installed tools:'
$installedTools | ForEach-Object { Write-Host ("  {0}:{1} {2} ({3} files, {4:N0} bytes)" -f $_.Package, $_.Name, $_.Version, $_.Files, $_.Bytes) }
$builtinTools = @($installedTools | Where-Object Package -EQ 'builtin')
if ($builtinTools.Count -eq 0) { throw "Arduino CLI did not install its built-in tools under data/packages/builtin/tools." }
# Exactly the reviewed tools at the reviewed versions (see $PinnedTools).
$toolDrift = [System.Collections.Generic.List[string]]::new()
foreach ($pin in $PinnedTools) {
  $found = @($installedTools | Where-Object { $_.Package -ceq $pin.Package -and $_.Name -ceq $pin.Name })
  $versions = @($found | ForEach-Object Version)
  if ($versions.Count -ne 1 -or $versions[0] -cne $pin.Version) {
    $toolDrift.Add("$($pin.Package):$($pin.Name) is pinned to $($pin.Version), but Arduino CLI installed $(if ($versions.Count) { $versions -join ', ' } else { 'nothing' }).")
  }
}
foreach ($tool in $installedTools) {
  if (-not @($PinnedTools | Where-Object { $_.Package -ceq $tool.Package -and $_.Name -ceq $tool.Name })) {
    $toolDrift.Add("Arduino CLI installed $($tool.Package):$($tool.Name) $($tool.Version), which this script does not pin.")
  }
}
if ($toolDrift.Count -gt 0) {
  throw ("The installed tools differ from the reviewed versions:`n - $($toolDrift -join "`n - ")`n" +
    'Arduino CLI installs the newest built-in tools that the current package index lists. A different version needs a license and source review ' +
    '(release/third-party-sources.json, src-tauri/resources/licenses/BUNDLED_COMPONENTS.txt, release/arduino-runtime-entries.json) before $PinnedTools in this script is updated.')
}
Write-Host "Installed tools match the $($PinnedTools.Count) pinned versions."
$upstreamHeader = Get-SerialUsbHeaderPath $paths.Stage
$markerCount = ([regex]::Matches([IO.File]::ReadAllText($upstreamHeader), [regex]::Escape($ShimMarker))).Count
if ($markerCount -lt 1) { throw "The staged SerialUSB.h has no line for the application's shim: $upstreamHeader" }
Write-Host "Upstream SerialUSB.h contains the shim marker $markerCount time(s); the archive keeps it unpatched."

# --- 4. Offline proof with every tool ---------------------------------------------

Write-Step '4. Offline proof: staged runtime with every installed tool'
$proof1Profile = New-ProofProfile (Join-Path $WorkRoot 'p1')
Copy-RuntimeContent -From $paths.Stage -To $proof1Profile.RuntimeRoot
$proof1 = Invoke-OfflineProof -Label 'p1-all-tools' -ProofProfile $proof1Profile
if (-not $proof1.Passed) { throw "The offline proof failed with every tool installed: $($proof1.Problems -join ' ')" }

# --- 5. Remove tools the UNO R4 WiFi never uses -------------------------------------

$removedTools = @()
$keptUnusedTools = @()
$proof2 = $null
if (-not $KeepUnusedTools) {
  Write-Step "5. Remove unused tools ($($UnusedTools -join ', ')) and repeat the offline proof"
  New-Item -ItemType Directory -Force -Path $paths.RemovedTools | Out-Null
  foreach ($name in $UnusedTools) {
    $toolFolder = Join-Path $stageData "packages\arduino\tools\$name"
    if (-not (Test-Path -LiteralPath $toolFolder)) { Write-Host "arduino:$name is not installed."; continue }
    foreach ($release in @($installedTools | Where-Object { $_.Package -eq 'arduino' -and $_.Name -eq $name })) { $removedTools += $release }
    Move-Item -LiteralPath $toolFolder -Destination (Join-Path $paths.RemovedTools $name)
    Write-Host "Removed data/packages/arduino/tools/$name"
  }
  $proof2Profile = New-ProofProfile (Join-Path $WorkRoot 'p2')
  Copy-RuntimeContent -From $paths.Stage -To $proof2Profile.RuntimeRoot
  $proof2 = Invoke-OfflineProof -Label 'p2-without-unused-tools' -ProofProfile $proof2Profile
  if (-not $proof2.Passed) {
    Write-Warning "The offline proof failed without $($UnusedTools -join ', '); putting them back."
    foreach ($name in $UnusedTools) {
      $moved = Join-Path $paths.RemovedTools $name
      if (Test-Path -LiteralPath $moved) { Move-Item -LiteralPath $moved -Destination (Join-Path $stageData "packages\arduino\tools\$name") }
    }
    $keptUnusedTools = $removedTools
    $removedTools = @()
  }
} else {
  Write-Step '5. Keep every installed tool (-KeepUnusedTools)'
  $keptUnusedTools = @($installedTools | Where-Object { $_.Package -eq 'arduino' -and $UnusedTools -contains $_.Name })
}

# --- 6. Strip and add the archive's own files --------------------------------------

Write-Step '6. Strip build-only files and add the manifest and license texts'
$strippedItems = [System.Collections.Generic.List[string]]::new()
foreach ($item in Get-ChildItem -LiteralPath $paths.Stage -Force) {
  if (@('arduino-cli.exe', 'arduino-cli-LICENSE.txt', 'data') -cnotcontains $item.Name) {
    $strippedItems.Add($item.Name)
    Remove-Item -LiteralPath $item.FullName -Recurse -Force
  }
}
foreach ($item in Get-ChildItem -LiteralPath $stageData -Force) {
  if ($item.Name -cne 'packages' -and $DataRootFiles -cnotcontains $item.Name) {
    $strippedItems.Add("data/$($item.Name)")
    Remove-Item -LiteralPath $item.FullName -Recurse -Force
  }
}
foreach ($leftover in @(Get-ChildItem -LiteralPath (Join-Path $stageData 'packages') -Recurse -File -Force | Where-Object { $_.Name -in @('arduino-cli.yaml', 'inventory.yaml') })) {
  $strippedItems.Add($leftover.FullName.Substring($paths.Stage.Length + 1).Replace('\', '/'))
  Remove-Item -LiteralPath $leftover.FullName -Force
}
Write-Host "Removed from the staged runtime: $(if ($strippedItems.Count) { $strippedItems -join ', ' } else { '(nothing)' })"
$logFiles = @(Get-ChildItem -LiteralPath $stageData -Recurse -File -Force -Filter '*.log')
if ($logFiles.Count -gt 0) { Write-Warning "Upstream *.log files kept for review: $($logFiles.FullName -join ', ')" }

$manifestText = "{`n  `"runtime_schema`": 1,`n  `"arduino_cli`": `"$CliVersion`",`n  `"renesas_uno_core`": `"$CoreVersion`",`n  `"bundle_revision`": $BundleRevision`n}`n"
[IO.File]::WriteAllText((Join-Path $paths.Stage 'runtime-manifest.json'), $manifestText, [Text.UTF8Encoding]::new($false))
foreach ($license in $LicenseTexts) {
  $downloaded = Save-Download -Url $license.Url -Folder (Join-Path $paths.Downloads ([IO.Path]::GetFileNameWithoutExtension($license.Name))) -FileName $license.Name
  Assert-Sha256 -Path $downloaded -Expected $license.Sha256 -Description $license.Name
  $text = [IO.File]::ReadAllText($downloaded)
  if (-not $text.Contains($license.Heading) -or -not $text.Contains($license.Version)) { throw "$($license.Name) is not the expected license text." }
  Copy-Item -LiteralPath $downloaded -Destination (Join-Path $paths.Stage $license.Name)
}
Write-Host $manifestText

# --- 7. Deterministic ZIP ----------------------------------------------------------

Write-Step '7. Write the deterministic ZIP, its checksum and the inventory'
$stageFiles = @(Get-ChildItem -LiteralPath $paths.Stage -Recurse -File -Force)
$stageBytes = ($stageFiles | Measure-Object -Property Length -Sum).Sum
$longestEntry = $stageFiles | ForEach-Object { $_.FullName.Substring($paths.Stage.Length + 1) } | Sort-Object Length -Descending | Select-Object -First 1
$entryCount = New-DeterministicZip -SourceRoot $paths.Stage -Destination $OutputZip
$zipSha256 = Get-Sha256 $OutputZip
$zipBytes = (Get-Item -LiteralPath $OutputZip).Length
[IO.File]::WriteAllText($checksumPath, "$zipSha256  $(Split-Path -Leaf $OutputZip)`n", [Text.UTF8Encoding]::new($false))
Write-Host "ZIP: $OutputZip"
Write-Host "  $entryCount entries ($($stageFiles.Count) files, $('{0:N0}' -f $stageBytes) bytes uncompressed), $('{0:N0}' -f $zipBytes) bytes"
Write-Host "  SHA-256 $zipSha256"
$determinism = 'not checked'
if ($CheckDeterminism) {
  $secondFolder = Join-Path $WorkRoot 'determinism'
  New-EmptyDirectory $secondFolder
  $second = Join-Path $secondFolder 'arduino-runtime.zip'
  New-DeterministicZip -SourceRoot $paths.Stage -Destination $second | Out-Null
  $secondSha256 = Get-Sha256 $second
  if ($secondSha256 -cne $zipSha256) { throw "A second ZIP from the same folder has SHA-256 $secondSha256, not $zipSha256." }
  Remove-Item -LiteralPath $secondFolder -Recurse -Force
  $determinism = "a second ZIP from the same folder had the same SHA-256 (PowerShell $($PSVersionTable.PSVersion), .NET $([Environment]::Version))"
  Write-Host "Determinism: $determinism"
}

# --- 8. Verify the finished archive -------------------------------------------------

Write-Step '8. Verify the finished archive offline'
$requiredEntries = @(
  'runtime-manifest.json', 'arduino-cli.exe', 'arduino-cli-LICENSE.txt', 'GPL-2.0.txt', 'LGPL-2.1.txt',
  'data/package_index.json', 'data/library_index.json',
  "data/packages/$CorePackager/hardware/$CoreArchitecture/$CoreVersion/platform.txt",
  "data/packages/$CorePackager/hardware/$CoreArchitecture/$CoreVersion/boards.txt",
  "data/packages/$CorePackager/hardware/$CoreArchitecture/$CoreVersion/LICENSE",
  "data/packages/$CorePackager/hardware/$CoreArchitecture/$CoreVersion/cores/arduino/usb/SerialUSB.h",
  "data/packages/arduino/tools/arm-none-eabi-gcc/$GccVersion/bin/arm-none-eabi-g++.exe",
  "data/packages/arduino/tools/arm-none-eabi-gcc/$GccVersion/share/doc/gcc-arm-none-eabi/license.txt",
  "data/packages/arduino/tools/bossac/$BossacVersion/bossac.exe"
)
foreach ($pin in @($PinnedTools | Where-Object Package -CEQ 'builtin')) {
  $requiredEntries += "data/packages/builtin/tools/$($pin.Name)/$($pin.Version)/$($pin.Name).exe"
}
$archiveProblems = @(Test-RuntimeArchive -Path $OutputZip -RequiredEntries $requiredEntries)
if ($archiveProblems.Count -gt 0) { throw "The archive failed its format checks:`n - $($archiveProblems -join "`n - ")" }
Write-Host 'Archive format checks passed: ordinal order, / separators, fixed timestamps, no wrapper folder, no per-machine files, required entries present.'

$verifyProfile = New-ProofProfile (Join-Path $WorkRoot 'v')
[IO.Compression.ZipFile]::ExtractToDirectory($OutputZip, $verifyProfile.RuntimeRoot)
$extractedManifest = Get-Content -LiteralPath (Join-Path $verifyProfile.RuntimeRoot 'runtime-manifest.json') -Raw | ConvertFrom-Json
if ($extractedManifest.runtime_schema -ne 1 -or $extractedManifest.arduino_cli -cne $CliVersion -or $extractedManifest.renesas_uno_core -cne $CoreVersion -or $extractedManifest.bundle_revision -ne $BundleRevision) {
  throw 'The extracted runtime-manifest.json does not hold the expected values.'
}
if ([IO.File]::ReadAllText((Get-SerialUsbHeaderPath $verifyProfile.RuntimeRoot)).Contains($ShimSignature)) { throw 'The archive holds a patched SerialUSB.h; it must ship the upstream file.' }
$proof3 = Invoke-OfflineProof -Label 'p3-final-archive' -ProofProfile $verifyProfile
if (-not $proof3.Passed) { throw "The offline proof failed for the finished archive: $($proof3.Problems -join ' ')" }

# What the release scripts require (release/arduino-runtime-entries.json):
# enforced for the revision that file describes, reported for another one.
$expectationLines = [System.Collections.Generic.List[string]]::new()
if ($null -eq $script:Entries) {
  $expectationLines.Add("  $entriesPath not found; not checked.")
} else {
  $entryProblems = @(Compare-ArchiveWithEntries -Path $OutputZip -Entries $script:Entries)
  $sameRevision = [int]$script:Entries.bundle_revision -eq $BundleRevision -and
    [string]$script:Entries.arduino_cli -ceq $CliVersion -and [string]$script:Entries.renesas_uno_core -ceq $CoreVersion
  if ($entryProblems.Count -eq 0) {
    $expectationLines.Add("  matches release/arduino-runtime-entries.json (revision $($script:Entries.bundle_revision)): required entries, license SHA-256 values, tool and platform folders, no forbidden entries")
  } elseif ($sameRevision) {
    throw "The archive does not match release/arduino-runtime-entries.json, which describes this revision:`n - $($entryProblems -join "`n - ")"
  } else {
    $expectationLines.Add("  release/arduino-runtime-entries.json describes revision $($script:Entries.bundle_revision) (Arduino CLI $($script:Entries.arduino_cli)); update it after review. Differences:")
    $entryProblems | ForEach-Object { $expectationLines.Add("    $_") }
  }
}
Write-Host 'Release entries check:'
$expectationLines | ForEach-Object { Write-Host $_ }

# --- Inventory -----------------------------------------------------------------------

$finalTools = @(Get-InstalledTools $stageData)
$noticePattern = '^(?i:license|copying|notice)(\.|$|_|-)|(?i)-license\.txt$|^(GPL-2\.0|LGPL-2\.1)\.txt$'
$noticeFiles = @($stageFiles | Where-Object { $_.Name -match $noticePattern } |
    ForEach-Object { [pscustomobject]@{ Path = $_.FullName.Substring($paths.Stage.Length + 1).Replace('\', '/'); Bytes = $_.Length; Sha256 = Get-Sha256 $_.FullName } } |
    Sort-Object Path)
$indexFiles = @($DataRootFiles | ForEach-Object { Join-Path $stageData $_ } | Where-Object { Test-Path -LiteralPath $_ } | ForEach-Object { Get-Item -LiteralPath $_ })

function Format-Proof {
  param($Proof)
  if ($null -eq $Proof) { return @('  (not run)') }
  $lines = @(
    "  $($Proof.Label): $(if ($Proof.Passed) { 'PASSED' } else { 'FAILED: ' + ($Proof.Problems -join ' ') })",
    "    runtime root: $($Proof.RuntimeRoot) ($($Proof.RuntimeRoot.Length) characters)",
    "    version: $($Proof.VersionText -replace '\s+', ' ')",
    "    core list installed_version: $($Proof.InstalledCore)",
    ('    flash: {0} bytes ({1}%) of {2}; RAM: {3} bytes ({4}%) of {5}, {6} bytes left for locals' -f $Proof.Usage.FlashBytes, $Proof.Usage.FlashPercent, $Proof.Usage.FlashMax, $Proof.Usage.RamBytes, $Proof.Usage.RamPercent, $Proof.Usage.RamMax, $Proof.Usage.RamFree),
    # The binary embeds source paths, so its SHA-256 differs between runtime folders.
    "    reference_unor4wifi.ino.bin: $($Proof.Binary.Bytes) bytes, SHA-256 $($Proof.Binary.Sha256); compile $($Proof.CompileSeconds) s",
    "    board details: tools.bossac.path=$($Proof.BossacPath); upload tool $($Proof.UploadTool)",
    "    files added in the runtime: $(if ($Proof.Added.Count) { $Proof.Added -join ', ' } else { '(none)' })",
    "    files changed in the runtime: $(if ($Proof.Changed.Count) { $Proof.Changed -join ', ' } else { '(none)' })",
    "    data/inventory.yaml keys written by Arduino CLI: $(if ($Proof.InventoryKeys.Count) { $Proof.InventoryKeys -join ', ' } else { '(file not written)' })",
    "    default build cache %LOCALAPPDATA%\arduino folders: $(if ($Proof.BuildCacheFolders.Count) { $Proof.BuildCacheFolders -join ', ' } else { '(none)' })",
    "    stderr: $(if ($Proof.Warnings.Count) { $Proof.Warnings -join ' | ' } else { '(empty)' })"
  )
  return $lines
}

$inventory = [System.Collections.Generic.List[string]]::new()
$inventory.Add('WVU Bioinstrumentation Studio - bundled Arduino runtime inventory')
$inventory.Add("Generated (UTC): $([DateTime]::UtcNow.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'")) by build_arduino_runtime.ps1")
$inventory.Add("PowerShell $($PSVersionTable.PSVersion), .NET $([Environment]::Version)")
$inventory.Add('')
$inventory.Add('Archive')
$inventory.Add("  file: $(Split-Path -Leaf $OutputZip)")
$inventory.Add("  SHA-256: $zipSha256")
$inventory.Add("  size: $zipBytes bytes")
$inventory.Add("  entries: $entryCount ($($stageFiles.Count) files, $($entryCount - $stageFiles.Count) empty-folder entries); $stageBytes bytes uncompressed")
$inventory.Add("  entry timestamp: $($ZipTimestamp.ToString('yyyy-MM-ddTHH:mm:ss')) for every entry; ordinal order; '/' separators; no wrapper folder")
$inventory.Add("  determinism: $determinism")
$inventory.Add("  longest entry path: $($longestEntry.Length) characters ($($longestEntry.Replace('\', '/')))")
$inventory.Add("    an extraction root of up to $(259 - 1 - $longestEntry.Length) characters keeps every path under MAX_PATH (260)")
$inventory.Add('')
$inventory.Add('runtime-manifest.json')
$manifestText.TrimEnd("`n").Split("`n") | ForEach-Object { $inventory.Add("  $_") }
$inventory.Add('')
$inventory.Add('Arduino CLI')
$inventory.Add("  version: $($versionJson.VersionString) (commit $($versionJson.Commit), date $cliBuildDate)")
$inventory.Add("  source: $CliArchiveUrl")
$inventory.Add("  source SHA-256: $CliArchiveSha256 (pinned; equals $CliVersion-checksums.txt from the GitHub release)")
$inventory.Add("  arduino-cli.exe SHA-256: $cliExeSha256; Authenticode: $signatureText")
$inventory.Add('')
$inventory.Add('Core')
$inventory.Add("  $($corePlatform.id) $($corePlatform.installed_version) (core install --skip-post-install)")
$inventory.Add("  SerialUSB.h shipped unpatched; shim marker found $markerCount time(s)")
$inventory.Add('')
$inventory.Add('Tools in the archive (package:name version, files, bytes)')
$finalTools | ForEach-Object { $inventory.Add(('  {0}:{1} {2}  {3} files  {4} bytes' -f $_.Package, $_.Name, $_.Version, $_.Files, $_.Bytes)) }
$inventory.Add('Tools removed (unused by the UNO R4 WiFi)')
if ($removedTools.Count) { $removedTools | ForEach-Object { $inventory.Add(('  {0}:{1} {2}  {3} files  {4} bytes - removed; offline proof p2 passed without it' -f $_.Package, $_.Name, $_.Version, $_.Files, $_.Bytes)) } } else { $inventory.Add('  (none)') }
if ($keptUnusedTools.Count) { $keptUnusedTools | ForEach-Object { $inventory.Add(('  kept: {0}:{1} {2}' -f $_.Package, $_.Name, $_.Version)) } }
$inventory.Add('')
$inventory.Add('Indexes (data/)')
$indexFiles | ForEach-Object { $inventory.Add(('  {0}  {1} bytes  SHA-256 {2}' -f $_.Name, $_.Length, (Get-Sha256 $_.FullName))) }
$inventory.Add("Removed before packaging: $(if ($strippedItems.Count) { $strippedItems -join ', ' } else { '(nothing)' })")
$inventory.Add('')
$inventory.Add("License and notice files ($($noticeFiles.Count); SHA-256  bytes  path)")
$noticeFiles | ForEach-Object { $inventory.Add(('  {0}  {1,7}  {2}' -f $_.Sha256, $_.Bytes, $_.Path)) }
$inventory.Add('')
$inventory.Add("Offline proofs (all HTTP(S) to $DeadProxy; reference_unor4wifi.ino SHA-256 $($script:ReferenceSketchSha256))")
$inventory.Add("  compile --fqbn $($script:Fqbn) --build-property `"build.defines=$($script:BuildDefines)`" --output-dir <job>\compile <job>\reference_unor4wifi")
foreach ($proof in @($proof1, $proof2, $proof3)) { Format-Proof $proof | ForEach-Object { $inventory.Add($_) } }
$inventory.Add('')
$inventory.Add('Release entries check (release/arduino-runtime-entries.json)')
$expectationLines | ForEach-Object { $inventory.Add($_) }
$inventory.Add('')
$inventory.Add('Arduino CLI commands run')
$script:CommandLog | ForEach-Object { $inventory.Add("  $_") }
[IO.File]::WriteAllText($inventoryPath, (($inventory -join "`n") + "`n"), [Text.UTF8Encoding]::new($false))

Write-Step 'Done'
Write-Host "Archive:   $OutputZip"
Write-Host "SHA-256:   $zipSha256 ($checksumPath)"
Write-Host "Inventory: $inventoryPath"
Write-Host "Logs:      $($paths.Logs)"
Write-Host ("Elapsed:   {0:N0} s" -f $scriptWatch.Elapsed.TotalSeconds)
Write-Host 'Review the inventory and notices before copying the archive to src-tauri\resources\arduino-runtime.zip.'

} finally {
  [Console]::OutputEncoding = $previousOutputEncoding
}
