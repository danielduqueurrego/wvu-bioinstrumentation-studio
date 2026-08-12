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
$iconSource = Join-Path $projectRoot 'assets\icon.svg'
$tauriConfigPath = Join-Path $tauriRoot 'tauri.conf.json'
$distRoot = Join-Path $projectRoot 'dist\WVU-Bioinstrumentation-Studio-1.0.0'
$zipPath = Join-Path $projectRoot 'dist\WVU-Bioinstrumentation-Studio-1.0.0-Windows-x64.zip'

Set-Location $projectRoot
$dirty = @(git status --short)
if ($dirty.Count -gt 0 -and -not $AllowDirty) {
  throw "Working tree is not clean. Review changes or rerun with -AllowDirty after confirming the intended release scope."
}
if ($dirty.Count -gt 0) {
  Write-Warning 'Building from a dirty working tree by explicit request.'
}

foreach ($required in @(
  $runtimeArchive,
  $manifestPath,
  $releaseManifestPath,
  $iconSource,
  $tauriConfigPath
)) {
  if (-not (Test-Path -LiteralPath $required)) {
    throw "Required pinned release asset is missing: $required"
  }
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
$frontendDistPath = [System.IO.Path]::GetFullPath((Join-Path $tauriRoot $tauriConfig.build.frontendDist))

$runtimeManifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ($runtimeManifest.arduino_cli -ne '1.5.2-rc.1' -or $runtimeManifest.renesas_uno_core -ne '1.6.0') {
  throw 'Unexpected Arduino runtime manifest. Update the reviewed release metadata before building.'
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

& cargo fmt --manifest-path src-tauri\Cargo.toml -- --check
& cargo check --manifest-path src-tauri\Cargo.toml
& cargo test --manifest-path src-tauri\Cargo.toml
& cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets -- -D warnings
& npm run check
& npm test
& npm run build
if (-not (Test-Path -LiteralPath $frontendDistPath) -or -not (Test-Path -LiteralPath (Join-Path $frontendDistPath 'index.html'))) {
  throw "Tauri frontendDist is not a built local frontend with index.html: $frontendDistPath"
}
# The Tauri Windows bundlers patch the same release executable. Build them
# serially so MSI and NSIS cannot contend for that file on slower computers.
& npm run tauri build
& npm run tauri -- build --bundles msi

$releaseExe = Join-Path $tauriRoot 'target\release\wvu_bioinstrumentation_studio.exe'
if (-not (Test-Path -LiteralPath $releaseExe)) {
  throw "Tauri did not produce the expected release executable: $releaseExe"
}
Write-Host "Production frontend: $frontendDistPath"
Write-Host "Production executable (built by Tauri): $releaseExe"
Write-Host 'Manual release smoke test: launch the executable with no Vite dev server running; it must load bundled UI and never request localhost.'

$nsis = Get-ChildItem -LiteralPath (Join-Path $tauriRoot 'target\release\bundle\nsis') -Filter '*-setup.exe' -File |
  Sort-Object LastWriteTime -Descending | Select-Object -First 1
$msi = Get-ChildItem -LiteralPath (Join-Path $tauriRoot 'target\release\bundle\msi') -Filter '*.msi' -File |
  Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $nsis -or -not $msi) {
  throw 'Tauri build completed without both NSIS and MSI installers.'
}

if (Test-Path -LiteralPath $distRoot) { Remove-Item -LiteralPath $distRoot -Recurse -Force }
New-Item -ItemType Directory -Path $distRoot -Force | Out-Null
$setupName = 'WVU_Bioinstrumentation_Studio_1.0.0_x64-setup.exe'
$msiName = 'WVU_Bioinstrumentation_Studio_1.0.0_x64.msi'
Copy-Item -LiteralPath $nsis.FullName -Destination (Join-Path $distRoot $setupName)
Copy-Item -LiteralPath $msi.FullName -Destination (Join-Path $distRoot $msiName)
Copy-Item -LiteralPath (Join-Path $projectRoot 'docs\STUDENT_QUICK_START.md') -Destination $distRoot
Copy-Item -LiteralPath (Join-Path $projectRoot 'RELEASE_NOTES_1.0.0.md') -Destination $distRoot

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
