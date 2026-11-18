#Requires -Version 7.0
<#
.SYNOPSIS
Rebuilds scripts/install_op_amp_lab_profile.zip, the student download for the
Op Amp Lab installer, or checks that the committed ZIP is current.

.DESCRIPTION
The ZIP always holds exactly these two entries, in this order, at its root:

  1. install_op_amp_lab_profile.bat
  2. install_op_amp_lab_profile.ps1

Each entry is the script from this folder with its line endings converted to
CRLF, whatever the checkout used, so cmd.exe and Windows PowerShell 5.1 read
it natively. Entries are stored without compression and carry a fixed
timestamp, so the same scripts always give a byte-identical ZIP. The scripts
must stay ASCII: Windows PowerShell 5.1 reads a BOM-less script in the ANSI
code page.

After editing either script, run this script and commit the regenerated ZIP
with the script change. With -Check it writes nothing: it compares the
committed ZIP's entries (names, order and contents) with the CRLF scripts and
exits with code 1 if the ZIP is stale. CI runs the -Check form.

.PARAMETER Check
Verify the committed ZIP instead of rebuilding it.

.PARAMETER OutputPath
Where to write or read the ZIP. Defaults to install_op_amp_lab_profile.zip
next to this script.
#>
[CmdletBinding()]
param(
  [switch]$Check,
  [string]$OutputPath = (Join-Path $PSScriptRoot 'install_op_amp_lab_profile.zip')
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem

$entryNames = @(
  'install_op_amp_lab_profile.bat',
  'install_op_amp_lab_profile.ps1'
)
# Fixed so that rebuilding unchanged scripts never changes the ZIP's bytes.
$entryTimestamp = [DateTimeOffset]::new(2026, 1, 1, 0, 0, 0, [TimeSpan]::Zero)

function Get-CrlfScriptBytes {
  param([Parameter(Mandatory = $true)][string]$Name)
  $path = Join-Path $PSScriptRoot $Name
  $bytes = [IO.File]::ReadAllBytes($path)
  for ($index = 0; $index -lt $bytes.Length; $index++) {
    if ($bytes[$index] -gt 0x7F) {
      throw "$Name contains a non-ASCII byte at offset $index. Keep the student installer scripts ASCII so Windows PowerShell 5.1 reads them unchanged."
    }
  }
  $text = [Text.Encoding]::ASCII.GetString($bytes)
  $crlf = ($text -replace "`r`n", "`n") -replace "`n", "`r`n"
  return , [Text.Encoding]::ASCII.GetBytes($crlf)
}

function New-InstallerZipBytes {
  $buffer = [IO.MemoryStream]::new()
  $archive = [IO.Compression.ZipArchive]::new($buffer, [IO.Compression.ZipArchiveMode]::Create, $true)
  try {
    foreach ($name in $entryNames) {
      $content = Get-CrlfScriptBytes $name
      $entry = $archive.CreateEntry($name, [IO.Compression.CompressionLevel]::NoCompression)
      $entry.LastWriteTime = $entryTimestamp
      $stream = $entry.Open()
      try {
        $stream.Write($content, 0, $content.Length)
      } finally {
        $stream.Dispose()
      }
    }
  } finally {
    $archive.Dispose()
  }
  return , $buffer.ToArray()
}

function Get-Sha256Hex {
  param([Parameter(Mandatory = $true)][byte[]]$Bytes)
  $sha256 = [Security.Cryptography.SHA256]::Create()
  try {
    return ([BitConverter]::ToString($sha256.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant()
  } finally {
    $sha256.Dispose()
  }
}

function Read-ZipEntryBytes {
  param([Parameter(Mandatory = $true)]$Entry)
  $stream = $Entry.Open()
  try {
    $copy = [IO.MemoryStream]::new()
    $stream.CopyTo($copy)
    return , $copy.ToArray()
  } finally {
    $stream.Dispose()
  }
}

if ($Check) {
  if (-not (Test-Path -LiteralPath $OutputPath -PathType Leaf)) {
    Write-Host "Missing student installer ZIP: $OutputPath" -ForegroundColor Red
    exit 1
  }
  $problems = @()
  $archive = [IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $OutputPath).Path)
  try {
    $actualNames = @($archive.Entries | ForEach-Object FullName)
    if (($actualNames -join '|') -cne ($entryNames -join '|')) {
      $problems += "entries are [$($actualNames -join ', ')]; expected [$($entryNames -join ', ')]"
    } else {
      foreach ($entry in $archive.Entries) {
        $expected = Get-CrlfScriptBytes $entry.FullName
        $actual = Read-ZipEntryBytes $entry
        if ((Get-Sha256Hex $actual) -ne (Get-Sha256Hex $expected)) {
          $problems += "$($entry.FullName) differs from the CRLF copy of scripts/$($entry.FullName)"
        }
      }
    }
  } finally {
    $archive.Dispose()
  }
  if ($problems.Count -gt 0) {
    Write-Host "The student installer ZIP is stale: $OutputPath" -ForegroundColor Red
    foreach ($problem in $problems) {
      Write-Host "- $problem" -ForegroundColor Red
    }
    Write-Host 'Run pwsh -NoProfile -File scripts/package_op_amp_lab_installer.ps1 and commit the ZIP.'
    exit 1
  }
  $builtHash = Get-Sha256Hex (New-InstallerZipBytes)
  $committedHash = (Get-FileHash -LiteralPath $OutputPath -Algorithm SHA256).Hash.ToLowerInvariant()
  Write-Host "The student installer ZIP matches the scripts: $OutputPath"
  if ($builtHash -ne $committedHash) {
    # Same entries and contents, different container bytes (for example a
    # different .NET ZIP writer). Not an error; rebuilding would normalize it.
    Write-Host "Note: a rebuild would produce different ZIP bytes ($builtHash) than the committed file ($committedHash)."
  }
  exit 0
}

$zipBytes = New-InstallerZipBytes
$outputFullPath = [IO.Path]::GetFullPath($OutputPath)
$temporaryPath = "$outputFullPath.tmp"
try {
  [IO.File]::WriteAllBytes($temporaryPath, $zipBytes)
  Move-Item -LiteralPath $temporaryPath -Destination $outputFullPath -Force
} finally {
  if (Test-Path -LiteralPath $temporaryPath) {
    Remove-Item -LiteralPath $temporaryPath -Force
  }
}
Write-Host "Wrote $outputFullPath"
Write-Host "Entries: $($entryNames -join ', ') (CRLF, stored, fixed timestamp)"
Write-Host "SHA-256: $(Get-Sha256Hex $zipBytes)"
exit 0
