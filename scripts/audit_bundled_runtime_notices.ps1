<#
.SYNOPSIS
Checks the bundled Arduino runtime archive and the installed component notices.

.DESCRIPTION
Reads release/arduino-runtime-entries.json (shared with
scripts/build_student_release.ps1 and scripts/build_arduino_runtime.ps1) and
checks that the runtime archive:

- holds runtime-manifest.json with the entries file's Arduino CLI, core and
  bundle revision;
- contains every required entry, and none under a forbidden prefix (the
  removed openocd and dfu-util tools, per-machine Arduino CLI files);
- has exactly the listed tool and platform folders, so no tool ships without
  a notice and source review;
- carries every listed license file with its reviewed SHA-256: the Arduino
  CLI release's own GPLv3 LICENSE.txt as arduino-cli-LICENSE.txt, the GPL-2.0
  and LGPL-2.1 texts at the archive root, and the core's, toolchain's and
  discovery tools' license files.

It also checks the installed notices in src-tauri/resources/licenses:
BUNDLED_COMPONENTS.txt must name the Arduino CLI, the core, every tool
version and the source-request form, and bossac-LICENSE.txt must be the
reviewed BOSSA license. It changes nothing.

.PARAMETER ArchivePath
The runtime archive. Default: src-tauri/resources/arduino-runtime.zip.

.PARAMETER LicenseRoot
Folder of the installed notices. Default: src-tauri/resources/licenses.

.PARAMETER EntriesPath
Default: release/arduino-runtime-entries.json.
#>
[CmdletBinding()]
param(
  [string]$ArchivePath,
  [string]$LicenseRoot,
  [string]$EntriesPath
)

$ErrorActionPreference = 'Stop'
# Resolved here, not as parameter defaults: Windows PowerShell 5.1 leaves
# $PSScriptRoot empty in parameter defaults when started with -File.
$projectRoot = Join-Path $PSScriptRoot '..'
if ([string]::IsNullOrWhiteSpace($ArchivePath)) { $ArchivePath = Join-Path $projectRoot 'src-tauri\resources\arduino-runtime.zip' }
if ([string]::IsNullOrWhiteSpace($LicenseRoot)) { $LicenseRoot = Join-Path $projectRoot 'src-tauri\resources\licenses' }
if ([string]::IsNullOrWhiteSpace($EntriesPath)) { $EntriesPath = Join-Path $projectRoot 'release\arduino-runtime-entries.json' }
$resolvedArchive = (Resolve-Path -LiteralPath $ArchivePath -ErrorAction Stop).Path
$resolvedLicenseRoot = (Resolve-Path -LiteralPath $LicenseRoot -ErrorAction Stop).Path
$resolvedEntries = (Resolve-Path -LiteralPath $EntriesPath -ErrorAction Stop).Path
$entries = [IO.File]::ReadAllText($resolvedEntries, [Text.Encoding]::UTF8) | ConvertFrom-Json
if ($entries.schema -ne 1) { throw "$resolvedEntries has an unsupported schema." }

$componentNotice = Join-Path $resolvedLicenseRoot 'BUNDLED_COMPONENTS.txt'
$bossacLicense = Join-Path $resolvedLicenseRoot 'bossac-LICENSE.txt'
foreach ($required in @($componentNotice, $bossacLicense)) {
  if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
    throw "Required bundled-component notice is missing: $required"
  }
}
if (-not (Select-String -LiteralPath $bossacLicense -SimpleMatch 'Copyright (c) 2011-2016, ShumaTech' -Quiet)) {
  throw 'The bundled BOSSA license does not match the reviewed upstream notice.'
}
# The installed notice must describe this runtime.
$noticeText = [IO.File]::ReadAllText($componentNotice, [Text.Encoding]::UTF8)
$noticeNeeds = @(
  "Arduino CLI $($entries.arduino_cli)",
  "bundle revision $($entries.bundle_revision)",
  'issues/new?template=source_code_request.yml',
  'third-party-sources.zip'
)
$noticeNeeds += @($entries.platforms | ForEach-Object { "$($_.version)" })
$noticeNeeds += @($entries.tools | ForEach-Object { "$($_.version)" })
foreach ($needed in $noticeNeeds) {
  if (-not $noticeText.Contains($needed)) {
    throw "BUNDLED_COMPONENTS.txt does not mention '$needed'. Update it for runtime revision $($entries.bundle_revision)."
  }
}

function Get-EntrySha256 {
  param([Parameter(Mandatory = $true)]$Entry)
  $stream = $Entry.Open()
  try {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
      return ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
    } finally {
      $sha.Dispose()
    }
  } finally {
    $stream.Dispose()
  }
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::OpenRead($resolvedArchive)
try {
  $byName = @{}
  foreach ($entry in $archive.Entries) { $byName[$entry.FullName] = $entry }
  $entryNames = @($archive.Entries | ForEach-Object FullName)

  # The runtime the entries file describes.
  $manifestEntry = $byName['runtime-manifest.json']
  if ($null -eq $manifestEntry) { throw 'Bundled runtime has no runtime-manifest.json.' }
  $reader = New-Object System.IO.StreamReader($manifestEntry.Open(), [Text.Encoding]::UTF8)
  try { $runtimeManifest = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
  foreach ($pair in @(@('arduino_cli', 'arduino_cli'), @('renesas_uno_core', 'renesas_uno_core'), @('bundle_revision', 'bundle_revision'))) {
    if ([string]$runtimeManifest.($pair[0]) -cne [string]$entries.($pair[1])) {
      throw "The runtime's runtime-manifest.json $($pair[0]) is '$($runtimeManifest.($pair[0]))'; release/arduino-runtime-entries.json describes '$($entries.($pair[1]))'."
    }
  }

  foreach ($required in @($entries.required_entries)) {
    if (-not $byName.ContainsKey($required)) {
      throw "Bundled runtime is missing the reviewed component or notice: $required"
    }
  }
  foreach ($prefix in @($entries.forbidden_prefixes)) {
    $forbidden = @($entryNames | Where-Object { $_.StartsWith($prefix, [StringComparison]::Ordinal) } | Select-Object -First 1)
    if ($forbidden.Count -gt 0) { throw "Bundled runtime contains an entry it must not ship: $($forbidden[0])" }
  }

  # Exactly the reviewed tool and platform folders.
  $expectedFolders = @(
    @($entries.tools | ForEach-Object { "data/packages/$($_.package)/tools/$($_.name)/$($_.version)/" }) +
    @($entries.platforms | ForEach-Object { "data/packages/$($_.package)/hardware/$($_.architecture)/$($_.version)/" })
  ) | Sort-Object -Unique
  $actualFolders = @($entryNames | ForEach-Object {
      $match = [regex]::Match($_, '^data/packages/[^/]+/(tools|hardware)/[^/]+/[^/]+/')
      if ($match.Success) { $match.Value }
    }) | Sort-Object -Unique
  $unexpected = @($actualFolders | Where-Object { $expectedFolders -cnotcontains $_ })
  $missing = @($expectedFolders | Where-Object { $actualFolders -cnotcontains $_ })
  if ($unexpected.Count -gt 0) { throw "Bundled runtime has tool or platform folders without a reviewed notice: $($unexpected -join ', ')" }
  if ($missing.Count -gt 0) { throw "Bundled runtime lacks reviewed tool or platform folders: $($missing -join ', ')" }

  # The reviewed license texts, byte for byte. arduino-cli-LICENSE.txt is the
  # Arduino CLI release's own LICENSE.txt (GPLv3).
  foreach ($license in @($entries.license_entries)) {
    $entry = $byName[$license.path]
    if ($null -eq $entry) { throw "Bundled runtime is missing the license file $($license.path)." }
    $hash = Get-EntrySha256 $entry
    if ($hash -cne $license.sha256) {
      throw "The license file $($license.path) has SHA-256 $hash; the reviewed $($license.license) is $($license.sha256)."
    }
  }

  $noticeEntries = @(
    $archive.Entries |
      Where-Object {
        $leaf = [System.IO.Path]::GetFileName($_.FullName)
        $leaf -match '^(?i:license|copying|notice)(\.|$|_)' -or $leaf -match '(?i)-license\.txt$' -or $leaf -match '^(GPL-2\.0|LGPL-2\.1)\.txt$'
      } |
      Select-Object -ExpandProperty FullName |
      Sort-Object -Unique
  )
  if ($noticeEntries.Count -eq 0) {
    throw "No LICENSE, COPYING, or NOTICE files were found in $resolvedArchive. Do not publish this runtime until its upstream notices are reviewed."
  }

  Write-Host "Bundled runtime: Arduino CLI $($runtimeManifest.arduino_cli), core $($runtimeManifest.renesas_uno_core), bundle revision $($runtimeManifest.bundle_revision)."
  Write-Host "Bundled-runtime notice inventory ($($noticeEntries.Count) files):"
  $noticeEntries
  Write-Host ''
  Write-Host "Checked $(@($entries.required_entries).Count) required entries, $(@($entries.license_entries).Count) license SHA-256 values, $($expectedFolders.Count) tool and platform folders and $(@($entries.forbidden_prefixes).Count) forbidden prefixes."
  Write-Host "Bundled component notice: $componentNotice"
} finally {
  $archive.Dispose()
}
