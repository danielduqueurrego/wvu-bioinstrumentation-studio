[CmdletBinding()]
param(
  [string]$ArchivePath = (Join-Path $PSScriptRoot '..\src-tauri\resources\arduino-runtime.zip'),
  [string]$LicenseRoot = (Join-Path $PSScriptRoot '..\src-tauri\resources\licenses')
)

$ErrorActionPreference = 'Stop'
$resolvedArchive = Resolve-Path -LiteralPath $ArchivePath -ErrorAction Stop
$resolvedLicenseRoot = Resolve-Path -LiteralPath $LicenseRoot -ErrorAction Stop
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

Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::OpenRead($resolvedArchive)
try {
  $requiredEntries = @(
    'arduino-cli.exe',
    'data/packages/arduino/hardware/renesas_uno/1.6.0/LICENSE',
    'data/packages/arduino/tools/arm-none-eabi-gcc/7-2017q4/share/doc/gcc-arm-none-eabi/license.txt',
    'data/packages/arduino/tools/bossac/1.9.1-arduino5/bossac.exe',
    'data/packages/builtin/tools/serial-discovery/1.5.2/LICENSE.txt'
  )
  $entryNames = @($archive.Entries | ForEach-Object FullName)
  foreach ($required in $requiredEntries) {
    if ($entryNames -notcontains $required) {
      throw "Bundled runtime is missing the reviewed component or notice: $required"
    }
  }

  # Arduino CLI v1.5.2-rc.1 publishes this exact GPLv3 text. The assembled
  # runtime carries the identical text with Arduino's discovery tools.
  $gplEntry = $archive.Entries |
    Where-Object FullName -eq 'data/packages/builtin/tools/serial-discovery/1.5.2/LICENSE.txt' |
    Select-Object -First 1
  $stream = $gplEntry.Open()
  try {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
      $gplHash = ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
    } finally {
      $sha.Dispose()
    }
  } finally {
    $stream.Dispose()
  }
  if ($gplHash -ne '3972dc9744f6499f0f9b2dbf76696f2ae7ad8af9b23dde66d6af86c9dfb36986') {
    throw "The retained Arduino GPLv3 license copy has an unexpected SHA-256: $gplHash"
  }

  $noticeEntries = @(
    $archive.Entries |
      Where-Object {
        $leaf = [System.IO.Path]::GetFileName($_.FullName)
        $leaf -match '^(?i:license|copying|notice)(\.|$|_)'
      } |
      Select-Object -ExpandProperty FullName |
      Sort-Object -Unique
  )
  if ($noticeEntries.Count -eq 0) {
    throw "No LICENSE, COPYING, or NOTICE files were found in $resolvedArchive. Do not publish this runtime until its upstream notices are reviewed."
  }

  Write-Host "Bundled-runtime notice inventory ($($noticeEntries.Count) files):"
  $noticeEntries
  Write-Host ''
  Write-Host 'Component-specific Arduino CLI and BOSSA notice checks passed.'
  Write-Host "Bundled component notice: $componentNotice"
} finally {
  $archive.Dispose()
}
