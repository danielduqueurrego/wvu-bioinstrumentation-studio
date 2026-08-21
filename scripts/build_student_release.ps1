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
$distRoot = Join-Path $projectRoot "dist\WVU-Bioinstrumentation-Studio-$appVersion"
$zipPath = Join-Path $projectRoot "dist\WVU-Bioinstrumentation-Studio-$appVersion-Windows-x64.zip"

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

Set-Location $projectRoot
$dirty = @(git status --short)
if ($dirty.Count -gt 0 -and -not $AllowDirty) {
  throw "Working tree is not clean. Review changes or rerun with -AllowDirty after confirming the intended release scope."
}
if ($dirty.Count -gt 0) {
  Write-Warning 'Building from a dirty working tree by explicit request.'
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

$tauriConfig = Get-Content -LiteralPath $tauriConfigPath -Raw | ConvertFrom-Json
if ($tauriConfig.version -ne $appVersion) {
  throw "Tauri version $($tauriConfig.version) does not match release manifest $appVersion."
}
$packageMetadata = Get-Content -LiteralPath (Join-Path $projectRoot 'package.json') -Raw | ConvertFrom-Json
if ($packageMetadata.version -ne $appVersion) {
  throw "package.json version $($packageMetadata.version) does not match release manifest $appVersion."
}
$packageLockMetadata = Get-Content -LiteralPath (Join-Path $projectRoot 'package-lock.json') -Raw | ConvertFrom-Json -AsHashtable
if ($packageLockMetadata['version'] -ne $appVersion -or $packageLockMetadata['packages']['']['version'] -ne $appVersion) {
  throw 'package-lock.json root versions do not match the release manifest.'
}
$cargoManifestText = Get-Content -LiteralPath (Join-Path $tauriRoot 'Cargo.toml') -Raw
if ($cargoManifestText -notmatch "(?m)^version\s*=\s*`"$([regex]::Escape($appVersion))`"\s*$") {
  throw "Cargo.toml package version does not match release manifest $appVersion."
}
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
if ($runtimeManifest.arduino_cli -ne '1.5.2-rc.1' -or $runtimeManifest.renesas_uno_core -ne '1.6.0') {
  throw 'Unexpected Arduino runtime manifest. Update the reviewed release metadata before building.'
}
$expectedReferenceFirmwareSha256 = $releaseManifest.reference_firmware_source_sha256
if ($expectedReferenceFirmwareSha256 -notmatch '^[0-9a-fA-F]{64}$') {
  throw 'Release manifest is missing the reviewed reference firmware SHA-256 digest.'
}
$referenceFirmwareHash = (Get-FileHash -LiteralPath $referenceFirmwarePath -Algorithm SHA256).Hash
if (-not $referenceFirmwareHash.Equals($expectedReferenceFirmwareSha256, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw "Reference firmware source hash does not match the reviewed manifest: $referenceFirmwareHash"
}
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
  foreach ($entry in @(
    'runtime-manifest.json',
    'arduino-cli.exe',
    'data/package_index.json',
    'data/library_index.json',
    'data/packages/arduino/hardware/renesas_uno/1.6.0/platform.txt',
    'data/packages/arduino/tools/arm-none-eabi-gcc/7-2017q4/bin/arm-none-eabi-g++.exe',
    'data/packages/arduino/tools/bossac/1.9.1-arduino5/bossac.exe',
    'data/packages/builtin/tools/serial-discovery/1.5.2/serial-discovery.exe'
  )) {
    if ($runtimeEntries -notcontains $entry) { throw "Arduino runtime archive is missing: $entry" }
  }
} finally {
  $runtimeZip.Dispose()
}
& (Join-Path $projectRoot 'scripts\audit_bundled_runtime_notices.ps1') -ArchivePath $runtimeArchive

Invoke-NativeChecked 'cargo fmt' { cargo fmt --manifest-path src-tauri\Cargo.toml -- --check }
Invoke-NativeChecked 'cargo check' { cargo check --manifest-path src-tauri\Cargo.toml }
Invoke-NativeChecked 'cargo test' { cargo test --manifest-path src-tauri\Cargo.toml }
Invoke-NativeChecked 'cargo clippy' { cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets -- -D warnings }
Invoke-NativeChecked 'npm run check' { npm run check }
Invoke-NativeChecked 'npm run build' { npm run build }
Invoke-NativeChecked 'npm test' { npm test }
if (-not (Test-Path -LiteralPath $frontendDistPath) -or -not (Test-Path -LiteralPath (Join-Path $frontendDistPath 'index.html'))) {
  throw "Tauri frontendDist is not a built local frontend with index.html: $frontendDistPath"
}
# The Tauri Windows bundlers patch the same release executable. Build them
# serially so MSI and NSIS cannot contend for that file on slower computers.
$releaseBuildStartedUtc = [DateTime]::UtcNow
Invoke-NativeChecked 'Tauri NSIS build' { npm run tauri build }
Invoke-NativeChecked 'Tauri MSI build' { npm run tauri -- build --bundles msi }

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

if (Test-Path -LiteralPath $zipPath) { Remove-Item -LiteralPath $zipPath -Force }
$installerHashTargets = @(
  (Join-Path $distRoot $setupName),
  (Join-Path $distRoot $msiName)
)
$hashLines = foreach ($file in $installerHashTargets) {
  $hash = Get-FileHash -LiteralPath $file -Algorithm SHA256
  "{0}  {1}" -f $hash.Hash.ToLowerInvariant(), (Split-Path -Leaf $file)
}
Set-Content -LiteralPath (Join-Path $distRoot 'SHA256SUMS.txt') -Value $hashLines
Compress-Archive -LiteralPath (Get-ChildItem -LiteralPath $distRoot -File | Select-Object -ExpandProperty FullName) -DestinationPath $zipPath -CompressionLevel Optimal
$zipHash = Get-FileHash -LiteralPath $zipPath -Algorithm SHA256
Add-Content -LiteralPath (Join-Path $distRoot 'SHA256SUMS.txt') -Value ("`n{0}  {1}" -f $zipHash.Hash.ToLowerInvariant(), (Split-Path -Leaf $zipPath))

Write-Host "Student distribution staged: $distRoot"
Write-Host "Distribution ZIP: $zipPath"
Write-Host "NSIS: $(Join-Path $distRoot $setupName)"
Write-Host "MSI: $(Join-Path $distRoot $msiName)"
