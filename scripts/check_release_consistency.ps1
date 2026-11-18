<#
.SYNOPSIS
Checks that the version, firmware and runtime metadata agree across the repository.

.DESCRIPTION
Reads files only and changes nothing. It checks that:

- The application version is the same in package.json, the package-lock.json
  root (both fields), src-tauri/Cargo.toml, the src-tauri/Cargo.lock root
  package, src-tauri/tauri.conf.json and release/release-manifest.json
  (app_version). CHANGELOG.md has a heading for that version, and
  RELEASE_NOTES_<version>.md exists.
- The firmware protocol version, build and device ID agree between
  src-tauri/src/protocol.rs and the reference .ino, and match the manifest's
  firmware_protocol and reference_firmware_build (and reference_device_id, if
  the manifest ever records it).
- The SHA-256 of the reference .ino matches the manifest's
  reference_firmware_source_sha256 and REFERENCE_SOURCE_SHA256 in
  src-tauri/src/reference_firmware.rs.
- The manifest's arduino_cli_version and arduino_core match both tracked
  runtime manifests, which must agree with each other, and the core version
  that src-tauri/src/arduino_runtime.rs patches.
- Each Tauri npm package in package-lock.json (@tauri-apps/api and
  @tauri-apps/plugin-*) is on the same major.minor release as its crate in
  src-tauri/Cargo.lock (tauri, tauri-plugin-*); `tauri build` refuses a
  mismatch.

All text comparisons are exact (case-sensitive).

Exit code 0 means everything agrees. Exit code 1 lists every mismatch.
CI and scripts/build_student_release.ps1 run it. It works in PowerShell 7 and
in Windows PowerShell 5.1.

.PARAMETER ProjectRoot
Repository root. Defaults to the parent of this script's folder.
#>
[CmdletBinding()]
param(
  [string]$ProjectRoot
)

$ErrorActionPreference = 'Stop'
# Resolved here, not as a parameter default: Windows PowerShell 5.1 leaves
# $PSScriptRoot empty in parameter defaults when started with -File.
if ([string]::IsNullOrWhiteSpace($ProjectRoot)) {
  $ProjectRoot = Join-Path $PSScriptRoot '..'
}
$root = (Resolve-Path -LiteralPath $ProjectRoot).Path
$problems = New-Object 'System.Collections.Generic.List[string]'
$passed = New-Object 'System.Collections.Generic.List[string]'

function Add-Problem {
  param([Parameter(Mandatory = $true)][string]$Message)
  $problems.Add($Message)
}

function Read-RepoText {
  param([Parameter(Mandatory = $true)][string]$RelativePath)
  $path = Join-Path $root $RelativePath
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
    Add-Problem "$RelativePath is missing."
    return $null
  }
  # Explicit UTF-8: Windows PowerShell 5.1 would otherwise read BOM-less files
  # in the ANSI code page.
  return [IO.File]::ReadAllText($path, [Text.Encoding]::UTF8)
}

function ConvertFrom-RepoJson {
  param(
    [Parameter(Mandatory = $true)][string]$RelativePath,
    # Untyped on purpose: a [string] parameter turns $null into ''.
    [AllowNull()]$Text
  )
  if ($null -eq $Text) { return $null }
  try {
    return $Text | ConvertFrom-Json
  } catch {
    Add-Problem "$RelativePath is not valid JSON: $($_.Exception.Message)"
    return $null
  }
}

function Get-JsonProperty {
  param(
    [AllowNull()]$Object,
    [Parameter(Mandatory = $true)][string]$Name
  )
  if ($null -eq $Object) { return $null }
  $property = $Object.PSObject.Properties[$Name]
  if ($null -eq $property) { return $null }
  return $property.Value
}

function Get-RegexValue {
  param(
    [AllowNull()]$Text,
    [Parameter(Mandatory = $true)][string]$Pattern,
    [Parameter(Mandatory = $true)][string]$Description
  )
  if ($null -eq $Text) { return $null }
  $match = [regex]::Match($Text, $Pattern)
  if (-not $match.Success) {
    Add-Problem "Could not find $Description."
    return $null
  }
  return $match.Groups[1].Value
}

# Parses a Rust or C integer literal such as 4, 0x0001_0006 or 0x554E4F34UL.
function ConvertTo-Integer {
  param([AllowNull()][string]$Literal)
  if ([string]::IsNullOrWhiteSpace($Literal)) { return $null }
  $digits = ($Literal -replace '_', '') -replace '(?i)(u8|u16|u32|u64|ul|u|l)$', ''
  try {
    if ($digits -match '^(?i)0x([0-9a-f]+)$') {
      return [Convert]::ToUInt64($Matches[1], 16)
    }
    return [UInt64]::Parse($digits, [Globalization.CultureInfo]::InvariantCulture)
  } catch {
    Add-Problem "'$Literal' is not an integer literal."
    return $null
  }
}

function Format-Hex32 {
  param([AllowNull()]$Value)
  if ($null -eq $Value) { return '(missing)' }
  return ('0x{0:X8}' -f [UInt64]$Value)
}

# Records one comparison: every source must report the same, non-empty value.
function Test-SameValue {
  param(
    [Parameter(Mandatory = $true)][string]$Label,
    [Parameter(Mandatory = $true)][System.Collections.Specialized.OrderedDictionary]$Values
  )
  # Exact, ordinal comparison: Sort-Object -Unique and -eq ignore case, but the
  # application compares these strings exactly (1.5.2-rc.1 is not 1.5.2-RC.1).
  $distinct = New-Object 'System.Collections.Generic.List[string]'
  $seen = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
  foreach ($value in $Values.Values) {
    if ($seen.Add([string]$value)) { $distinct.Add([string]$value) }
  }
  $missing = @($Values.Keys | Where-Object { [string]::IsNullOrWhiteSpace([string]$Values[$_]) })
  if ($missing.Count -eq 0 -and $distinct.Count -eq 1) {
    $passed.Add("${Label}: $($distinct[0])")
    return
  }
  $detail = ($Values.Keys | ForEach-Object {
    $shown = if ([string]::IsNullOrWhiteSpace([string]$Values[$_])) { '(missing)' } else { [string]$Values[$_] }
    "    ${_}: $shown"
  }) -join [Environment]::NewLine
  Add-Problem "$Label differs:$([Environment]::NewLine)$detail"
}

# --- Application version -----------------------------------------------------

$releaseManifestPath = 'release/release-manifest.json'
$releaseManifest = ConvertFrom-RepoJson $releaseManifestPath (Read-RepoText $releaseManifestPath)
$appVersion = [string](Get-JsonProperty $releaseManifest 'app_version')

$packageJson = ConvertFrom-RepoJson 'package.json' (Read-RepoText 'package.json')

# The lockfile's root package uses an empty key, which ConvertFrom-Json rejects
# without -AsHashtable (PowerShell 7 only). Rename that one key instead.
$packageLockText = Read-RepoText 'package-lock.json'
$packageLock = $null
if ($null -ne $packageLockText) {
  $rootKey = [regex]::new('("packages"\s*:\s*\{\s*)""(\s*:)')
  $packageLock = ConvertFrom-RepoJson 'package-lock.json' ($rootKey.Replace($packageLockText, '$1"__root_package__"$2', 1))
}
$packageLockRoot = Get-JsonProperty (Get-JsonProperty $packageLock 'packages') '__root_package__'

$cargoToml = Read-RepoText 'src-tauri/Cargo.toml'
$cargoPackage = Get-RegexValue $cargoToml '(?ms)^\[package\][ \t]*\r?$(.*?)(?=^\[|\z)' 'the [package] table in src-tauri/Cargo.toml'
$cargoName = Get-RegexValue $cargoPackage '(?m)^name\s*=\s*"([^"]+)"' 'the package name in src-tauri/Cargo.toml'
$cargoVersion = Get-RegexValue $cargoPackage '(?m)^version\s*=\s*"([^"]+)"' 'the package version in src-tauri/Cargo.toml'

$cargoLockText = Read-RepoText 'src-tauri/Cargo.lock'
$cargoLockVersion = $null
if ($cargoName) {
  $cargoLockVersion = Get-RegexValue $cargoLockText `
    "(?m)^\[\[package\]\]\r?\nname = `"$([regex]::Escape($cargoName))`"\r?\nversion = `"([^`"]+)`"" `
    "the $cargoName package in src-tauri/Cargo.lock"
}

$tauriConfig = ConvertFrom-RepoJson 'src-tauri/tauri.conf.json' (Read-RepoText 'src-tauri/tauri.conf.json')

$versions = [ordered]@{
  'release/release-manifest.json app_version' = $appVersion
  'package.json version' = Get-JsonProperty $packageJson 'version'
  'package-lock.json version' = Get-JsonProperty $packageLock 'version'
  'package-lock.json packages[""].version' = Get-JsonProperty $packageLockRoot 'version'
  'src-tauri/Cargo.toml [package] version' = $cargoVersion
  'src-tauri/Cargo.lock root package version' = $cargoLockVersion
  'src-tauri/tauri.conf.json version' = Get-JsonProperty $tauriConfig 'version'
}
Test-SameValue 'Application version' $versions

if ($appVersion -notmatch '^\d+\.\d+\.\d+$') {
  Add-Problem "release/release-manifest.json app_version '$appVersion' is not MAJOR.MINOR.PATCH."
} else {
  $changelog = Read-RepoText 'CHANGELOG.md'
  if ($null -ne $changelog) {
    if ($changelog -match "(?m)^##\s+\[?v?$([regex]::Escape($appVersion))\]?(?=\s|$)") {
      $passed.Add("CHANGELOG.md heading: $appVersion")
    } else {
      Add-Problem "CHANGELOG.md has no '## $appVersion' heading."
    }
  }
  $releaseNotes = "RELEASE_NOTES_$appVersion.md"
  if (Test-Path -LiteralPath (Join-Path $root $releaseNotes) -PathType Leaf) {
    $passed.Add("Release notes: $releaseNotes")
  } else {
    Add-Problem "$releaseNotes is missing."
  }
}

# --- Firmware identity --------------------------------------------------------

$protocolRs = Read-RepoText 'src-tauri/src/protocol.rs'
$inoPath = 'firmware/reference_unor4wifi/reference_unor4wifi.ino'
$ino = Read-RepoText $inoPath

$rustMajor = ConvertTo-Integer (Get-RegexValue $protocolRs '(?m)^\s*pub const PROTOCOL_MAJOR:\s*u8\s*=\s*([0-9A-Za-z_]+)\s*;' 'PROTOCOL_MAJOR in src-tauri/src/protocol.rs')
$rustMinor = ConvertTo-Integer (Get-RegexValue $protocolRs '(?m)^\s*pub const PROTOCOL_MINOR:\s*u8\s*=\s*([0-9A-Za-z_]+)\s*;' 'PROTOCOL_MINOR in src-tauri/src/protocol.rs')
$rustBuild = ConvertTo-Integer (Get-RegexValue $protocolRs '(?m)^\s*pub const REFERENCE_FIRMWARE_BUILD:\s*u32\s*=\s*([0-9A-Za-z_]+)\s*;' 'REFERENCE_FIRMWARE_BUILD in src-tauri/src/protocol.rs')
$rustDevice = ConvertTo-Integer (Get-RegexValue $protocolRs '(?m)^\s*pub const REFERENCE_DEVICE_ID:\s*u32\s*=\s*([0-9A-Za-z_]+)\s*;' 'REFERENCE_DEVICE_ID in src-tauri/src/protocol.rs')

$inoMajor = ConvertTo-Integer (Get-RegexValue $ino '\bPROTOCOL_MAJOR\s*=\s*([0-9A-Za-z]+)\s*[,;]' "PROTOCOL_MAJOR in $inoPath")
$inoMinor = ConvertTo-Integer (Get-RegexValue $ino '\bPROTOCOL_MINOR\s*=\s*([0-9A-Za-z]+)\s*[,;]' "PROTOCOL_MINOR in $inoPath")
$inoBuild = ConvertTo-Integer (Get-RegexValue $ino '\bFIRMWARE_BUILD\s*=\s*([0-9A-Za-z]+)\s*;' "FIRMWARE_BUILD in $inoPath")
$inoDevice = ConvertTo-Integer (Get-RegexValue $ino '\bDEVICE_ID\s*=\s*([0-9A-Za-z]+)\s*;' "DEVICE_ID in $inoPath")

$manifestProtocol = [string](Get-JsonProperty $releaseManifest 'firmware_protocol')
$manifestBuildText = [string](Get-JsonProperty $releaseManifest 'reference_firmware_build')
$manifestBuild = $null
if ($manifestBuildText -match '^0x[0-9A-Fa-f]{8}$') {
  $manifestBuild = ConvertTo-Integer $manifestBuildText
} else {
  Add-Problem "release/release-manifest.json reference_firmware_build '$manifestBuildText' is not 0x followed by 8 hex digits."
}

function Format-Protocol {
  param($Major, $Minor)
  if ($null -eq $Major -or $null -eq $Minor) { return $null }
  return "$Major.$Minor"
}

Test-SameValue 'Firmware protocol version' ([ordered]@{
  'src-tauri/src/protocol.rs PROTOCOL_MAJOR.PROTOCOL_MINOR' = Format-Protocol $rustMajor $rustMinor
  "$inoPath PROTOCOL_MAJOR.PROTOCOL_MINOR" = Format-Protocol $inoMajor $inoMinor
  'release/release-manifest.json firmware_protocol' = $manifestProtocol
})
Test-SameValue 'Reference firmware build' ([ordered]@{
  'src-tauri/src/protocol.rs REFERENCE_FIRMWARE_BUILD' = Format-Hex32 $rustBuild
  "$inoPath FIRMWARE_BUILD" = Format-Hex32 $inoBuild
  'release/release-manifest.json reference_firmware_build' = Format-Hex32 $manifestBuild
})
$deviceIds = [ordered]@{
  'src-tauri/src/protocol.rs REFERENCE_DEVICE_ID' = Format-Hex32 $rustDevice
  "$inoPath DEVICE_ID" = Format-Hex32 $inoDevice
}
$manifestDevice = Get-JsonProperty $releaseManifest 'reference_device_id'
if ($null -ne $manifestDevice) {
  $deviceIds['release/release-manifest.json reference_device_id'] = Format-Hex32 (ConvertTo-Integer ([string]$manifestDevice))
}
Test-SameValue 'Reference device ID' $deviceIds

# --- Reference firmware source hash --------------------------------------------

$inoFullPath = Join-Path $root $inoPath
$inoHash = $null
if (Test-Path -LiteralPath $inoFullPath -PathType Leaf) {
  $inoHash = (Get-FileHash -LiteralPath $inoFullPath -Algorithm SHA256).Hash.ToLowerInvariant()
}
$rustSourceHash = Get-RegexValue (Read-RepoText 'src-tauri/src/reference_firmware.rs') `
  'REFERENCE_SOURCE_SHA256:\s*&str\s*=\s*"([0-9A-Fa-f]{64})"' 'REFERENCE_SOURCE_SHA256 in src-tauri/src/reference_firmware.rs'
Test-SameValue 'Reference firmware source SHA-256' ([ordered]@{
  "$inoPath (computed)" = $inoHash
  'release/release-manifest.json reference_firmware_source_sha256' = ([string](Get-JsonProperty $releaseManifest 'reference_firmware_source_sha256')).ToLowerInvariant()
  'src-tauri/src/reference_firmware.rs REFERENCE_SOURCE_SHA256' = if ($rustSourceHash) { $rustSourceHash.ToLowerInvariant() } else { $null }
})

# --- Arduino runtime versions -------------------------------------------------

$runtimeManifestPath = 'src-tauri/resources/arduino-runtime-manifest.json'
$stagingManifestPath = 'src-tauri/resources/arduino-runtime/runtime-manifest.json'
$runtimeManifest = ConvertFrom-RepoJson $runtimeManifestPath (Read-RepoText $runtimeManifestPath)
$stagingManifest = ConvertFrom-RepoJson $stagingManifestPath (Read-RepoText $stagingManifestPath)
$patchedCore = Get-RegexValue (Read-RepoText 'src-tauri/src/arduino_runtime.rs') `
  'manifest\.renesas_uno_core\s*!=\s*"([^"]+)"' 'the core version checked by src-tauri/src/arduino_runtime.rs'

Test-SameValue 'Arduino CLI version' ([ordered]@{
  'release/release-manifest.json arduino_cli_version' = Get-JsonProperty $releaseManifest 'arduino_cli_version'
  "$runtimeManifestPath arduino_cli" = Get-JsonProperty $runtimeManifest 'arduino_cli'
  "$stagingManifestPath arduino_cli" = Get-JsonProperty $stagingManifest 'arduino_cli'
})
Test-SameValue 'Arduino UNO R4 core version' ([ordered]@{
  'release/release-manifest.json arduino_core' = Get-JsonProperty $releaseManifest 'arduino_core'
  "$runtimeManifestPath renesas_uno_core" = Get-JsonProperty $runtimeManifest 'renesas_uno_core'
  "$stagingManifestPath renesas_uno_core" = Get-JsonProperty $stagingManifest 'renesas_uno_core'
  'src-tauri/src/arduino_runtime.rs patched core' = $patchedCore
})
foreach ($field in @('runtime_schema', 'bundle_revision')) {
  Test-SameValue "Arduino runtime $field" ([ordered]@{
    "$runtimeManifestPath $field" = Get-JsonProperty $runtimeManifest $field
    "$stagingManifestPath $field" = Get-JsonProperty $stagingManifest $field
  })
}

# --- Tauri npm packages and Rust crates ---------------------------------------

# `tauri build` stops with "Found version mismatched Tauri packages" when an npm
# package and its Rust crate are on different major.minor releases:
# @tauri-apps/api and tauri, and @tauri-apps/plugin-<name> and
# tauri-plugin-<name>. Dependabot updates npm and Cargo in separate pull
# requests and CI does not run `tauri build`, so compare the lockfiles here.
# Like the Tauri CLI, a plugin package without a matching crate is skipped.
function Get-MajorMinor {
  param([AllowNull()][string]$Version)
  if ($Version -match '^(\d+\.\d+)\.') { return $Matches[1] }
  return $null
}

$tauriApiCompared = $false
$packageLockPackages = Get-JsonProperty $packageLock 'packages'
if ($null -ne $packageLockPackages -and $null -ne $cargoLockText) {
  foreach ($npmPackage in @($packageLockPackages.PSObject.Properties | Where-Object { $_.Name -cmatch '^node_modules/@tauri-apps/(api|plugin-[a-z0-9-]+)$' })) {
    $npmName = $npmPackage.Name.Substring('node_modules/'.Length)
    $crateName = if ($npmName -ceq '@tauri-apps/api') { 'tauri' } else { 'tauri-' + $npmName.Substring('@tauri-apps/'.Length) }
    $npmVersion = [string](Get-JsonProperty $npmPackage.Value 'version')
    $crateVersions = @([regex]::Matches($cargoLockText,
        "(?m)^\[\[package\]\]\r?\nname = `"$([regex]::Escape($crateName))`"\r?\nversion = `"([^`"]+)`"") |
      ForEach-Object { $_.Groups[1].Value })
    if ($crateVersions.Count -eq 0) { continue }
    if ($crateName -ceq 'tauri') { $tauriApiCompared = $true }
    $npmMajorMinor = Get-MajorMinor $npmVersion
    $mismatched = @($crateVersions | Where-Object { $null -eq $npmMajorMinor -or (Get-MajorMinor $_) -cne $npmMajorMinor })
    if ($mismatched.Count -eq 0) {
      $passed.Add("Tauri $npmName $npmVersion and $crateName $($crateVersions -join ', '): same major.minor $npmMajorMinor")
    } else {
      Add-Problem ("Tauri $npmName and $crateName are on different major.minor releases, so ``tauri build`` would stop:" +
        [Environment]::NewLine + "    package-lock.json ${npmName}: $(if ($npmVersion) { $npmVersion } else { '(missing)' })" +
        [Environment]::NewLine + "    src-tauri/Cargo.lock ${crateName}: $($crateVersions -join ', ')" +
        [Environment]::NewLine + '    Update the npm package and the crate together.')
    }
  }
}
if ($null -ne $packageLock -and $null -ne $cargoLockText -and -not $tauriApiCompared) {
  Add-Problem 'Could not compare @tauri-apps/api in package-lock.json with the tauri crate in src-tauri/Cargo.lock: one of them is missing.'
}

# --- Report -------------------------------------------------------------------

foreach ($line in $passed) {
  Write-Host "ok    $line"
}
if ($problems.Count -gt 0) {
  Write-Host ''
  Write-Host "Release metadata is inconsistent ($($problems.Count) problem(s)):" -ForegroundColor Red
  foreach ($problem in $problems) {
    Write-Host "- $problem" -ForegroundColor Red
    if ($env:GITHUB_ACTIONS -eq 'true') {
      # One annotation per problem; GitHub encodes newlines as %0A.
      Write-Host ("::error title=Release consistency::{0}" -f (($problem -replace '%', '%25') -replace "`r?`n", '%0A'))
    }
  }
  exit 1
}
Write-Host ''
Write-Host 'Release metadata is consistent.' -ForegroundColor Green
exit 0
