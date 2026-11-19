<#
.SYNOPSIS
Validates the third-party source manifests without downloading anything.

.DESCRIPTION
CI and scripts/build_student_release.ps1 run this. For
release/third-party-sources.json (the current runtime) and
release/third-party-sources-runtime3.json (releases 1.0.0-1.0.2) it:

- runs scripts/build_third_party_sources.ps1 -ValidateOnly (structure, ids,
  https URLs, license references, directions, written-offer fields);
- requires every download and vendor tree to be pinned, so a release build
  cannot record new values unnoticed.

For the current manifest it also checks that it covers the runtime the
application ships:

- applies_to names the bundle revision, Arduino CLI and core of
  src-tauri/resources/arduino-runtime-manifest.json and the archive SHA-256
  pinned in release/release-manifest.json;
- the arduino-cli and ArduinoCore-renesas components have those versions;
- every tool and platform folder and every third-party file at the archive
  root listed in release/arduino-runtime-entries.json is claimed by a
  component's bundled_paths, and every removed tool (a forbidden
  data/packages/<package>/tools/<name>/ prefix) is a removed component.

It reads files only. Exit code 0 means everything is consistent.

.PARAMETER ProjectRoot
Repository root. Defaults to the parent of this script's folder.
#>
#Requires -Version 7.2
[CmdletBinding()]
param(
  [string]$ProjectRoot
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($ProjectRoot)) { $ProjectRoot = Join-Path $PSScriptRoot '..' }
$root = (Resolve-Path -LiteralPath $ProjectRoot).Path
$problems = [System.Collections.Generic.List[string]]::new()
$builder = Join-Path $root 'scripts\build_third_party_sources.ps1'

function Read-Json {
  param([Parameter(Mandatory = $true)][string]$Relative)
  $path = Join-Path $root $Relative
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { $problems.Add("$Relative is missing."); return $null }
  return [IO.File]::ReadAllText($path, [Text.Encoding]::UTF8) | ConvertFrom-Json -AsHashtable
}

$manifests = @('release/third-party-sources.json', 'release/third-party-sources-runtime3.json')
$loaded = @{}
foreach ($relative in $manifests) {
  & $builder -Manifest (Join-Path $root $relative) -ValidateOnly
  if ($LASTEXITCODE -ne 0) { $problems.Add("$relative failed build_third_party_sources.ps1 -ValidateOnly."); continue }
  $manifest = Read-Json $relative
  $loaded[$relative] = $manifest
  foreach ($component in @($manifest.components)) {
    if (-not $component['sha256']) { $problems.Add("$relative component $($component.id) has no pinned sha256.") }
    if ($component['go_vendored']) {
      foreach ($field in @('go_version', 'modules', 'files', 'files_sha256', 'modules_txt_sha256')) {
        $value = $component.vendor[$field]
        if ($null -eq $value -or "$value" -eq '') { $problems.Add("$relative component $($component.id) has no pinned vendor.$field.") }
      }
      if ("$($component.vendor['go_version'])" -cne "$($manifest['go_toolchain'])") {
        $problems.Add("$relative component $($component.id) vendor.go_version $($component.vendor['go_version']) differs from go_toolchain $($manifest['go_toolchain']).")
      }
    }
  }
  $versions = @($manifest.applies_to.app_versions)
  if ($versions.Count -eq 1 -and $manifest.bundle_name -cne "WVU-Bioinstrumentation-Studio-$($versions[0])-third-party-sources") {
    $problems.Add("$relative bundle_name $($manifest.bundle_name) does not match its app version $($versions[0]).")
  }
}

$current = $loaded['release/third-party-sources.json']
$runtimeManifest = Read-Json 'src-tauri/resources/arduino-runtime-manifest.json'
$releaseManifest = Read-Json 'release/release-manifest.json'
$entries = Read-Json 'release/arduino-runtime-entries.json'
if ($null -ne $current -and $null -ne $runtimeManifest -and $null -ne $releaseManifest -and $null -ne $entries) {
  $applies = $current.applies_to
  foreach ($pair in @(
      @('runtime_bundle_revision', $runtimeManifest.bundle_revision, 'src-tauri/resources/arduino-runtime-manifest.json bundle_revision'),
      @('runtime_arduino_cli', $runtimeManifest.arduino_cli, 'src-tauri/resources/arduino-runtime-manifest.json arduino_cli'),
      @('runtime_renesas_uno_core', $runtimeManifest.renesas_uno_core, 'src-tauri/resources/arduino-runtime-manifest.json renesas_uno_core'),
      @('runtime_archive_sha256', $releaseManifest.arduino_runtime_archive_sha256, 'release/release-manifest.json arduino_runtime_archive_sha256'))) {
    if ("$($applies[$pair[0]])" -cne "$($pair[1])") {
      $problems.Add("release/third-party-sources.json applies_to.$($pair[0]) is '$($applies[$pair[0]])'; $($pair[2]) is '$($pair[1])'.")
    }
  }
  foreach ($pair in @(@('bundle_revision', 'bundle_revision'), @('arduino_cli', 'arduino_cli'), @('renesas_uno_core', 'renesas_uno_core'))) {
    if ("$($entries[$pair[0]])" -cne "$($runtimeManifest[$pair[1]])") {
      $problems.Add("release/arduino-runtime-entries.json $($pair[0]) is '$($entries[$pair[0]])'; the runtime manifest names '$($runtimeManifest[$pair[1]])'.")
    }
  }
  $byId = @{}
  foreach ($component in @($current.components)) { $byId[$component.id] = $component }
  foreach ($pair in @(@('arduino-cli', $runtimeManifest.arduino_cli), @('ArduinoCore-renesas', $runtimeManifest.renesas_uno_core))) {
    if (-not $byId.ContainsKey($pair[0])) { $problems.Add("release/third-party-sources.json has no $($pair[0]) component.") }
    elseif ("$($byId[$pair[0]].version)" -cne "$($pair[1])") { $problems.Add("release/third-party-sources.json $($pair[0]) is version $($byId[$pair[0]].version); the runtime has $($pair[1]).") }
  }

  # Every shipped third-party item is claimed by a component.
  $claimed = @($current.components | ForEach-Object { @($_['bundled_paths']) } | Where-Object { $_ })
  $mustCover = [System.Collections.Generic.List[string]]::new()
  foreach ($tool in @($entries.tools)) { $mustCover.Add("data/packages/$($tool.package)/tools/$($tool.name)/$($tool.version)/") }
  foreach ($platform in @($entries.platforms)) { $mustCover.Add("data/packages/$($platform.package)/hardware/$($platform.architecture)/$($platform.version)/") }
  # Root files other than WVU's own runtime-manifest.json.
  foreach ($entry in @($entries.required_entries)) {
    if (-not $entry.Contains('/') -and $entry -cne 'runtime-manifest.json') { $mustCover.Add($entry) }
  }
  foreach ($item in $mustCover) {
    $covered = @($claimed | Where-Object { $_ -ceq $item -or ($_.EndsWith('/') -and $item.StartsWith($_, [StringComparison]::Ordinal)) -or ($item.EndsWith('/') -and $_.StartsWith($item, [StringComparison]::Ordinal)) })
    if ($covered.Count -eq 0) { $problems.Add("release/third-party-sources.json has no component whose bundled_paths cover $item.") }
  }
  $removedNames = @($current.removed_components | ForEach-Object { "$($_.component)".ToLowerInvariant() })
  foreach ($prefix in @($entries.forbidden_prefixes)) {
    $match = [regex]::Match($prefix, '^data/packages/[^/]+/tools/([^/]+)/$')
    if ($match.Success -and $removedNames -notcontains $match.Groups[1].Value.ToLowerInvariant()) {
      $problems.Add("release/third-party-sources.json removed_components does not list the removed tool $($match.Groups[1].Value).")
    }
  }
}

if ($problems.Count -gt 0) {
  Write-Host ''
  Write-Host "Third-party source manifests are inconsistent ($($problems.Count) problem(s)):" -ForegroundColor Red
  foreach ($problem in $problems) {
    Write-Host "- $problem" -ForegroundColor Red
    if ($env:GITHUB_ACTIONS -eq 'true') {
      Write-Host ("::error title=Third-party sources::{0}" -f (($problem -replace '%', '%25') -replace "`r?`n", '%0A'))
    }
  }
  exit 1
}
Write-Host 'Third-party source manifests are consistent with the runtime they describe.' -ForegroundColor Green
exit 0
