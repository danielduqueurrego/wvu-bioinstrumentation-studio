#Requires -Version 7.0
[CmdletBinding()]
param(
  [switch]$AllowDirty
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
  $releaseNotesPath
)) {
  if (-not (Test-Path -LiteralPath $required)) {
    throw "Required pinned release asset is missing: $required"
  }
}

# One shared check, also run by CI: the application version in package.json,
# package-lock.json, Cargo.toml, Cargo.lock, tauri.conf.json and the release
# manifest; the CHANGELOG heading and release notes; the firmware protocol,
# build and device ID; the reference .ino SHA-256; and the Arduino CLI and core
# versions in the release manifest and the runtime manifests.
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
$runtimeZip = [System.IO.Compression.ZipFile]::OpenRead($runtimeArchive)
try {
  $runtimeEntries = @($runtimeZip.Entries | ForEach-Object FullName)
  $coreVersion = [string]$releaseManifest.arduino_core
  foreach ($entry in @(
    'runtime-manifest.json',
    'arduino-cli.exe',
    'data/package_index.json',
    'data/library_index.json',
    "data/packages/arduino/hardware/renesas_uno/$coreVersion/platform.txt",
    'data/packages/arduino/tools/arm-none-eabi-gcc/7-2017q4/bin/arm-none-eabi-g++.exe',
    'data/packages/arduino/tools/bossac/1.9.1-arduino5/bossac.exe',
    'data/packages/builtin/tools/serial-discovery/1.5.2/serial-discovery.exe'
  )) {
    if ($runtimeEntries -notcontains $entry) { throw "Arduino runtime archive is missing: $entry" }
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
Copy-Item -LiteralPath (Join-Path $projectRoot 'LICENSE') -Destination $distRoot
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
  "PowerShell: $($PSVersionTable.PSVersion)"
)
[IO.File]::WriteAllText($buildInfoPath, (($buildInfo -join "`n") + "`n"), [Text.UTF8Encoding]::new($false))

Write-Host "Student distribution staged: $distRoot"
Write-Host "Distribution ZIP: $zipPath"
Write-Host "ZIP checksum: $zipChecksumPath"
Write-Host "Build information: $buildInfoPath"
Write-Host "NSIS: $(Join-Path $distRoot $setupName)"
Write-Host "MSI: $(Join-Path $distRoot $msiName)"
