<#
.SYNOPSIS
Builds the corresponding-source bundle for the GPL/LGPL third-party tools in
WVU Bioinstrumentation Studio's offline Arduino runtime.

.DESCRIPTION
The Windows installers carry resources\arduino-runtime.zip, which holds
Arduino CLI, Arduino's discovery and monitor tools, ctags, the GNU Arm
toolchain and the Arduino UNO R4 core (and, up to runtime revision 3, OpenOCD
and dfu-util). Several are GPL or LGPL programs. This script assembles their
source into one ZIP that is published beside the installers. It never changes
the repository, except that -RecordHashes rewrites the -Manifest it is given.

The input is a manifest (release/third-party-sources.json for the current
runtime, release/third-party-sources-runtime3.json for releases 1.0.0-1.0.2).
Every text in the bundle's README.txt that depends on a particular runtime
comes from the manifest: the project description, the components and their
notes, the build directions, the license references and the source that the
maintainers do not hold. -ValidateOnly checks a manifest without downloading
anything (CI runs it through scripts/check_third_party_sources.ps1), and
-VerifyBundle checks a finished bundle against the manifest, also offline
(scripts/build_student_release.ps1 runs it on -ThirdPartySourcesZip).

For each component the script:

 1. Downloads source_url into its own new, empty folder under the run folder,
    or copies it from -DownloadCache when a file with the pinned SHA-256 is
    cached there.
 2. Checks the file's magic bytes (gzip, bzip2, zip) and its SHA-256 against
    the manifest. A missing SHA-256 stops the build unless -RecordHashes is
    given, which records it. A recorded value is never overwritten: a
    mismatch always stops the build.
 3. Checks published_checksum (MD5, SHA1, SHA256 or SHA512 published by
    upstream) and re-reads the publisher's checksum file when it can.
 4. For GitHub archives with a "git" entry, reads the commit ID that git
    archive stores in the tarball's pax header and requires git.commit.
 5. For go_vendored components, extracts the archive and runs 'go mod vendor'
    and 'go mod verify' with the manifest's go_toolchain (GOTOOLCHAIN is set
    to it, so Go uses or downloads exactly that release; the checksum
    database verifies the download), GOFLAGS=-mod=mod,
    GOPROXY=https://proxy.golang.org, GOSUMDB=sum.golang.org, GOENV=off and
    GOPATH, GOCACHE, APPDATA and LOCALAPPDATA inside the run folder. It then
    checks the vendor tree against the manifest's "vendor" pins (see below)
    and packs it as <id>-<version>-vendor.zip (entries under vendor/).
 6. With -Runtime (an extracted runtime folder or a runtime ZIP, read only),
    checks that every bundled path exists, that the runtime is the one the
    manifest names, and, for go_binaries, that each executable's embedded Go
    build information names git.commit with vcs.modified=false and links only
    modules whose version and h1: hash appear in the tag's go.sum and in
    vendor/modules.txt.

It then writes README.txt, SHA256SUMS.txt and a copy of the manifest, and
packs everything into <BundleName>.zip with one wrapper folder, ordinal-sorted
'/' entry names and the manifest's fixed zip_timestamp. Already-compressed
files are stored; the rest is deflated. <BundleName>.zip.sha256 (sha256sum
layout) is written beside it.

What is pinned, and what is reproducible:
- Every download is pinned by SHA-256. GitHub generates tag archives on
  request and has said it keeps them stable; if one ever changes, step 2
  stops the build and the archive inside a published bundle stays
  authoritative.
- A vendor tree is pinned by content, not by ZIP bytes:
    vendor.go_version          the Go release that ran 'go mod vendor'
    vendor.modules             module count in vendor/modules.txt
    vendor.files               file count, modules.txt excluded
    vendor.files_sha256        SHA-256 of the UTF-8 text made of one line
                               "<sha256 of file>  <path under vendor/>\n" per
                               file (modules.txt excluded), sorted by path
                               (ordinal)
    vendor.modules_txt_sha256  SHA-256 of vendor/modules.txt
  The file selection and modules.txt are written by Go, so they are only
  comparable with the same go_toolchain; the script refuses a different one.
- The vendor ZIPs and the bundle ZIP are byte-identical across runs with the
  same PowerShell and .NET versions (the .NET Deflate encoder writes the
  bytes). Another .NET version may compress differently, so their SHA-256 is
  reported, not pinned; the published bundle's .sha256 is what users check.

.PARAMETER Manifest
Path of the manifest JSON. With -RecordHashes it is rewritten (UTF-8, LF)
with the recorded values.

.PARAMETER OutputDir
Folder for <BundleName>.zip and <BundleName>.zip.sha256. Created if missing.
Not needed with -ValidateOnly.

.PARAMETER BundleName
Name of the bundle (ZIP name and wrapper folder). Defaults to the manifest's
bundle_name; when both are given they must match.

.PARAMETER RecordHashes
Record values that the manifest leaves empty (sha256 and the vendor pins)
instead of failing. Pinned values are still verified and never replaced.

.PARAMETER ValidateOnly
Check the manifest's structure and exit. Nothing is downloaded or written.

.PARAMETER VerifyBundle
Path of a finished <BundleName>.zip to check against -Manifest, then exit.
It is read only; nothing is downloaded or written, and Go is not needed. The
bundle must have the one wrapper folder named bundle_name and safe '/' entry
names; SHA256SUMS.txt must list every other file with its SHA-256; the
third-party-sources.json inside must equal -Manifest; every component's file
must have the manifest's sha256; every Go component's vendor ZIP must match
its vendor pins (file count, files_sha256, modules_txt_sha256, modules); and
there must be no other file.

.PARAMETER WorkRoot
Parent folder of the run folders. Default C:\wvu-srcbuild. The script marks
the folder it creates (file .wvu-third-party-sources) and refuses a
non-empty folder without that marker. Keep it short: Go module paths below it
are long.

.PARAMETER Clean
Before this run, delete earlier run folders under -WorkRoot. Only folders
that carry this script's run marker (.wvu-third-party-sources-run) are
deleted.

.PARAMETER KeepRunFolder
Keep this run's downloads, extracted sources, Go caches and staging folder.
By default a successful run deletes them and keeps logs\ and build-log.txt.

.PARAMETER DownloadCache
Optional folder of verified downloads, reused across runs: <cache>\<sha256>\
<file name>. A cached file is used only when the manifest pins that SHA-256
and the file still has it; every verified download is added.

.PARAMETER GoModCache
Optional GOMODCACHE shared across runs. Go checks every cached module
against go.sum ('go mod verify').

.PARAMETER Runtime
Optional extracted runtime folder or runtime ZIP to cross-check (read only).

.PARAMETER GoExe
Go executable used for vendoring and for reading build information.
Default: go.exe on PATH, else C:\Program Files\Go\bin\go.exe.

.PARAMETER Force
Replace an existing <BundleName>.zip and .sha256 in -OutputDir.

.EXAMPLE
pwsh -NoProfile -File scripts\build_third_party_sources.ps1 -Manifest release\third-party-sources.json -ValidateOnly

.EXAMPLE
pwsh -NoProfile -File scripts\build_third_party_sources.ps1 -Manifest release\third-party-sources.json -OutputDir dist -Runtime src-tauri\resources\arduino-runtime.zip -Force

Release build: every pin must match, and the runtime archive is cross-checked.

.EXAMPLE
pwsh -NoProfile -File scripts\build_third_party_sources.ps1 -Manifest release\third-party-sources.json -VerifyBundle C:\wvu-srcout\WVU-Bioinstrumentation-Studio-1.1.0-third-party-sources.zip

Checks a bundle built earlier, for example before it is published.

.EXAMPLE
pwsh -NoProfile -File scripts\build_third_party_sources.ps1 -Manifest release\third-party-sources.json -OutputDir C:\wvu-srcout -RecordHashes

After adding a component or changing a version: review the downloads and the
vendor trees, then commit the recorded manifest.
#>
#Requires -Version 7.2
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][string]$Manifest,
  [string]$OutputDir,
  [string]$BundleName,
  [switch]$RecordHashes,
  [switch]$ValidateOnly,
  [string]$VerifyBundle,
  [string]$WorkRoot = 'C:\wvu-srcbuild',
  [switch]$Clean,
  [switch]$KeepRunFolder,
  [string]$DownloadCache,
  [string]$GoModCache,
  [string]$Runtime,
  [string]$GoExe,
  [switch]$Force
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$UserAgent = 'third-party-sources-builder/2'
$ManifestSchema = 2
$StoredExtensions = @('.gz', '.tgz', '.bz2', '.xz', '.zip', '.7z', '.zst')
$SafeName = '^[A-Za-z0-9][A-Za-z0-9._+-]*$'
$WorkRootMarker = '.wvu-third-party-sources'
$RunMarker = '.wvu-third-party-sources-run'

# --- Helpers ----------------------------------------------------------------------

$script:Log = [System.Collections.Generic.List[string]]::new()

function Write-Step {
  param([Parameter(Mandatory = $true)][string]$Message)
  $script:Log.Add("==> $Message")
  Write-Host ''
  Write-Host "==> $Message" -ForegroundColor Cyan
}

function Write-Note {
  param([Parameter(Mandatory = $true)][string]$Message)
  $script:Log.Add($Message)
  Write-Host $Message
}

# Relative paths are resolved against PowerShell's current location, not the
# .NET process directory (which `& script.ps1` from another folder keeps).
function Resolve-FullPath {
  param([Parameter(Mandatory = $true)][string]$Path)
  return $PSCmdlet.GetUnresolvedProviderPathFromPSPath($Path)
}

function Get-Field {
  param($Table, [string]$Name, $Default = $null)
  if ($Table -is [System.Collections.IDictionary] -and $Table.Contains($Name)) { return $Table[$Name] }
  return $Default
}

function Get-FileHashText {
  param([Parameter(Mandatory = $true)][string]$Path, [string]$Algorithm = 'SHA256')
  return (Get-FileHash -LiteralPath $Path -Algorithm $Algorithm).Hash.ToLowerInvariant()
}

function Get-TextSha256 {
  param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)
  $sha = [Security.Cryptography.SHA256]::Create()
  try {
    return [Convert]::ToHexString($sha.ComputeHash([Text.UTF8Encoding]::new($false).GetBytes($Text))).ToLowerInvariant()
  } finally { $sha.Dispose() }
}

function New-EmptyDirectory {
  param([Parameter(Mandatory = $true)][string]$Path)
  if (Test-Path -LiteralPath $Path) {
    if (@(Get-ChildItem -LiteralPath $Path -Force).Count -gt 0) { throw "Folder is not empty: $Path" }
  } else {
    New-Item -ItemType Directory -Path $Path | Out-Null
  }
}

function Test-PathInside {
  param([string]$Child, [string]$Parent)
  $c = [IO.Path]::GetFullPath($Child).TrimEnd('\')
  $p = [IO.Path]::GetFullPath($Parent).TrimEnd('\')
  return ($c -ieq $p) -or $c.StartsWith("$p\", [StringComparison]::OrdinalIgnoreCase)
}

# Downloads one URL into a new, empty folder. Retries a failed transfer twice.
function Save-Download {
  param(
    [Parameter(Mandatory = $true)][string]$Url,
    [Parameter(Mandatory = $true)][string]$Folder,
    [Parameter(Mandatory = $true)][string]$FileName
  )
  New-EmptyDirectory $Folder
  $path = Join-Path $Folder $FileName
  for ($attempt = 1; ; $attempt++) {
    try {
      Write-Note "Downloading $Url"
      Invoke-WebRequest -Uri $Url -OutFile $path -MaximumRedirection 10 -UserAgent $UserAgent -TimeoutSec 1800
      return $path
    } catch {
      if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
      if ($attempt -ge 3) { throw "Download failed after $attempt attempts: $Url ($($_.Exception.Message))" }
      Write-Warning "Download attempt $attempt failed: $Url ($($_.Exception.Message)). Retrying."
      Start-Sleep -Seconds (5 * $attempt)
    }
  }
}

# A component's source file: from -DownloadCache when a file with the pinned
# SHA-256 is cached there, otherwise downloaded. The caller verifies the hash
# either way; Add-ToDownloadCache stores a verified download.
function Get-ComponentSource {
  param(
    [Parameter(Mandatory = $true)]$Component,
    [Parameter(Mandatory = $true)][string]$Folder
  )
  $pinned = Get-Field $Component 'sha256' ''
  if ($script:DownloadCachePath -and $pinned) {
    $cached = Join-Path (Join-Path $script:DownloadCachePath $pinned) $Component.source_filename
    if ((Test-Path -LiteralPath $cached -PathType Leaf) -and (Get-FileHashText $cached) -ceq $pinned) {
      New-EmptyDirectory $Folder
      $path = Join-Path $Folder $Component.source_filename
      Copy-Item -LiteralPath $cached -Destination $path
      Write-Note "From the download cache: $cached (downloaded earlier from $($Component.source_url))"
      return $path
    }
  }
  return Save-Download $Component.source_url $Folder $Component.source_filename
}

function Add-ToDownloadCache {
  param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][string]$Sha256)
  if (-not $script:DownloadCachePath) { return }
  $folder = Join-Path $script:DownloadCachePath $Sha256
  $target = Join-Path $folder ([IO.Path]::GetFileName($Path))
  if (Test-Path -LiteralPath $target -PathType Leaf) { return }
  New-Item -ItemType Directory -Force -Path $folder | Out-Null
  Copy-Item -LiteralPath $Path -Destination $target
}

function Assert-MagicBytes {
  param([Parameter(Mandatory = $true)][string]$Path)
  $name = [IO.Path]::GetFileName($Path).ToLowerInvariant()
  $expected = $null
  if ($name.EndsWith('.tar.gz') -or $name.EndsWith('.tgz')) { $expected = @(0x1f, 0x8b) }
  elseif ($name.EndsWith('.tar.bz2')) { $expected = @(0x42, 0x5a, 0x68) }
  elseif ($name.EndsWith('.zip')) { $expected = @(0x50, 0x4b) }
  if ($null -eq $expected) { return }
  $stream = [IO.File]::OpenRead($Path)
  try {
    $head = [byte[]]::new($expected.Count)
    $read = $stream.Read($head, 0, $head.Length)
  } finally { $stream.Dispose() }
  for ($i = 0; $i -lt $expected.Count; $i++) {
    if ($read -le $i -or $head[$i] -ne $expected[$i]) {
      throw "$name does not start with the expected archive signature (an HTML error page or a changed format?)."
    }
  }
}

# git archive stores the commit ID in a pax global header ("52 comment=<id>").
function Get-GitArchiveCommit {
  param([Parameter(Mandatory = $true)][string]$Path)
  $file = [IO.File]::OpenRead($Path)
  try {
    $gzip = [IO.Compression.GZipStream]::new($file, [IO.Compression.CompressionMode]::Decompress)
    try {
      $buffer = [byte[]]::new(1536)
      $read = 0
      while ($read -lt $buffer.Length) {
        $n = $gzip.Read($buffer, $read, $buffer.Length - $read)
        if ($n -le 0) { break }
        $read += $n
      }
    } finally { $gzip.Dispose() }
  } finally { $file.Dispose() }
  if ($read -lt 1024) { return $null }
  $name = [Text.Encoding]::ASCII.GetString($buffer, 0, 100).TrimEnd([char]0)
  if ($name -ne 'pax_global_header' -or [char]$buffer[156] -ne 'g') { return $null }
  $sizeText = [Text.Encoding]::ASCII.GetString($buffer, 124, 12).Trim([char]0, ' ')
  $size = [Convert]::ToInt32($sizeText, 8)
  $data = [Text.Encoding]::UTF8.GetString($buffer, 512, [Math]::Min($size, 1024))
  $match = [regex]::Match($data, '(?m)^\d+ comment=([0-9a-f]{40})$')
  if ($match.Success) { return $match.Groups[1].Value }
  return $null
}

# Extracts a .tar.gz with .NET's TarReader (Windows tar.exe cannot create the
# symbolic links some test folders contain). Writes only regular files and
# folders inside -Destination; returns the links it skipped as 'name -> target'.
function Expand-TarGz {
  param(
    [Parameter(Mandatory = $true)][string]$Archive,
    [Parameter(Mandatory = $true)][string]$Destination
  )
  $root = [IO.Path]::GetFullPath($Destination).TrimEnd('\')
  $skipped = [System.Collections.Generic.List[string]]::new()
  $file = [IO.File]::OpenRead($Archive)
  try {
    $gzip = [IO.Compression.GZipStream]::new($file, [IO.Compression.CompressionMode]::Decompress)
    $reader = [System.Formats.Tar.TarReader]::new($gzip)
    try {
      while ($null -ne ($entry = $reader.GetNextEntry())) {
        $type = "$($entry.EntryType)"
        if ($type -in @('GlobalExtendedAttributes', 'ExtendedAttributes', 'LongPath', 'LongLink')) { continue }
        $name = $entry.Name
        if ($name.StartsWith('/') -or $name.Contains(':') -or $name.Contains('\') -or ($name -split '/') -contains '..') { throw "Unsafe tar entry name: $name" }
        $target = [IO.Path]::GetFullPath((Join-Path $root $name.TrimEnd('/').Replace('/', '\')))
        if (-not $target.StartsWith("$root\", [StringComparison]::OrdinalIgnoreCase)) { throw "Tar entry escapes the folder: $name" }
        switch ($type) {
          'Directory' { [IO.Directory]::CreateDirectory($target) | Out-Null }
          { $_ -in @('RegularFile', 'V7RegularFile', 'ContiguousFile') } {
            [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target)) | Out-Null
            $entry.ExtractToFile($target, $false)
          }
          { $_ -in @('SymbolicLink', 'HardLink') } { $skipped.Add("$($name.TrimEnd('/')) -> $($entry.LinkName)") }
          default { throw "Unsupported tar entry type $type for $name" }
        }
      }
    } finally { $reader.Dispose() }
  } finally { $file.Dispose() }
  return $skipped
}

$script:GoLogIndex = 0
# Runs Go with an isolated environment; restores the session's variables and
# $ErrorActionPreference afterwards.
function Invoke-Go {
  param(
    [Parameter(Mandatory = $true)][string]$Directory,
    [Parameter(Mandatory = $true)][string[]]$Arguments,
    [switch]$AllowFailure
  )
  $saved = @{}
  foreach ($name in $script:GoEnvironment.Keys) {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name)
    [Environment]::SetEnvironmentVariable($name, $script:GoEnvironment[$name])
  }
  $previousPreference = $ErrorActionPreference
  try {
    # Go reports progress on stderr; only the exit code decides success. The
    # relaxed preference covers this native call only.
    $ErrorActionPreference = 'Continue'
    $output = @(& $script:Go -C $Directory @Arguments 2>&1 | ForEach-Object { "$_" })
    $code = $LASTEXITCODE
  } finally {
    $ErrorActionPreference = $previousPreference
    foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
  }
  $script:GoLogIndex++
  $logFile = Join-Path $script:LogDir ('{0:D3}-go.txt' -f $script:GoLogIndex)
  $text = "go -C `"$Directory`" $($Arguments -join ' ')`nexit code: $code`n" + (@($output) -join "`n") + "`n"
  [IO.File]::WriteAllText($logFile, $text, [Text.UTF8Encoding]::new($false))
  $script:Log.Add("go -C `"$Directory`" $($Arguments -join ' ')  (exit $code, log $logFile)")
  if ($code -ne 0 -and -not $AllowFailure) {
    throw "go $($Arguments -join ' ') failed with exit code $code. Output: $logFile"
  }
  return [pscustomobject]@{ ExitCode = $code; Output = @($output) }
}

# The content pins of a vendor/ folder (see the help above).
function Get-VendorTreePins {
  param([Parameter(Mandatory = $true)][string]$VendorDir)
  $root = [IO.Path]::GetFullPath($VendorDir).TrimEnd('\')
  $entries = [System.Collections.Generic.List[string]]::new()
  foreach ($file in [IO.Directory]::EnumerateFiles($root, '*', [IO.SearchOption]::AllDirectories)) {
    $relative = $file.Substring($root.Length + 1).Replace('\', '/')
    if ($relative -ceq 'modules.txt') { continue }
    $entries.Add($relative)
  }
  $entries.Sort([StringComparer]::Ordinal)
  $listing = [Text.StringBuilder]::new()
  foreach ($relative in $entries) {
    [void]$listing.Append((Get-FileHashText (Join-Path $root $relative))).Append('  ').Append($relative).Append("`n")
  }
  $modulesTxt = Join-Path $root 'modules.txt'
  return [pscustomobject]@{
    Files = $entries.Count
    FilesSha256 = Get-TextSha256 $listing.ToString()
    ModulesTxtSha256 = Get-FileHashText $modulesTxt
    Modules = @(Get-Content -LiteralPath $modulesTxt | Where-Object { $_ -match '^# ' }).Count
    Listing = $listing.ToString()
  }
}

function Get-CompressionLevel {
  param([string]$Name)
  $lower = $Name.ToLowerInvariant()
  foreach ($extension in $StoredExtensions) {
    if ($lower.EndsWith($extension)) { return [IO.Compression.CompressionLevel]::NoCompression }
  }
  return [IO.Compression.CompressionLevel]::Optimal
}

# Ordinal-sorted entries under one prefix, '/' separators, one timestamp.
# Files imply their folders; only empty folders get their own entry.
function New-DeterministicZip {
  param(
    [Parameter(Mandatory = $true)][string]$SourceRoot,
    [Parameter(Mandatory = $true)][string]$Destination,
    [Parameter(Mandatory = $true)][string]$Prefix,
    [Parameter(Mandatory = $true)][DateTimeOffset]$Timestamp
  )
  $root = [IO.Path]::GetFullPath($SourceRoot).TrimEnd('\')
  $sources = [System.Collections.Generic.Dictionary[string, string]]::new([StringComparer]::Ordinal)
  foreach ($item in Get-ChildItem -LiteralPath $root -Recurse -Force) {
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Links are not allowed in a bundle: $($item.FullName)" }
    $relative = $item.FullName.Substring($root.Length + 1).Replace('\', '/')
    $name = if ($Prefix) { "$Prefix/$relative" } else { $relative }
    if ($item.PSIsContainer) {
      if (@(Get-ChildItem -LiteralPath $item.FullName -Force).Count -eq 0) { $sources["$name/"] = '' }
    } else {
      $sources[$name] = $item.FullName
    }
  }
  $names = [System.Collections.Generic.List[string]]::new($sources.Keys)
  $names.Sort([StringComparer]::Ordinal)
  $seen = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
  foreach ($name in $names) { if (-not $seen.Add($name)) { throw "Two entries differ only in case: $name" } }
  $stream = [IO.File]::Open($Destination, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
  try {
    $zip = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create, $false, [Text.UTF8Encoding]::new($false))
    try {
      foreach ($name in $names) {
        $entry = $zip.CreateEntry($name, (Get-CompressionLevel $name))
        $entry.LastWriteTime = $Timestamp
        if (-not $name.EndsWith('/')) {
          $reader = [IO.File]::OpenRead($sources[$name])
          try {
            $writer = $entry.Open()
            try { $reader.CopyTo($writer) } finally { $writer.Dispose() }
          } finally { $reader.Dispose() }
        }
      }
    } finally { $zip.Dispose() }
  } finally { $stream.Dispose() }
  return $sources
}

# Re-reads a finished ZIP: entry order, names, timestamps and every file's bytes.
function Test-DeterministicZip {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)]$Sources,
    [Parameter(Mandatory = $true)][DateTimeOffset]$Timestamp
  )
  $sha = [Security.Cryptography.SHA256]::Create()
  $archive = [IO.Compression.ZipFile]::OpenRead($Path)
  try {
    $names = @($archive.Entries | ForEach-Object FullName)
    $sorted = [System.Collections.Generic.List[string]]::new([string[]]$names)
    $sorted.Sort([StringComparer]::Ordinal)
    if (($names -join "`n") -cne ($sorted -join "`n")) { throw "$Path entries are not in ordinal order." }
    if ($names.Count -ne $Sources.Count) { throw "$Path has $($names.Count) entries; expected $($Sources.Count)." }
    foreach ($entry in $archive.Entries) {
      $name = $entry.FullName
      if (-not $Sources.ContainsKey($name)) { throw "$Path has an unexpected entry: $name" }
      if ($name.Contains('\') -or $name.StartsWith('/') -or $name.Contains(':') -or ($name -split '/') -contains '..') { throw "Unsafe entry name: $name" }
      if ($entry.LastWriteTime.DateTime -ne $Timestamp.DateTime) { throw "Entry timestamp is not fixed: $name" }
      if ($name.EndsWith('/')) { continue }
      $entryStream = $entry.Open()
      try { $entryHash = [Convert]::ToHexString($sha.ComputeHash($entryStream)).ToLowerInvariant() } finally { $entryStream.Dispose() }
      if ($entryHash -cne (Get-FileHashText $Sources[$name])) { throw "Entry content differs from its source: $name" }
    }
  } finally {
    $archive.Dispose()
    $sha.Dispose()
  }
}

# --- Checking a finished bundle (-VerifyBundle) -------------------------------------

function Get-ZipEntrySha256 {
  param([Parameter(Mandatory = $true)][IO.Compression.ZipArchiveEntry]$Entry)
  $sha = [Security.Cryptography.SHA256]::Create()
  $stream = $Entry.Open()
  try { return [Convert]::ToHexString($sha.ComputeHash($stream)).ToLowerInvariant() } finally {
    $stream.Dispose()
    $sha.Dispose()
  }
}

# The content pins (see the help above) of a vendor ZIP inside a bundle.
function Get-VendorZipPins {
  param([Parameter(Mandatory = $true)][IO.Compression.ZipArchiveEntry]$Entry)
  $buffer = [IO.MemoryStream]::new()
  $stream = $Entry.Open()
  try { $stream.CopyTo($buffer) } finally { $stream.Dispose() }
  $buffer.Position = 0
  $vendor = [IO.Compression.ZipArchive]::new($buffer, [IO.Compression.ZipArchiveMode]::Read)
  try {
    $hashes = [System.Collections.Generic.Dictionary[string, string]]::new([StringComparer]::Ordinal)
    $modulesTxt = $null
    $modules = 0
    $outside = 0
    foreach ($entry in $vendor.Entries) {
      $name = $entry.FullName
      if ($name.EndsWith('/')) { continue }
      if (-not $name.StartsWith('vendor/', [StringComparison]::Ordinal)) { $outside++; continue }
      $relative = $name.Substring('vendor/'.Length)
      if ($relative -ceq 'modules.txt') {
        $modulesTxt = Get-ZipEntrySha256 $entry
        $reader = [IO.StreamReader]::new($entry.Open(), [Text.UTF8Encoding]::new($false))
        try { $modules = @(($reader.ReadToEnd() -split "`r?`n") | Where-Object { $_ -match '^# ' }).Count } finally { $reader.Dispose() }
        continue
      }
      $hashes[$relative] = Get-ZipEntrySha256 $entry
    }
    $names = [System.Collections.Generic.List[string]]::new($hashes.Keys)
    $names.Sort([StringComparer]::Ordinal)
    $listing = [Text.StringBuilder]::new()
    foreach ($relative in $names) { [void]$listing.Append($hashes[$relative]).Append('  ').Append($relative).Append("`n") }
    return [pscustomobject]@{
      Files = $names.Count
      FilesSha256 = Get-TextSha256 $listing.ToString()
      ModulesTxtSha256 = $modulesTxt
      Modules = $modules
      Outside = $outside
    }
  } finally {
    $vendor.Dispose()
    $buffer.Dispose()
  }
}

# Checks a finished bundle against its manifest without writing anything.
# Returns a list of problems; empty when the bundle is the one the manifest
# describes.
function Test-SourcesBundle {
  param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)]$Data)
  $problems = [System.Collections.Generic.List[string]]::new()
  $prefix = "$($Data.bundle_name)/"
  $archive = [IO.Compression.ZipFile]::OpenRead($Path)
  try {
    $files = [System.Collections.Generic.Dictionary[string, IO.Compression.ZipArchiveEntry]]::new([StringComparer]::Ordinal)
    foreach ($entry in $archive.Entries) {
      $name = $entry.FullName
      if ($name.Contains('\') -or $name.Contains(':') -or $name.StartsWith('/') -or ($name -split '/') -contains '..') { $problems.Add("Unsafe entry name: $name"); continue }
      if (-not $name.StartsWith($prefix, [StringComparison]::Ordinal)) { $problems.Add("Entry outside the wrapper folder $prefix`: $name"); continue }
      if ($name.EndsWith('/')) { continue }
      $relative = $name.Substring($prefix.Length)
      if ($files.ContainsKey($relative)) { $problems.Add("Duplicate entry: $name"); continue }
      $files[$relative] = $entry
    }

    # SHA256SUMS.txt lists every other file, with its content's SHA-256.
    $sumsName = 'SHA256SUMS.txt'
    if (-not $files.ContainsKey($sumsName)) {
      $problems.Add("$sumsName is missing.")
    } else {
      $reader = [IO.StreamReader]::new($files[$sumsName].Open(), [Text.UTF8Encoding]::new($false))
      try { $sumsText = $reader.ReadToEnd() } finally { $reader.Dispose() }
      $listed = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
      foreach ($line in ($sumsText -split "`n")) {
        if ($line -eq '') { continue }
        if ($line -cnotmatch '^([0-9a-f]{64})  (\S.*)$') { $problems.Add("$sumsName has a malformed line: $line"); continue }
        $hash = $Matches[1]
        $relative = $Matches[2]
        if (-not $listed.Add($relative)) { $problems.Add("$sumsName lists $relative twice."); continue }
        if (-not $files.ContainsKey($relative)) { $problems.Add("$sumsName lists $relative, which the bundle does not contain."); continue }
        if ((Get-ZipEntrySha256 $files[$relative]) -cne $hash) { $problems.Add("$relative does not match its SHA-256 in $sumsName.") }
      }
      foreach ($relative in $files.Keys) {
        if ($relative -cne $sumsName -and -not $listed.Contains($relative)) { $problems.Add("$sumsName does not list $relative.") }
      }
    }

    # The manifest copy is this manifest.
    $manifestName = 'third-party-sources.json'
    if (-not $files.ContainsKey($manifestName)) {
      $problems.Add("$manifestName is missing.")
    } else {
      $reader = [IO.StreamReader]::new($files[$manifestName].Open(), [Text.UTF8Encoding]::new($false))
      try { $bundled = $reader.ReadToEnd() | ConvertFrom-Json -AsHashtable } finally { $reader.Dispose() }
      if (($bundled | ConvertTo-Json -Depth 32 -Compress) -cne ($Data | ConvertTo-Json -Depth 32 -Compress)) {
        $problems.Add("The bundle's $manifestName differs from the manifest: it was built from another manifest.")
      }
    }

    # Every component's file, and only those files.
    $expected = [System.Collections.Generic.HashSet[string]]::new([string[]]@('README.txt', $sumsName, $manifestName), [StringComparer]::Ordinal)
    foreach ($c in @($Data.components)) {
      $id = $c.id
      $relative = if ($c.kind -eq 'license-text') { "licenses/$($c.source_filename)" } else { "sources/$id/$($c.source_filename)" }
      [void]$expected.Add($relative)
      $pinned = Get-Field $c 'sha256' ''
      if (-not $files.ContainsKey($relative)) { $problems.Add("$id`: $relative is missing.") }
      elseif (-not $pinned) { $problems.Add("$id has no pinned sha256 in the manifest.") }
      elseif ((Get-ZipEntrySha256 $files[$relative]) -cne $pinned) { $problems.Add("$id`: $relative does not have the pinned SHA-256 $pinned.") }
      if (-not (Get-Field $c 'go_vendored' $false)) { continue }
      $vendorRelative = "sources/$id/$id-$($c.version)-vendor.zip"
      [void]$expected.Add($vendorRelative)
      if (-not $files.ContainsKey($vendorRelative)) { $problems.Add("$id`: $vendorRelative is missing."); continue }
      $vendor = Get-Field $c 'vendor' @{}
      $pins = Get-VendorZipPins $files[$vendorRelative]
      if ($pins.Outside -gt 0) { $problems.Add("$id`: $vendorRelative has $($pins.Outside) file(s) outside vendor/.") }
      foreach ($pair in @(@('files', $pins.Files), @('files_sha256', $pins.FilesSha256), @('modules_txt_sha256', $pins.ModulesTxtSha256), @('modules', $pins.Modules))) {
        $pinnedValue = Get-Field $vendor $pair[0]
        if ($null -eq $pinnedValue -or "$pinnedValue" -eq '') { $problems.Add("$id has no pinned vendor.$($pair[0]) in the manifest.") }
        elseif ("$pinnedValue" -cne "$($pair[1])") { $problems.Add("$id`: $vendorRelative has vendor.$($pair[0]) $($pair[1]); the manifest pins $pinnedValue.") }
      }
    }
    foreach ($relative in $files.Keys) {
      if (-not $expected.Contains($relative)) { $problems.Add("Unexpected file in the bundle: $relative") }
    }
  } finally {
    $archive.Dispose()
  }
  return @($problems)
}

function Format-Wrapped {
  param([string]$Text, [int]$Indent, [int]$Width = 78, [string]$FirstPrefix = '')
  $pad = ' ' * $Indent
  $lines = [System.Collections.Generic.List[string]]::new()
  $line = $pad + $FirstPrefix
  $lineHasWord = $false
  # Keep each "-X name=value" linker flag on one line.
  $words = [System.Collections.Generic.List[string]]::new()
  foreach ($word in ($Text -split '\s+' | Where-Object { $_ -ne '' })) {
    if ($words.Count -gt 0 -and ($words[$words.Count - 1] -ceq '-X' -or $words[$words.Count - 1] -ceq '"-X')) { $words[$words.Count - 1] += " $word" } else { $words.Add($word) }
  }
  foreach ($word in $words) {
    if ($lineHasWord -and ($line.Length + 1 + $word.Length) -gt $Width) {
      $lines.Add($line)
      $line = $pad + (' ' * $FirstPrefix.Length) + $word
    } else {
      $line = if ($lineHasWord) { "$line $word" } else { "$line$word" }
    }
    $lineHasWord = $true
  }
  if ($lineHasWord -or $FirstPrefix) { $lines.Add($line) }
  return $lines
}

function ConvertTo-ManifestJson {
  param([Parameter(Mandatory = $true)]$Data)
  return (($Data | ConvertTo-Json -Depth 32) -replace "`r`n", "`n") + "`n"
}

# --- Manifest validation (also -ValidateOnly) -------------------------------------

# Returns a list of problems; empty when the manifest is well formed.
function Test-SourcesManifest {
  param([Parameter(Mandatory = $true)]$Data)
  $problems = [System.Collections.Generic.List[string]]::new()
  $isHex64 = { param($value) "$value" -match '^[0-9a-f]{64}$' }
  if ((Get-Field $Data 'schema') -ne $ManifestSchema) { $problems.Add("schema must be $ManifestSchema.") }
  foreach ($field in @('bundle_name', 'title', 'source_request_url', 'zip_timestamp')) {
    if (-not (Get-Field $Data $field)) { $problems.Add("missing top-level field '$field'.") }
  }
  $bundle = Get-Field $Data 'bundle_name'
  if ($bundle -and $bundle -notmatch $SafeName) { $problems.Add("unsafe bundle_name: $bundle") }
  $request = Get-Field $Data 'source_request_url'
  if ($request -and $request -notmatch '^https://') { $problems.Add('source_request_url must use https.') }
  $years = Get-Field $Data 'written_offer_years'
  if ($null -eq $years -or [int]$years -lt 3) { $problems.Add('written_offer_years must be at least 3.') }
  $project = Get-Field $Data 'project'
  foreach ($field in @('name', 'description', 'license', 'url', 'maintainers')) {
    if (-not (Get-Field $project $field)) { $problems.Add("project.$field is missing.") }
  }
  $applies = Get-Field $Data 'applies_to'
  foreach ($field in @('app_versions', 'installers', 'runtime_archive', 'runtime_extracted_to', 'runtime_bundle_revision', 'runtime_arduino_cli', 'runtime_renesas_uno_core', 'runtime_archive_sha256')) {
    if ($null -eq (Get-Field $applies $field)) { $problems.Add("applies_to.$field is missing.") }
  }
  $archiveSha = Get-Field $applies 'runtime_archive_sha256'
  if ($archiveSha -and -not (& $isHex64 $archiveSha)) { $problems.Add('applies_to.runtime_archive_sha256 is not a lowercase SHA-256.') }

  $components = @(Get-Field $Data 'components' @())
  if ($components.Count -eq 0) { $problems.Add('components is empty.') }
  $ids = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
  $targets = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
  $hasGo = $false
  $licenseRefsUsed = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
  foreach ($c in $components) {
    $id = Get-Field $c 'id' '(no id)'
    foreach ($field in @('id', 'component', 'version', 'kind', 'license', 'source_url', 'source_filename', 'notes')) {
      if (-not (Get-Field $c $field)) { $problems.Add("component '$id' lacks '$field'.") }
    }
    if ("$id" -notmatch $SafeName) { $problems.Add("unsafe component id: $id") }
    elseif (-not $ids.Add($id)) { $problems.Add("duplicate component id: $id") }
    $file = Get-Field $c 'source_filename' ''
    if ($file -and $file -notmatch $SafeName) { $problems.Add("unsafe source_filename for ${id}: $file") }
    $kind = Get-Field $c 'kind' ''
    if ($kind -notin @('source', 'license-only', 'license-text')) { $problems.Add("unknown kind '$kind' for $id.") }
    $url = Get-Field $c 'source_url' ''
    if ($url -and $url -notmatch '^https://') { $problems.Add("source_url of $id must use https.") }
    $sha = Get-Field $c 'sha256' ''
    if ($sha -and -not (& $isHex64 $sha)) { $problems.Add("malformed sha256 for $id.") }
    $git = Get-Field $c 'git'
    if ($git -and ("$(Get-Field $git 'commit')" -notmatch '^[0-9a-f]{40}$' -or -not (Get-Field $git 'repository') -or -not (Get-Field $git 'ref'))) {
      $problems.Add("git of $id needs repository, ref and a 40-hex commit.")
    }
    $published = Get-Field $c 'published_checksum'
    if ($published -and ("$(Get-Field $published 'algorithm')".ToUpperInvariant() -notin @('MD5', 'SHA1', 'SHA256', 'SHA512') -or -not (Get-Field $published 'value'))) {
      $problems.Add("published_checksum of $id needs algorithm (MD5, SHA1, SHA256 or SHA512) and value.")
    }
    $exact = Get-Field $c 'exact' $true
    if ($exact -isnot [bool]) { $problems.Add("exact of $id must be true or false.") }
    elseif (-not $exact) {
      foreach ($field in @('offer_subject', 'exact_source_holder')) {
        if (-not (Get-Field $c $field)) { $problems.Add("component $id is not exact source, so it needs '$field'.") }
      }
    }
    if (Get-Field $c 'go_vendored' $false) {
      $hasGo = $true
      if ($kind -ne 'source') { $problems.Add("go_vendored needs kind 'source': $id") }
      if ($file -and -not $file.EndsWith('.tar.gz')) { $problems.Add("go_vendored sources must be .tar.gz: $id") }
      if (Get-Field $c 'vendor_sha256') { $problems.Add("$id uses the retired vendor_sha256 (ZIP bytes); pin 'vendor' instead.") }
      $vendor = Get-Field $c 'vendor'
      if ($null -eq $vendor) { $problems.Add("go_vendored component $id has no 'vendor' pins.") }
      else {
        foreach ($field in @('files_sha256', 'modules_txt_sha256')) {
          $value = Get-Field $vendor $field ''
          if ($value -and -not (& $isHex64 $value)) { $problems.Add("vendor.$field of $id is not a lowercase SHA-256.") }
        }
      }
    }
    foreach ($ref in [regex]::Matches("$(Get-Field $c 'license' '')", 'LicenseRef-[A-Za-z0-9.-]+')) { [void]$licenseRefsUsed.Add($ref.Value) }
    $target = if ($kind -eq 'license-text') { "licenses/$file" } else { "sources/$id/$file" }
    if (-not $targets.Add($target)) { $problems.Add("two components write $target.") }
  }
  if ($hasGo -and "$(Get-Field $Data 'go_toolchain' '')" -notmatch '^go\d+\.\d+(\.\d+)?$') {
    $problems.Add("go_toolchain (for example go1.27.4) is required when a component is go_vendored.")
  }
  $licenseRefs = Get-Field $Data 'license_refs' @{}
  foreach ($ref in $licenseRefsUsed) {
    if ($ref -ceq 'LicenseRef-GNU-license-text') { continue }
    if (-not (Get-Field $licenseRefs $ref)) { $problems.Add("license_refs has no text for $ref.") }
  }
  $directions = @(Get-Field $Data 'directions' @())
  if ($directions.Count -eq 0) { $problems.Add('directions is empty: the README needs clear build directions.') }
  foreach ($direction in $directions) {
    if (-not (Get-Field $direction 'title') -or -not (Get-Field $direction 'text')) { $problems.Add('every direction needs a title and a text.') }
    foreach ($ref in @(Get-Field $direction 'components' @())) {
      if (-not $ids.Contains($ref)) { $problems.Add("direction '$(Get-Field $direction 'title')' names component '$ref', which this manifest does not list.") }
    }
  }
  foreach ($removed in @(Get-Field $Data 'removed_components' @())) {
    foreach ($field in @('component', 'version', 'removed_in_revision', 'notes')) {
      if ($null -eq (Get-Field $removed $field)) { $problems.Add("removed_components entry lacks '$field'.") }
    }
  }
  $timestamp = Get-Field $Data 'zip_timestamp'
  if ($timestamp -and $timestamp -isnot [DateTime] -and "$timestamp" -notmatch '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}$') {
    $problems.Add('zip_timestamp must look like 2026-11-18T00:00:00.')
  }
  return @($problems)
}

# --- Runtime access (read only) ------------------------------------------------------

$script:RuntimeZip = $null
$script:RuntimeEntries = $null
$script:RuntimeFolder = $null

function Open-Runtime {
  param([Parameter(Mandatory = $true)][string]$Path)
  if (Test-Path -LiteralPath $Path -PathType Container) {
    $script:RuntimeFolder = $Path.TrimEnd('\')
  } elseif ((Test-Path -LiteralPath $Path -PathType Leaf) -and $Path.EndsWith('.zip', [StringComparison]::OrdinalIgnoreCase)) {
    $script:RuntimeZip = [IO.Compression.ZipFile]::OpenRead($Path)
    $script:RuntimeEntries = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($entry in $script:RuntimeZip.Entries) { [void]$script:RuntimeEntries.Add($entry.FullName) }
  } else {
    throw "-Runtime must be an extracted runtime folder or a .zip: $Path"
  }
}

function Test-RuntimePath {
  param([Parameter(Mandatory = $true)][string]$Relative)
  if ($script:RuntimeFolder) {
    $local = Join-Path $script:RuntimeFolder ($Relative.TrimEnd('/').Replace('/', '\'))
    if ($Relative.EndsWith('/')) { return Test-Path -LiteralPath $local -PathType Container }
    return Test-Path -LiteralPath $local -PathType Leaf
  }
  if ($Relative.EndsWith('/')) {
    foreach ($name in $script:RuntimeEntries) { if ($name.StartsWith($Relative, [StringComparison]::Ordinal)) { return $true } }
    return $false
  }
  return $script:RuntimeEntries.Contains($Relative)
}

# Returns a local path to a runtime file: the file itself for a folder, or a copy
# extracted into a new folder for a ZIP.
function Get-RuntimeFile {
  param([Parameter(Mandatory = $true)][string]$Relative, [Parameter(Mandatory = $true)][string]$Scratch)
  if ($script:RuntimeFolder) { return Join-Path $script:RuntimeFolder $Relative.Replace('/', '\') }
  New-EmptyDirectory $Scratch
  $entry = $script:RuntimeZip.GetEntry($Relative)
  if ($null -eq $entry) { throw "Runtime archive has no entry $Relative" }
  $target = Join-Path $Scratch ([IO.Path]::GetFileName($Relative))
  [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $target)
  return $target
}

# --- Inputs ---------------------------------------------------------------------------

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.Formats.Tar

$ManifestPath = (Resolve-Path -LiteralPath (Resolve-FullPath $Manifest)).Path
$manifestData = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json -AsHashtable
$manifestProblems = @(Test-SourcesManifest $manifestData)
if ($manifestProblems.Count -gt 0) {
  throw "$ManifestPath is not a valid third-party sources manifest:`n - $($manifestProblems -join "`n - ")"
}
$components = @($manifestData.components)
if ($ValidateOnly) {
  $vendored = @($components | Where-Object { Get-Field $_ 'go_vendored' $false }).Count
  $unpinned = @($components | Where-Object { -not (Get-Field $_ 'sha256') }).Count
  Write-Host "$ManifestPath is valid: $($components.Count) components ($vendored Go-vendored), bundle $($manifestData.bundle_name), runtime revision $($manifestData.applies_to.runtime_bundle_revision)."
  if ($unpinned -gt 0) { Write-Warning "$unpinned component(s) have no pinned sha256 yet; a release build needs every pin." }
  exit 0
}
if ($VerifyBundle) {
  $bundlePath = Resolve-FullPath $VerifyBundle
  if (-not (Test-Path -LiteralPath $bundlePath -PathType Leaf)) { throw "-VerifyBundle file not found: $bundlePath" }
  if ([IO.Path]::GetFileName($bundlePath) -cne "$($manifestData.bundle_name).zip") {
    throw "-VerifyBundle must be $($manifestData.bundle_name).zip for this manifest: $bundlePath"
  }
  $bundleProblems = @(Test-SourcesBundle -Path $bundlePath -Data $manifestData)
  if ($bundleProblems.Count -gt 0) {
    throw "$bundlePath is not the bundle $ManifestPath describes:`n - $($bundleProblems -join "`n - ")"
  }
  $vendored = @($components | Where-Object { Get-Field $_ 'go_vendored' $false }).Count
  Write-Host "$bundlePath matches $ManifestPath`: SHA256SUMS.txt, the manifest copy, $($components.Count) component files with their pinned SHA-256 and $vendored vendor trees with their pins."
  exit 0
}

if ([string]::IsNullOrWhiteSpace($OutputDir)) { throw '-OutputDir is required unless -ValidateOnly is given.' }
$manifestBundleName = $manifestData.bundle_name
if (-not $BundleName) { $BundleName = $manifestBundleName }
if ($manifestBundleName -cne $BundleName) {
  throw "-BundleName '$BundleName' differs from the manifest's bundle_name '$manifestBundleName'."
}
if ($BundleName -notmatch $SafeName) { throw "Unsafe bundle name: $BundleName" }
$appliesTo = $manifestData.applies_to
$project = $manifestData.project
$requestUrl = $manifestData.source_request_url
$offerYears = [int]$manifestData.written_offer_years
$zipTimestampValue = $manifestData.zip_timestamp
# ConvertFrom-Json turns ISO date strings into DateTime values; accept both.
if ($zipTimestampValue -is [DateTime]) {
  $zipDate = [DateTime]::SpecifyKind($zipTimestampValue, [DateTimeKind]::Unspecified)
} else {
  $zipDate = [DateTime]::ParseExact("$zipTimestampValue", 'yyyy-MM-ddTHH:mm:ss', [Globalization.CultureInfo]::InvariantCulture)
}
if ($zipDate.Year -lt 1980) { throw 'zip_timestamp must be 1980 or later (ZIP dates start in 1980).' }
$manifestData['zip_timestamp'] = $zipDate.ToString('yyyy-MM-ddTHH:mm:ss', [Globalization.CultureInfo]::InvariantCulture)
$ZipTimestamp = [DateTimeOffset]::new($zipDate, [TimeSpan]::Zero)
$hasGo = @($components | Where-Object { Get-Field $_ 'go_vendored' $false }).Count -gt 0
$goToolchain = Get-Field $manifestData 'go_toolchain' ''

if (-not $GoExe) {
  $onPath = Get-Command go.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  $GoExe = if ($onPath) { $onPath.Source } else { 'C:\Program Files\Go\bin\go.exe' }
} else {
  $GoExe = Resolve-FullPath $GoExe
}
if ($hasGo -or $Runtime) {
  if (-not (Test-Path -LiteralPath $GoExe -PathType Leaf)) { throw "Go was not found: $GoExe" }
}
$script:Go = $GoExe

$OutputDir = Resolve-FullPath $OutputDir
$WorkRoot = (Resolve-FullPath $WorkRoot).TrimEnd('\')
if ($Runtime) { $Runtime = Resolve-FullPath $Runtime }
$script:DownloadCachePath = if ($DownloadCache) { Resolve-FullPath $DownloadCache } else { $null }
if ($GoModCache) { $GoModCache = Resolve-FullPath $GoModCache }
if ([IO.Path]::GetPathRoot($WorkRoot).TrimEnd('\') -eq $WorkRoot) { throw "-WorkRoot must not be a drive root: $WorkRoot" }
foreach ($protected in @(
    (Join-Path $env:LOCALAPPDATA 'WVU-BMEG'),
    (Join-Path $env:LOCALAPPDATA 'WVU Bioinstrumentation Studio'),
    (Join-Path $env:LOCALAPPDATA 'Arduino15'),
    (Join-Path $env:LOCALAPPDATA 'arduino'))) {
  foreach ($path in @($WorkRoot, $OutputDir, $script:DownloadCachePath, $GoModCache)) {
    if (-not $path) { continue }
    if ((Test-PathInside $path $protected) -or (Test-PathInside $protected $path)) { throw "Refusing to write in or above $protected ($path)." }
  }
}
if ((Test-PathInside $OutputDir $WorkRoot) -or (Test-PathInside $WorkRoot $OutputDir)) { throw '-OutputDir and -WorkRoot must not contain each other.' }

$zipPath = Join-Path $OutputDir "$BundleName.zip"
$shaPath = "$zipPath.sha256"
foreach ($path in @($zipPath, $shaPath)) {
  if ((Test-Path -LiteralPath $path) -and -not $Force) { throw "$path exists. Use -Force to replace it." }
}

# The work root belongs to this script: it carries the marker, or it is new.
if (Test-Path -LiteralPath $WorkRoot) {
  $hasMarker = Test-Path -LiteralPath (Join-Path $WorkRoot $WorkRootMarker) -PathType Leaf
  if (-not $hasMarker) {
    if (@(Get-ChildItem -LiteralPath $WorkRoot -Force).Count -gt 0) {
      throw "-WorkRoot $WorkRoot is not empty and was not created by this script (no $WorkRootMarker file). Choose another folder."
    }
    [IO.File]::WriteAllText((Join-Path $WorkRoot $WorkRootMarker), "Work root of scripts/build_third_party_sources.ps1. Safe to delete.`n")
  }
} else {
  New-Item -ItemType Directory -Path $WorkRoot | Out-Null
  [IO.File]::WriteAllText((Join-Path $WorkRoot $WorkRootMarker), "Work root of scripts/build_third_party_sources.ps1. Safe to delete.`n")
}
if ($Clean) {
  foreach ($old in @(Get-ChildItem -LiteralPath $WorkRoot -Directory -Force)) {
    if (Test-Path -LiteralPath (Join-Path $old.FullName $RunMarker) -PathType Leaf) {
      Write-Host "Deleting earlier run folder $($old.FullName) (-Clean)."
      Remove-Item -LiteralPath $old.FullName -Recurse -Force
    }
  }
}

# A short, new folder per run: Go module paths below it are long.
$Run = Join-Path $WorkRoot ('{0}-{1:x4}' -f (Get-Date -Format 'yyMMdd-HHmmss'), (Get-Random -Maximum 65536))
New-Item -ItemType Directory -Path $Run | Out-Null
[IO.File]::WriteAllText((Join-Path $Run $RunMarker), "Run folder of scripts/build_third_party_sources.ps1. Safe to delete.`n")
$script:LogDir = Join-Path $Run 'logs'
New-EmptyDirectory $script:LogDir
$StageRoot = Join-Path $Run 'stage'
$Stage = Join-Path $StageRoot $BundleName
New-Item -ItemType Directory -Path $Stage -Force | Out-Null

$script:GoEnvironment = [ordered]@{
  GOFLAGS = '-mod=mod'
  # Exactly the recorded Go release: the vendor pins depend on it. A local Go
  # of another version downloads it (verified by the checksum database).
  GOTOOLCHAIN = if ($goToolchain) { $goToolchain } else { 'local' }
  GOENV = 'off'
  GOWORK = 'off'
  GOPROXY = 'https://proxy.golang.org'
  GOSUMDB = 'sum.golang.org'
  GONOSUMDB = $null
  GONOSUMCHECK = $null
  GOPRIVATE = $null
  GONOPROXY = $null
  GOINSECURE = $null
  GOVCS = $null
  GOEXPERIMENT = $null
  GOOS = $null
  GOARCH = $null
  CGO_ENABLED = $null
  GOTELEMETRY = 'off'
  GOPATH = (Join-Path $Run 'go\path')
  GOMODCACHE = if ($GoModCache) { $GoModCache } else { (Join-Path $Run 'go\modcache') }
  GOCACHE = (Join-Path $Run 'go\cache')
  APPDATA = (Join-Path $Run 'go\appdata')
  LOCALAPPDATA = (Join-Path $Run 'go\localappdata')
}
foreach ($folder in @('GOPATH', 'GOMODCACHE', 'GOCACHE', 'APPDATA', 'LOCALAPPDATA')) {
  New-Item -ItemType Directory -Path $script:GoEnvironment[$folder] -Force | Out-Null
}
if ($script:DownloadCachePath) { New-Item -ItemType Directory -Force -Path $script:DownloadCachePath | Out-Null }

$recorded = [System.Collections.Generic.List[string]]::new()
$verification = [System.Collections.Generic.List[string]]::new()
$results = [ordered]@{}
$exitCode = 1
$goVersionUsed = $null
try {
  Write-Step "Bundle $BundleName"
  Write-Note "Manifest: $ManifestPath"
  Write-Note "Run folder: $Run"
  Write-Note ("PowerShell {0}, .NET {1}" -f $PSVersionTable.PSVersion, [Environment]::Version)
  if ($hasGo -or $Runtime) {
    $goVersionUsed = ((Invoke-Go -Directory $Run -Arguments @('env', 'GOVERSION')).Output | Select-Object -Last 1).Trim()
    Write-Note "Go: $goVersionUsed (GOTOOLCHAIN=$($script:GoEnvironment.GOTOOLCHAIN), $($script:Go))"
    if ($goToolchain -and $goVersionUsed -cne $goToolchain) {
      throw "Go reports $goVersionUsed, but the manifest's go_toolchain is $goToolchain. Use a Go that can switch toolchains (Go 1.21 or later) and network access to proxy.golang.org."
    }
  }

  if ($Runtime) {
    Write-Step "Runtime to cross-check: $Runtime"
    Open-Runtime $Runtime
    $expectedArchiveSha = Get-Field $appliesTo 'runtime_archive_sha256'
    if ($script:RuntimeZip -and $expectedArchiveSha) {
      $actual = Get-FileHashText $Runtime
      if ($actual -cne $expectedArchiveSha) { throw "Runtime archive SHA-256 is $actual; the manifest names $expectedArchiveSha." }
      $verification.Add("runtime archive SHA-256 $actual matches applies_to.runtime_archive_sha256")
    }
    $manifestFile = Get-RuntimeFile 'runtime-manifest.json' (Join-Path $Run 'runtime\manifest')
    $runtimeManifest = Get-Content -LiteralPath $manifestFile -Raw | ConvertFrom-Json -AsHashtable
    foreach ($pair in @(@('bundle_revision', 'runtime_bundle_revision'), @('arduino_cli', 'runtime_arduino_cli'), @('renesas_uno_core', 'runtime_renesas_uno_core'))) {
      $expected = Get-Field $appliesTo $pair[1]
      if ($null -ne $expected -and "$($runtimeManifest[$pair[0]])" -cne "$expected") {
        throw "Runtime $($pair[0]) is '$($runtimeManifest[$pair[0]])'; the manifest names '$expected'."
      }
    }
    $verification.Add("runtime-manifest.json: bundle_revision $($runtimeManifest.bundle_revision), arduino_cli $($runtimeManifest.arduino_cli), renesas_uno_core $($runtimeManifest.renesas_uno_core)")
  }

  foreach ($c in $components) {
    $id = $c.id
    Write-Step "$($c.component) $($c.version) [$id]"
    $download = Get-ComponentSource -Component $c -Folder (Join-Path $Run "dl\$id")
    Assert-MagicBytes $download
    $size = (Get-Item -LiteralPath $download).Length
    $hash = Get-FileHashText $download
    $pinned = Get-Field $c 'sha256' ''
    if ($pinned) {
      if ($hash -cne $pinned) { throw "$($c.source_filename) has SHA-256 $hash; the manifest pins $pinned. Stopped." }
      Write-Note "SHA-256 verified: $hash ($size bytes)"
    } elseif ($RecordHashes) {
      $c['sha256'] = $hash
      $recorded.Add("$id sha256 $hash")
      Write-Note "SHA-256 recorded: $hash ($size bytes)"
    } else {
      throw "$id has no pinned sha256. Review the download, then run with -RecordHashes."
    }
    Add-ToDownloadCache -Path $download -Sha256 $hash

    $published = Get-Field $c 'published_checksum'
    if ($published) {
      $algorithm = "$($published.algorithm)".ToUpperInvariant()
      $value = "$($published.value)".ToLowerInvariant()
      $actual = Get-FileHashText $download $algorithm
      if ($actual -cne $value) { throw "$id $algorithm is $actual; upstream publishes $value." }
      $line = "$id upstream $algorithm $value matches"
      if (Get-Field $published 'url') {
        try {
          $checksumFile = Save-Download $published.url (Join-Path $Run "dl\$id-published-checksum") 'checksum.txt'
          $text = (Get-Content -LiteralPath $checksumFile -Raw).ToLowerInvariant()
          if ($text -notmatch "(^|[^0-9a-f])$([regex]::Escape($value))([^0-9a-f]|$)") { throw "$($published.url) does not list $value." }
          $line += " (re-read from $($published.url))"
        } catch {
          if ($_.Exception.Message -like '*does not list*') { throw }
          Write-Warning "Could not re-read $($published.url): $($_.Exception.Message). The pinned value was still checked."
          $line += ' (publisher file not reachable; pinned value checked)'
        }
      }
      $verification.Add($line)
      Write-Note $line
    }

    $git = Get-Field $c 'git'
    if ($git -and (Get-Field $git 'commit')) {
      $commit = Get-GitArchiveCommit $download
      if ($commit -cne $git.commit) { throw "$($c.source_filename) records commit '$commit'; the manifest names $($git.commit)." }
      $verification.Add("$id archive pax header names commit $commit ($($git.ref))")
      Write-Note "git archive commit verified: $commit"
    }

    $targetFolder = if ($c.kind -eq 'license-text') { Join-Path $Stage 'licenses' } else { Join-Path $Stage "sources\$id" }
    New-Item -ItemType Directory -Path $targetFolder -Force | Out-Null
    Copy-Item -LiteralPath $download -Destination (Join-Path $targetFolder $c.source_filename)
    $result = [ordered]@{ Size = $size; Sha256 = $hash; Vendor = $null; VendorSha256 = $null; VendorSize = $null; VendorPins = $null }

    if (Get-Field $c 'go_vendored' $false) {
      $extract = Join-Path $Run "go\src\$id"
      New-EmptyDirectory $extract
      $links = @(Expand-TarGz -Archive $download -Destination $extract)
      foreach ($link in $links) {
        # Go never builds testdata folders; a link anywhere else could change the module.
        if (($link -split '/') -notcontains 'testdata') { throw "$($c.source_filename) has a link outside testdata: $link" }
      }
      if ($links.Count -gt 0) { Write-Note "Skipped $($links.Count) symbolic links in testdata folders (not used by 'go mod vendor')." }
      $tops = @(Get-ChildItem -LiteralPath $extract -Force)
      if ($tops.Count -ne 1 -or -not $tops[0].PSIsContainer) { throw "$($c.source_filename) does not have a single top folder." }
      $moduleRoot = $tops[0].FullName
      if (-not (Test-Path -LiteralPath (Join-Path $moduleRoot 'go.mod'))) { throw "$($c.source_filename) has no go.mod at its top." }
      if (Test-Path -LiteralPath (Join-Path $moduleRoot 'vendor')) { throw "$($c.source_filename) already contains vendor/; refusing to mix trees." }
      Invoke-Go -Directory $moduleRoot -Arguments @('mod', 'vendor') | Out-Null
      Invoke-Go -Directory $moduleRoot -Arguments @('mod', 'verify') | Out-Null
      $vendorDir = Join-Path $moduleRoot 'vendor'
      $modulesTxt = Join-Path $vendorDir 'modules.txt'
      if (-not (Test-Path -LiteralPath $modulesTxt)) { throw "go mod vendor wrote no vendor/modules.txt for $id." }

      # Pin the vendor tree by content (see the help above).
      $pins = Get-VendorTreePins $vendorDir
      [IO.File]::WriteAllText((Join-Path $script:LogDir "$id-vendor-files.txt"), $pins.Listing, [Text.UTF8Encoding]::new($false))
      $vendor = $c['vendor']
      $observed = [ordered]@{
        go_version = $goVersionUsed
        modules = $pins.Modules
        files = $pins.Files
        files_sha256 = $pins.FilesSha256
        modules_txt_sha256 = $pins.ModulesTxtSha256
      }
      foreach ($field in $observed.Keys) {
        $expectedValue = Get-Field $vendor $field
        if ($null -ne $expectedValue -and "$expectedValue" -ne '') {
          if ("$expectedValue" -cne "$($observed[$field])") {
            $hint = switch ($field) {
              'go_version' { "The vendor pins were recorded with $expectedValue; set the manifest's go_toolchain to it." }
              'modules_txt_sha256' { "vendor/modules.txt differs while go_version matches: review $id's go.mod and go.sum." }
              default { "The vendored files differ; compare $(Join-Path $script:LogDir "$id-vendor-files.txt") with the vendor ZIP of the published bundle." }
            }
            throw "$id vendor.$field is $($observed[$field]); the manifest pins $expectedValue. $hint"
          }
        } elseif ($RecordHashes) {
          $vendor[$field] = $observed[$field]
          $recorded.Add("$id vendor.$field $($observed[$field])")
        } else {
          throw "$id has no pinned vendor.$field. Review the vendor tree, then run with -RecordHashes."
        }
      }
      Write-Note "vendor tree verified: $($pins.Modules) modules, $($pins.Files) files, files SHA-256 $($pins.FilesSha256), modules.txt SHA-256 $($pins.ModulesTxtSha256) ($goVersionUsed)"
      $verification.Add("$id vendor tree: $($pins.Modules) modules, $($pins.Files) files, files_sha256 $($pins.FilesSha256), modules_txt_sha256 $($pins.ModulesTxtSha256), $goVersionUsed")

      $vendorName = "$id-$($c.version)-vendor.zip"
      if ($vendorName -notmatch $SafeName) { throw "Unsafe vendor ZIP name: $vendorName" }
      $vendorZip = Join-Path $Run "go\$vendorName"
      $vendorSources = New-DeterministicZip -SourceRoot $vendorDir -Destination $vendorZip -Prefix 'vendor' -Timestamp $ZipTimestamp
      Test-DeterministicZip -Path $vendorZip -Sources $vendorSources -Timestamp $ZipTimestamp
      Copy-Item -LiteralPath $vendorZip -Destination (Join-Path $targetFolder $vendorName)
      $result.Vendor = $vendorName
      $result.VendorSha256 = Get-FileHashText $vendorZip
      $result.VendorSize = (Get-Item -LiteralPath $vendorZip).Length
      $result.VendorPins = $observed
      Write-Note "vendor ZIP: $vendorName ($($result.VendorSize) bytes, SHA-256 $($result.VendorSha256); not pinned: the bytes depend on .NET's Deflate)"

      if ($Runtime) {
        $goSum = @(Get-Content -LiteralPath (Join-Path $moduleRoot 'go.sum'))
        $modulesLines = @(Get-Content -LiteralPath $modulesTxt)
        foreach ($binary in @(Get-Field $c 'go_binaries' @())) {
          $local = Get-RuntimeFile $binary (Join-Path $Run "runtime\$id-$([IO.Path]::GetFileNameWithoutExtension($binary))")
          $info = (Invoke-Go -Directory $Run -Arguments @('version', '-m', $local)).Output
          $revision = ($info | Where-Object { $_ -match '^\s+build\s+vcs\.revision=' } | ForEach-Object { ($_ -split '=', 2)[1].Trim() }) | Select-Object -First 1
          $modified = ($info | Where-Object { $_ -match '^\s+build\s+vcs\.modified=' } | ForEach-Object { ($_ -split '=', 2)[1].Trim() }) | Select-Object -First 1
          if ($git -and $revision -cne $git.commit) { throw "$binary was built from '$revision', not $($git.commit)." }
          if ($modified -cne 'false') { throw "$binary reports vcs.modified=$modified." }
          $checked = 0
          for ($i = 0; $i -lt $info.Count; $i++) {
            $fields = $info[$i].Trim() -split '\s+'
            if ($fields[0] -ne 'dep') { continue }
            $path = $fields[1]; $version = $fields[2]; $sum = if ($fields.Count -ge 4) { $fields[3] } else { $null }
            $expectedModule = "# $path $version"
            if ($i + 1 -lt $info.Count -and ($info[$i + 1].Trim() -split '\s+')[0] -eq '=>') {
              $replacement = $info[$i + 1].Trim() -split '\s+'
              $expectedModule = "# $path $version => $($replacement[1]) $($replacement[2])"
              $sumPath = $replacement[1]; $sumVersion = $replacement[2]; $sum = if ($replacement.Count -ge 4) { $replacement[3] } else { $null }
            } else {
              $sumPath = $path; $sumVersion = $version
            }
            if ($modulesLines -cnotcontains $expectedModule) { throw "$binary links '$expectedModule', which vendor/modules.txt does not list." }
            if ($sum -and $goSum -cnotcontains "$sumPath $sumVersion $sum") { throw "$binary links $sumPath $sumVersion $sum, which go.sum does not list." }
            $checked++
          }
          $line = "$binary`: vcs.revision $revision, vcs.modified=false, $checked linked modules found in vendor/modules.txt and go.sum with the same h1: hash"
          $verification.Add($line)
          Write-Note $line
        }
      }
    }

    if ($Runtime) {
      foreach ($bundled in @(Get-Field $c 'bundled_paths' @())) {
        if (-not (Test-RuntimePath $bundled)) { throw "$id names $bundled, which the runtime does not contain." }
      }
      $verification.Add("$id`: all $(@(Get-Field $c 'bundled_paths' @()).Count) bundled paths present in the runtime")
    }
    $results[$id] = $result
  }

  # --- Manifest copy, README, SHA256SUMS -------------------------------------------
  if ($recorded.Count -gt 0) {
    Write-Step 'Writing recorded values to the manifest'
    $recorded | ForEach-Object { Write-Note "  $_" }
    [IO.File]::WriteAllText($ManifestPath, (ConvertTo-ManifestJson $manifestData), [Text.UTF8Encoding]::new($false))
  }
  [IO.File]::WriteAllText((Join-Path $Stage 'third-party-sources.json'), (ConvertTo-ManifestJson $manifestData), [Text.UTF8Encoding]::new($false))

  Write-Step 'Writing README.txt'
  $r = [System.Collections.Generic.List[string]]::new()
  $appVersions = @($appliesTo.app_versions)
  $versionText = if ($appVersions.Count -le 2) { $appVersions -join ' and ' } else { ($appVersions[0..($appVersions.Count - 2)] -join ', ') + ' and ' + $appVersions[-1] }
  $offerUntil = $zipDate.AddYears($offerYears).ToString('yyyy-MM-dd')
  $inexact = @($components | Where-Object { -not (Get-Field $_ 'exact' $true) })
  $removed = @(Get-Field $manifestData 'removed_components' @())
  $title = Get-Field $manifestData 'title' $BundleName
  $projectName = $project.name
  $maintainers = $project.maintainers
  $licenseOnly = @($components | Where-Object { $_.kind -eq 'license-only' })
  $sourceComponents = @($components | Where-Object { $_.kind -ne 'license-text' })

  $r.Add("$projectName - third-party source bundle")
  $r.Add($BundleName)
  $r.Add('=' * 78)
  $r.Add('')
  Format-Wrapped "$title. Bundle date: $($zipDate.ToString('yyyy-MM-dd'))." 0 | ForEach-Object { $r.Add($_) }
  $r.Add('')
  $r.Add('1. What this bundle is for')
  $r.Add('--------------------------')
  $purpose = "$projectName is $($project.description). Its own code is licensed under $($project.license); its source is at $($project.url). " +
    'Its Windows installers also carry an offline Arduino runtime made of third-party programs under their own licenses. Several of those programs are licensed under the GNU GPL or the GNU LGPL, which entitle you to their corresponding source code. ' +
    'This bundle provides that source'
  if ($licenseOnly.Count -gt 0) {
    $purpose += ", plus the license of each component for which no source is required ($(@($licenseOnly | ForEach-Object { "$($_.component) $($_.version)" }) -join '; '))"
  }
  $purpose += '. It does not change, replace or add to any upstream license.'
  Format-Wrapped $purpose 0 | ForEach-Object { $r.Add($_) }
  $r.Add('')
  $r.Add('Applies to:')
  $r.Add("  $projectName $versionText")
  foreach ($installer in @($appliesTo.installers)) { $r.Add("    $installer") }
  $r.Add("  Runtime archive: $($appliesTo.runtime_archive)")
  $r.Add("    bundle revision $($appliesTo.runtime_bundle_revision): Arduino CLI $($appliesTo.runtime_arduino_cli), Arduino UNO R4 core $($appliesTo.runtime_renesas_uno_core)")
  $r.Add("    SHA-256 $($appliesTo.runtime_archive_sha256)")
  Format-Wrapped ("The application extracts that archive to $($appliesTo.runtime_extracted_to) and runs the tools from there. " +
    'Every "In the installer" path below is relative to the archive root, which is the same as that folder.') 2 | ForEach-Object { $r.Add($_) }
  $r.Add('')
  $r.Add('2. Contents')
  $r.Add('-----------')
  $r.Add('  README.txt                this file')
  $r.Add('  SHA256SUMS.txt            SHA-256 of every other file (sha256sum format)')
  $r.Add('  third-party-sources.json  machine-readable list of the components below')
  $r.Add('  licenses/                 reference copies of the GNU license texts')
  $r.Add('  sources/<component>/      one folder per component (section 3)')
  $r.Add('')
  $r.Add('3. Components')
  $r.Add('-------------')
  $n = 0
  foreach ($c in $sourceComponents) {
    $n++
    $res = $results[$c.id]
    $r.Add('')
    $r.Add("3.$n $($c.component) $($c.version)")
    $r.Add("  License (SPDX):    $($c.license)")
    $paths = @(Get-Field $c 'bundled_paths' @())
    if ($paths.Count -gt 0) {
      $r.Add("  In the installer:  $($paths[0])")
      foreach ($p in ($paths | Select-Object -Skip 1)) { $r.Add("                     $p") }
    }
    $label = if ($c.kind -eq 'license-only') { 'License in bundle:' } else { 'Source in bundle: ' }
    $r.Add("  $label sources/$($c.id)/$($c.source_filename)")
    $r.Add("                     $($res.Size) bytes, SHA-256 $($res.Sha256)")
    if ($res.Vendor) {
      $r.Add("  Go modules:        sources/$($c.id)/$($res.Vendor)")
      $r.Add("                     $($res.VendorSize) bytes, SHA-256 $($res.VendorSha256)")
      $r.Add("                     $($res.VendorPins.modules) modules, $($res.VendorPins.files) files, 'go mod vendor' with $($res.VendorPins.go_version)")
      $r.Add("                     vendor tree SHA-256 $($res.VendorPins.files_sha256) (see section 5)")
    }
    $r.Add("  Downloaded from:   $($c.source_url)")
    $git = Get-Field $c 'git'
    if ($git) { $r.Add("  Git:               $($git.repository) $($git.ref) (commit $($git.commit))") }
    $published = Get-Field $c 'published_checksum'
    if ($published) { $r.Add("  Upstream checksum: $($published.algorithm) $($published.value)") }
    if ($c.kind -eq 'license-only') {
      $r.Add('  Exact source:      not required (permissive license); license included')
    } elseif (Get-Field $c 'exact' $true) {
      $r.Add('  Exact source:      yes')
    } else {
      $r.Add("  Exact source:      NO - closest available source; the exact source is held by $($c.exact_source_holder) (section 7)")
    }
    $buildHint = Get-Field $c 'build_hint'
    if ($buildHint) { Format-Wrapped $buildHint 2 -FirstPrefix 'Upstream build:    ' | ForEach-Object { $r.Add($_) } }
    Format-Wrapped $c.notes 2 -FirstPrefix 'Notes:             ' | ForEach-Object { $r.Add($_) }
  }
  $r.Add('')
  $r.Add('4. Components no longer included')
  $r.Add('--------------------------------')
  if ($removed.Count -eq 0) {
    $r.Add('  None.')
  } else {
    foreach ($item in $removed) {
      $r.Add("  $($item.component) $($item.version): removed in runtime revision $($item.removed_in_revision).")
      Format-Wrapped $item.notes 4 | ForEach-Object { $r.Add($_) }
    }
  }
  $r.Add('')
  $r.Add('5. Clear directions: how this source corresponds to the installed programs')
  $r.Add('--------------------------------------------------------------------------')
  $d = 0
  foreach ($direction in @($manifestData.directions)) {
    $d++
    Format-Wrapped "$($direction.title). $($direction.text)" 2 -FirstPrefix "5.$d " | ForEach-Object { $r.Add($_) }
    $r.Add('')
  }
  $r.Add('6. Licenses')
  $r.Add('-----------')
  $licenseIntro = 'Each source archive keeps its upstream license files (LICENSE, COPYING and similar); they govern the component. The runtime archive keeps the upstream license files of the installed tools in place.'
  $installedNotices = Get-Field $project 'installed_notices' ''
  if ($installedNotices) { $licenseIntro += " $installedNotices" }
  $licenseIntro += ' licenses/ in this bundle holds reference copies:'
  Format-Wrapped $licenseIntro 2 | ForEach-Object { $r.Add($_) }
  foreach ($c in @($components | Where-Object { $_.kind -eq 'license-text' })) {
    $res = $results[$c.id]
    $r.Add("    licenses/$($c.source_filename)  $($c.component)")
    $r.Add("      from $($c.source_url), SHA-256 $($res.Sha256)")
  }
  $licenseNames = @($sourceComponents | ForEach-Object { $_.license } | Sort-Object -Unique)
  $r.Add('  SPDX expressions used above:')
  foreach ($l in $licenseNames) { Format-Wrapped $l 4 | ForEach-Object { $r.Add($_) } }
  $licenseRefs = Get-Field $manifestData 'license_refs' @{}
  $allLicenses = $licenseNames -join ' '
  foreach ($ref in @($licenseRefs.Keys | Sort-Object)) {
    if ($allLicenses.Contains($ref)) { Format-Wrapped "$ref`: $($licenseRefs[$ref])" 2 | ForEach-Object { $r.Add($_) } }
  }
  $r.Add('')
  $r.Add('7. Written offer and source requests')
  $r.Add('------------------------------------')
  $period = "for at least $offerYears years after we last distribute $projectName $versionText, and in any case until at least $offerUntil"
  Format-Wrapped ("WRITTEN OFFER. $maintainers (`"we`") offer, $period, to give any third party, on request, a complete machine-readable copy of the corresponding source code that we hold for the GPL- and LGPL-licensed programs in those installers, as listed in section 3, under the terms of each program's license. " +
    'We provide it by download or on a medium customarily used for software interchange, for no charge beyond our cost of physically performing the distribution. ' +
    'To ask, or to report something missing or wrong, open a "Request corresponding source" issue at:') 2 | ForEach-Object { $r.Add($_) }
  $r.Add("    $requestUrl")
  Format-Wrapped 'Name the release version and the component. We reply in the issue; do not include an email address or other personal information.' 2 | ForEach-Object { $r.Add($_) }
  if ($inexact.Count -gt 0) {
    foreach ($holder in @($inexact | ForEach-Object { $_.exact_source_holder } | Sort-Object -Unique)) {
      $held = @($inexact | Where-Object { $_.exact_source_holder -ceq $holder })
      $subjects = [System.Collections.Generic.List[string]]::new()
      foreach ($c in $held) { if (-not $subjects.Contains($c.offer_subject)) { $subjects.Add($c.offer_subject) } }
      $r.Add('')
      Format-Wrapped ("SOURCE WE DO NOT HOLD. For the programs below, $holder has not published the exact source code of the binaries that these installers carried, and we do not have it:") 2 | ForEach-Object { $r.Add($_) }
      foreach ($subject in $subjects) { Format-Wrapped $subject 4 -FirstPrefix '- ' | ForEach-Object { $r.Add($_) } }
      Format-Wrapped 'What this bundle includes for them instead (marked "Exact source: NO" in section 3):' 2 | ForEach-Object { $r.Add($_) }
      foreach ($c in $held) { Format-Wrapped "$($c.component) $($c.version): sources/$($c.id)/$($c.source_filename)" 4 -FirstPrefix '- ' | ForEach-Object { $r.Add($_) } }
      Format-Wrapped ("The exact build sources of those programs are held by $holder. If you ask for them with the issue form above, we will forward the request to $holder and publish any source $holder provides, as an asset of the affected GitHub Releases or in the issue. " +
        "This applies for the same period as the written offer above.") 2 | ForEach-Object { $r.Add($_) }
    }
  }
  $r.Add('')
  $readme = ($r -join "`r`n") + "`r`n"
  [IO.File]::WriteAllText((Join-Path $Stage 'README.txt'), $readme, [Text.UTF8Encoding]::new($false))

  Write-Step 'Writing SHA256SUMS.txt'
  $stageRootFull = [IO.Path]::GetFullPath($Stage).TrimEnd('\')
  $sumLines = [System.Collections.Generic.List[string]]::new()
  $files = @(Get-ChildItem -LiteralPath $Stage -Recurse -File -Force | ForEach-Object { $_.FullName.Substring($stageRootFull.Length + 1).Replace('\', '/') })
  [Array]::Sort($files, [StringComparer]::Ordinal)
  foreach ($relative in $files) {
    if ($relative -ceq 'SHA256SUMS.txt') { continue }
    $sumLines.Add("$(Get-FileHashText (Join-Path $Stage $relative))  $relative")
  }
  [IO.File]::WriteAllText((Join-Path $Stage 'SHA256SUMS.txt'), (($sumLines -join "`n") + "`n"), [Text.UTF8Encoding]::new($false))

  Write-Step "Writing $BundleName.zip"
  New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
  $tempZip = Join-Path $Run "$BundleName.zip"
  $bundleSources = New-DeterministicZip -SourceRoot $Stage -Destination $tempZip -Prefix $BundleName -Timestamp $ZipTimestamp
  Test-DeterministicZip -Path $tempZip -Sources $bundleSources -Timestamp $ZipTimestamp
  $bundleHash = Get-FileHashText $tempZip
  $bundleSize = (Get-Item -LiteralPath $tempZip).Length
  if ($Force) {
    foreach ($path in @($zipPath, $shaPath)) { if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force } }
  }
  Copy-Item -LiteralPath $tempZip -Destination $zipPath
  if ((Get-FileHashText $zipPath) -cne $bundleHash) { throw "Copy of $zipPath does not match the verified ZIP." }
  [IO.File]::WriteAllText($shaPath, "$bundleHash  $BundleName.zip`n", [Text.UTF8Encoding]::new($false))

  Write-Step 'Done'
  Write-Note "$zipPath"
  Write-Note "  $bundleSize bytes, $($bundleSources.Count) entries, SHA-256 $bundleHash"
  Write-Note "$shaPath"
  if ($verification.Count -gt 0) {
    Write-Note 'Verification:'
    $verification | ForEach-Object { Write-Note "  $_" }
  }
  $exitCode = 0
} finally {
  if ($script:RuntimeZip) { $script:RuntimeZip.Dispose() }
  if ($script:LogDir -and (Test-Path -LiteralPath $script:LogDir)) {
    $logText = ($script:Log -join "`n") + "`n" + "verification:`n" + (($verification | ForEach-Object { "  $_" }) -join "`n") + "`n"
    [IO.File]::WriteAllText((Join-Path $Run 'build-log.txt'), $logText, [Text.UTF8Encoding]::new($false))
    Write-Host "Log: $(Join-Path $Run 'build-log.txt')"
  }
  if ($exitCode -eq 0 -and -not $KeepRunFolder) {
    # Downloads, extracted sources, Go caches and the staging copy are large;
    # the logs and build-log.txt stay for review.
    foreach ($large in @('dl', 'go', 'stage', 'runtime', "$BundleName.zip")) {
      $path = Join-Path $Run $large
      if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Recurse -Force }
    }
  }
}
exit $exitCode
