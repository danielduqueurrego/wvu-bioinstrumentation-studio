<#
.SYNOPSIS
Builds the student distribution: installers, release folder, ZIP, checksums,
and the third-party source bundle.

.PARAMETER AllowDirty
Build from a working tree with uncommitted changes (an explicitly reviewed
temporary candidate only).

.PARAMETER ThirdPartySourcesZip
A source bundle already built with scripts/build_third_party_sources.ps1 for
this version (WVU-Bioinstrumentation-Studio-<version>-third-party-sources.zip,
with its .sha256 beside it). It is verified against release/third-party-sources.json
(build_third_party_sources.ps1 -VerifyBundle: SHA256SUMS.txt, the manifest copy,
every pinned component and vendor tree) and copied to dist/. Without it, the bundle is built into dist/, which needs
network access and Go (docs/DEVELOPER_GUIDE.md).

.PARAMETER SourcesDownloadCache
Passed to build_third_party_sources.ps1 as -DownloadCache when the bundle is
built here.
#>
#Requires -Version 7.2
[CmdletBinding()]
param(
  [switch]$AllowDirty,
  [string]$ThirdPartySourcesZip,
  [string]$SourcesDownloadCache
)

$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$tauriRoot = Join-Path $projectRoot 'src-tauri'
$runtimeRoot = Join-Path $tauriRoot 'resources'
$runtimeArchive = Join-Path $runtimeRoot 'arduino-runtime.zip'
$manifestPath = Join-Path $runtimeRoot 'arduino-runtime-manifest.json'
$releaseManifestPath = Join-Path $projectRoot 'release\release-manifest.json'
$referenceFirmwarePath = Join-Path $projectRoot 'firmware\reference_unor4wifi\reference_unor4wifi.ino'
$iconSource = Join-Path $projectRoot 'assets\icon.svg'
$tauriConfigPath = Join-Path $tauriRoot 'tauri.conf.json'
$componentNoticePath = Join-Path $runtimeRoot 'licenses\BUNDLED_COMPONENTS.txt'
$bossacLicensePath = Join-Path $runtimeRoot 'licenses\bossac-LICENSE.txt'
$licensePath = Join-Path $projectRoot 'LICENSE'
$noticePath = Join-Path $projectRoot 'NOTICE'
$runtimeEntriesPath = Join-Path $projectRoot 'release\arduino-runtime-entries.json'
$sourcesManifestPath = Join-Path $projectRoot 'release\third-party-sources.json'
# Relative paths are resolved against PowerShell's current location.
if ($ThirdPartySourcesZip) { $ThirdPartySourcesZip = $PSCmdlet.GetUnresolvedProviderPathFromPSPath($ThirdPartySourcesZip) }
if ($SourcesDownloadCache) { $SourcesDownloadCache = $PSCmdlet.GetUnresolvedProviderPathFromPSPath($SourcesDownloadCache) }
$releaseManifest = Get-Content -LiteralPath $releaseManifestPath -Raw | ConvertFrom-Json
$appVersion = [string]$releaseManifest.app_version
if ($appVersion -notmatch '^\d+\.\d+\.\d+$') {
  throw "Release manifest app_version is not semantic version text: $appVersion"
}
$releaseNotesPath = Join-Path $projectRoot "RELEASE_NOTES_$appVersion.md"
$distParent = Join-Path $projectRoot 'dist'
$distRoot = Join-Path $distParent "WVU-Bioinstrumentation-Studio-$appVersion"
$zipPath = Join-Path $distParent "WVU-Bioinstrumentation-Studio-$appVersion-Windows-x64.zip"
$zipChecksumPath = "$zipPath.sha256"
$buildInfoPath = Join-Path $distParent "WVU-Bioinstrumentation-Studio-$appVersion-build-info.txt"
# Published beside the installers (not inside the student ZIP: it is large).
$sourcesBundleName = "WVU-Bioinstrumentation-Studio-$appVersion-third-party-sources"
$sourcesBundlePath = Join-Path $distParent "$sourcesBundleName.zip"
$sourcesBundleChecksumPath = "$sourcesBundlePath.sha256"

function Invoke-NativeChecked {
  param(
    [Parameter(Mandatory = $true)][string]$Label,
    [Parameter(Mandatory = $true)][scriptblock]$Action
  )
  & $Action
  if ($LASTEXITCODE -ne 0) {
    throw "$Label failed with exit code $LASTEXITCODE. Release packaging stopped; no artifact from this run will be staged."
  }
}

# Checksum files use the `sha256sum` layout: "<lowercase hex>  <file name>",
# LF line endings, UTF-8 without a BOM, so `sha256sum -c` works in the folder
# that holds the listed files.
function Write-ChecksumFile {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][string[]]$Files
  )
  $lines = foreach ($file in $Files) {
    $hash = Get-FileHash -LiteralPath $file -Algorithm SHA256
    '{0}  {1}' -f $hash.Hash.ToLowerInvariant(), (Split-Path -Leaf $file)
  }
  [IO.File]::WriteAllText($Path, (($lines -join "`n") + "`n"), [Text.UTF8Encoding]::new($false))
}

Set-Location $projectRoot
$dirty = @(git status --short)
if ($LASTEXITCODE -ne 0) {
  throw "git status failed with exit code $LASTEXITCODE. Release builds must run from a Git work tree."
}
if ($dirty.Count -gt 0 -and -not $AllowDirty) {
  throw "Working tree is not clean. Review changes or rerun with -AllowDirty after confirming the intended release scope."
}
if ($dirty.Count -gt 0) {
  Write-Warning 'Building from a dirty working tree by explicit request.'
}
$sourceCommit = (git rev-parse HEAD)
if ($LASTEXITCODE -ne 0) {
  throw "git rev-parse HEAD failed with exit code $LASTEXITCODE."
}

$diagnosticBinRoot = Join-Path $tauriRoot 'src\bin'
$diagnosticBins = @(Get-ChildItem -LiteralPath $diagnosticBinRoot -Filter '*.rs' -File -ErrorAction SilentlyContinue)
if ($diagnosticBins.Count -gt 0) {
  $names = ($diagnosticBins | ForEach-Object Name) -join ', '
  throw "Diagnostic Rust binaries are present under src-tauri/src/bin ($names). Move them outside the release worktree before packaging; they must never be eligible as the application binary."
}

foreach ($required in @(
  $runtimeArchive,
  $manifestPath,
  $releaseManifestPath,
  $iconSource,
  $tauriConfigPath,
  $referenceFirmwarePath,
  $componentNoticePath,
  $bossacLicensePath,
  $licensePath,
  $noticePath,
  $releaseNotesPath,
  $runtimeEntriesPath,
  $sourcesManifestPath
)) {
  if (-not (Test-Path -LiteralPath $required)) {
    throw "Required pinned release asset is missing: $required"
  }
}

# One shared check, also run by CI: the application version in package.json,
# package-lock.json, Cargo.toml, Cargo.lock, tauri.conf.json and the release
# manifest; the CHANGELOG heading and release notes; the firmware protocol,
# build and device ID; the reference .ino SHA-256; the Arduino CLI and core
# versions in the release manifest and the runtime manifests; the LICENSE and
# NOTICE copies the installers ship from src-tauri/resources/licenses; and the
# custom NSIS template in src-tauri/nsis against the Tauri CLI version.
& (Join-Path $PSScriptRoot 'check_release_consistency.ps1') -ProjectRoot $projectRoot
if ($LASTEXITCODE -ne 0) {
  throw 'Release metadata is inconsistent (listed above). Release packaging stopped.'
}

$tauriConfig = Get-Content -LiteralPath $tauriConfigPath -Raw | ConvertFrom-Json
if ($tauriConfig.bundle.windows.nsis.installMode -ne 'perMachine') {
  throw 'The student NSIS installer must use the reviewed perMachine installation mode.'
}
if ([string]::IsNullOrWhiteSpace($tauriConfig.build.frontendDist) -or $tauriConfig.build.frontendDist -match '^[a-z][a-z0-9+.-]*://') {
  throw 'The production Tauri frontendDist must be a local filesystem path, never a development URL.'
}
if ([string]::IsNullOrWhiteSpace($tauriConfig.build.beforeBuildCommand)) {
  throw 'The production Tauri configuration must define a frontend build command.'
}
if ($tauriConfig.mainBinaryName -ne 'wvu_bioinstrumentation_studio') {
  throw 'The production Tauri configuration must explicitly select wvu_bioinstrumentation_studio as the application binary.'
}
$frontendDistPath = [System.IO.Path]::GetFullPath((Join-Path $tauriRoot $tauriConfig.build.frontendDist))

$runtimeManifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
$expectedRuntimeArchiveSha256 = $releaseManifest.arduino_runtime_archive_sha256
if ($expectedRuntimeArchiveSha256 -notmatch '^[0-9a-fA-F]{64}$') {
  throw 'Release manifest is missing the reviewed Arduino runtime archive SHA-256 digest.'
}
$runtimeArchiveHash = (Get-FileHash -LiteralPath $runtimeArchive -Algorithm SHA256).Hash
if (-not $runtimeArchiveHash.Equals($expectedRuntimeArchiveSha256, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw "Arduino runtime archive hash does not match the reviewed manifest: $runtimeArchiveHash"
}
Add-Type -AssemblyName System.IO.Compression.FileSystem
# What the reviewed archive must and must not contain, shared with
# audit_bundled_runtime_notices.ps1 (which checks the license files too).
$runtimeEntryRules = Get-Content -LiteralPath $runtimeEntriesPath -Raw | ConvertFrom-Json
foreach ($field in @('arduino_cli', 'renesas_uno_core', 'bundle_revision')) {
  if ([string]$runtimeEntryRules.$field -cne [string]$runtimeManifest.$field) {
    throw "release/arduino-runtime-entries.json $field '$($runtimeEntryRules.$field)' does not match arduino-runtime-manifest.json '$($runtimeManifest.$field)'."
  }
}
$runtimeZip = [System.IO.Compression.ZipFile]::OpenRead($runtimeArchive)
try {
  $runtimeEntries = @($runtimeZip.Entries | ForEach-Object FullName)
  foreach ($entry in @($runtimeEntryRules.required_entries)) {
    if ($runtimeEntries -cnotcontains $entry) { throw "Arduino runtime archive is missing: $entry" }
  }
  foreach ($prefix in @($runtimeEntryRules.forbidden_prefixes)) {
    $forbidden = @($runtimeEntries | Where-Object { $_.StartsWith($prefix, [StringComparison]::Ordinal) } | Select-Object -First 1)
    if ($forbidden.Count -gt 0) { throw "Arduino runtime archive contains an entry it must not ship: $($forbidden[0])" }
  }
  # The application accepts an extracted runtime only when its
  # runtime-manifest.json equals arduino-runtime-manifest.json.
  $archivedManifestEntry = $runtimeZip.GetEntry('runtime-manifest.json')
  $reader = [System.IO.StreamReader]::new($archivedManifestEntry.Open(), [System.Text.Encoding]::UTF8)
  try {
    $archivedManifest = $reader.ReadToEnd() | ConvertFrom-Json
  } finally {
    $reader.Dispose()
  }
  foreach ($field in @('runtime_schema', 'arduino_cli', 'renesas_uno_core', 'bundle_revision')) {
    # -cne: the application compares these values exactly, including case.
    if ([string]$archivedManifest.$field -cne [string]$runtimeManifest.$field) {
      throw "The runtime archive's runtime-manifest.json $field '$($archivedManifest.$field)' does not match arduino-runtime-manifest.json '$($runtimeManifest.$field)'."
    }
  }
} finally {
  $runtimeZip.Dispose()
}
& (Join-Path $projectRoot 'scripts\audit_bundled_runtime_notices.ps1') -ArchivePath $runtimeArchive

# The corresponding-source bundle for the GPL/LGPL tools in the runtime is a
# release asset of this version, published beside the installers.
& (Join-Path $PSScriptRoot 'check_third_party_sources.ps1') -ProjectRoot $projectRoot
if ($LASTEXITCODE -ne 0) {
  throw 'The third-party source manifests are inconsistent (listed above). Release packaging stopped.'
}
$sourcesManifest = Get-Content -LiteralPath $sourcesManifestPath -Raw | ConvertFrom-Json
if ($sourcesManifest.bundle_name -cne $sourcesBundleName -or @($sourcesManifest.applies_to.app_versions) -cnotcontains $appVersion) {
  throw "release/third-party-sources.json describes $($sourcesManifest.bundle_name) for $(@($sourcesManifest.applies_to.app_versions) -join ', '), not $sourcesBundleName. Update its bundle_name, applies_to.app_versions and installers for $appVersion."
}
New-Item -ItemType Directory -Force -Path $distParent | Out-Null
foreach ($stale in @($sourcesBundlePath, $sourcesBundleChecksumPath)) {
  if ((Test-Path -LiteralPath $stale) -and -not ($ThirdPartySourcesZip -and $ThirdPartySourcesZip -ieq $sourcesBundlePath)) {
    Remove-Item -LiteralPath $stale -Force
  }
}
if ($ThirdPartySourcesZip) {
  # A bundle built earlier: same name, its own .sha256, and the content the
  # repository's manifest describes.
  if ([IO.Path]::GetFileName($ThirdPartySourcesZip) -cne "$sourcesBundleName.zip") {
    throw "-ThirdPartySourcesZip must be $sourcesBundleName.zip: $ThirdPartySourcesZip"
  }
  $providedChecksum = "$ThirdPartySourcesZip.sha256"
  foreach ($required in @($ThirdPartySourcesZip, $providedChecksum)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "Third-party source bundle file is missing: $required" }
  }
  $recordedHash = ((Get-Content -LiteralPath $providedChecksum -Raw).Trim() -split '\s+')[0].ToLowerInvariant()
  $actualHash = (Get-FileHash -LiteralPath $ThirdPartySourcesZip -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actualHash -cne $recordedHash) { throw "$ThirdPartySourcesZip has SHA-256 $actualHash; its .sha256 file records $recordedHash." }
  # Offline and read only: SHA256SUMS.txt, the manifest copy (it must be
  # release/third-party-sources.json), every component's pinned SHA-256, the
  # vendor-tree pins, and no other file. A sidecar alone would accept a
  # damaged or altered bundle whose .sha256 was rewritten.
  & (Join-Path $PSScriptRoot 'build_third_party_sources.ps1') -Manifest $sourcesManifestPath -VerifyBundle $ThirdPartySourcesZip
  if ($LASTEXITCODE -ne 0) {
    throw "$ThirdPartySourcesZip is not the bundle release/third-party-sources.json describes (listed above). Rebuild it."
  }
  if ($ThirdPartySourcesZip -ine $sourcesBundlePath) {
    Copy-Item -LiteralPath $ThirdPartySourcesZip -Destination $sourcesBundlePath
    Copy-Item -LiteralPath $providedChecksum -Destination $sourcesBundleChecksumPath
  }
} else {
  $sourcesArguments = @{
    Manifest = $sourcesManifestPath
    OutputDir = $distParent
    Runtime = $runtimeArchive
    Force = $true
  }
  if ($SourcesDownloadCache) { $sourcesArguments.DownloadCache = $SourcesDownloadCache }
  & (Join-Path $PSScriptRoot 'build_third_party_sources.ps1') @sourcesArguments
  if ($LASTEXITCODE -ne 0) {
    throw "build_third_party_sources.ps1 failed with exit code $LASTEXITCODE. Release packaging stopped."
  }
}
Write-Host "Third-party source bundle: $sourcesBundlePath"

# npm ci installs exactly package-lock.json (it needs network access or a warm
# npm cache). Cargo commands use --locked so an out-of-date Cargo.lock stops
# the build instead of being rewritten after the clean-tree check.
Invoke-NativeChecked 'npm ci' { npm ci }
Invoke-NativeChecked 'cargo fmt' { cargo fmt --manifest-path src-tauri\Cargo.toml -- --check }
Invoke-NativeChecked 'cargo check' { cargo check --manifest-path src-tauri\Cargo.toml --all-targets --locked }
Invoke-NativeChecked 'cargo test' { cargo test --manifest-path src-tauri\Cargo.toml --locked }
Invoke-NativeChecked 'cargo clippy' { cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets --locked -- -D warnings }
Invoke-NativeChecked 'npm run check' { npm run check }
Invoke-NativeChecked 'npm run build' { npm run build }
Invoke-NativeChecked 'npm test' { npm test }
if (-not (Test-Path -LiteralPath $frontendDistPath) -or -not (Test-Path -LiteralPath (Join-Path $frontendDistPath 'index.html'))) {
  throw "Tauri frontendDist is not a built local frontend with index.html: $frontendDistPath"
}
# The Tauri Windows bundlers patch the same release executable. Build them
# serially so MSI and NSIS cannot contend for that file on slower computers.
# Arguments after the second `--` go to cargo build.
$releaseBuildStartedUtc = [DateTime]::UtcNow
Invoke-NativeChecked 'Tauri NSIS build' { npm run tauri -- build -- --locked }
Invoke-NativeChecked 'Tauri MSI build' { npm run tauri -- build --bundles msi -- --locked }

$releaseExe = Join-Path $tauriRoot 'target\release\wvu_bioinstrumentation_studio.exe'
if (-not (Test-Path -LiteralPath $releaseExe)) {
  throw "Tauri did not produce the expected release executable: $releaseExe"
}
Write-Host "Production frontend: $frontendDistPath"
Write-Host "Production executable (built by Tauri): $releaseExe"
Write-Host 'Manual release smoke test: launch the executable with no Vite dev server running; it must load bundled UI and never request localhost.'

function Get-CurrentBundleArtifact {
  param(
    [Parameter(Mandatory = $true)][string]$Directory,
    [Parameter(Mandatory = $true)][string]$Filter,
    [Parameter(Mandatory = $true)][string]$Label
  )
  $artifacts = @(
    Get-ChildItem -LiteralPath $Directory -Filter $Filter -File -ErrorAction SilentlyContinue |
      Where-Object { $_.LastWriteTimeUtc -ge $releaseBuildStartedUtc }
  )
  if ($artifacts.Count -ne 1) {
    throw "Expected exactly one $Label artifact produced by this run after $releaseBuildStartedUtc; found $($artifacts.Count). Refusing to stage a stale installer."
  }
  return $artifacts[0]
}
$nsis = Get-CurrentBundleArtifact (Join-Path $tauriRoot 'target\release\bundle\nsis') '*-setup.exe' 'NSIS'
$msi = Get-CurrentBundleArtifact (Join-Path $tauriRoot 'target\release\bundle\msi') '*.msi' 'MSI'

if (Test-Path -LiteralPath $distRoot) { Remove-Item -LiteralPath $distRoot -Recurse -Force }
New-Item -ItemType Directory -Path $distRoot -Force | Out-Null
$setupName = "WVU_Bioinstrumentation_Studio_${appVersion}_x64-setup.exe"
$msiName = "WVU_Bioinstrumentation_Studio_${appVersion}_x64.msi"
Copy-Item -LiteralPath $nsis.FullName -Destination (Join-Path $distRoot $setupName)
Copy-Item -LiteralPath $msi.FullName -Destination (Join-Path $distRoot $msiName)
Copy-Item -LiteralPath (Join-Path $projectRoot 'docs\STUDENT_QUICK_START.md') -Destination $distRoot
Copy-Item -LiteralPath $releaseNotesPath -Destination $distRoot
Copy-Item -LiteralPath $licensePath -Destination $distRoot
Copy-Item -LiteralPath $noticePath -Destination $distRoot
Copy-Item -LiteralPath (Join-Path $projectRoot 'docs\THIRD_PARTY_NOTICES.md') -Destination $distRoot

foreach ($stale in @($zipPath, $zipChecksumPath, $buildInfoPath)) {
  if (Test-Path -LiteralPath $stale) { Remove-Item -LiteralPath $stale -Force }
}
# SHA256SUMS.txt lists the installers in the folder and is never changed after
# zipping, so the folder copy and the copy inside the ZIP are identical. The
# ZIP's own checksum goes in a separate <zip>.sha256 beside the ZIP.
Write-ChecksumFile -Path (Join-Path $distRoot 'SHA256SUMS.txt') -Files @(
  (Join-Path $distRoot $setupName),
  (Join-Path $distRoot $msiName)
)
Compress-Archive -LiteralPath (Get-ChildItem -LiteralPath $distRoot -File | Select-Object -ExpandProperty FullName) -DestinationPath $zipPath -CompressionLevel Optimal
Write-ChecksumFile -Path $zipChecksumPath -Files @($zipPath)

# Build provenance for the maintainer; not part of the student ZIP.
$tauriCliVersion = (Get-Content -LiteralPath (Join-Path $projectRoot 'node_modules\@tauri-apps\cli\package.json') -Raw | ConvertFrom-Json).version
$rustcVersion = Invoke-NativeChecked 'rustc --version' { rustc --version }
$cargoVersion = Invoke-NativeChecked 'cargo --version' { cargo --version }
$nodeVersion = Invoke-NativeChecked 'node --version' { node --version }
$npmVersion = Invoke-NativeChecked 'npm --version' { npm --version }
$buildInfo = @(
  "WVU Bioinstrumentation Studio $appVersion",
  "Built (UTC): $($releaseBuildStartedUtc.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'"))",
  "Source commit: $sourceCommit",
  "Working tree: $(if ($dirty.Count -gt 0) { 'dirty (built with -AllowDirty)' } else { 'clean' })",
  "rustc: $rustcVersion",
  "cargo: $cargoVersion",
  "node: $nodeVersion",
  "npm: $npmVersion",
  "@tauri-apps/cli: $tauriCliVersion",
  "PowerShell: $($PSVersionTable.PSVersion)",
  "Arduino runtime: $([System.IO.Path]::GetFileName($runtimeArchive)) SHA-256 $($runtimeArchiveHash.ToLowerInvariant()) (bundle revision $($runtimeManifest.bundle_revision))",
  "Third-party source bundle: $([System.IO.Path]::GetFileName($sourcesBundlePath)) SHA-256 $((Get-FileHash -LiteralPath $sourcesBundlePath -Algorithm SHA256).Hash.ToLowerInvariant())"
)
[IO.File]::WriteAllText($buildInfoPath, (($buildInfo -join "`n") + "`n"), [Text.UTF8Encoding]::new($false))

Write-Host "Student distribution staged: $distRoot"
Write-Host "Distribution ZIP: $zipPath"
Write-Host "ZIP checksum: $zipChecksumPath"
Write-Host "Build information: $buildInfoPath"
Write-Host "NSIS: $(Join-Path $distRoot $setupName)"
Write-Host "MSI: $(Join-Path $distRoot $msiName)"
Write-Host "Third-party source bundle (attach to the release with the installers): $sourcesBundlePath and $sourcesBundleChecksumPath"
