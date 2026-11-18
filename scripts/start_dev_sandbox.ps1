<#
.SYNOPSIS
Runs `npm run tauri dev` with the application's data in a disposable sandbox
folder instead of your real AppData.

.DESCRIPTION
Development only. For the dev application it starts, this script sets:

- LOCALAPPDATA to the sandbox folder. The lab catalog, calibration presets,
  application and firmware logs and the Project folder preference then live
  in <sandbox>\WVU Bioinstrumentation Studio.
- WVU_STUDIO_DEV_LOCAL_DATA to the sandbox folder. Debug builds extract the
  Arduino runtime to <sandbox>\WVU-BMEG\rt instead of the real
  %LOCALAPPDATA%\WVU-BMEG\rt (that path comes from the Windows Known Folder
  API, which ignores LOCALAPPDATA). Release builds ignore this variable.
- With -RemoteDebuggingPort, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS to
  --remote-debugging-port=<port>, so DevTools or browser automation can attach
  to the WebView at http://localhost:<port>.

The first run also writes <sandbox>\WVU Bioinstrumentation Studio\
project-folder.json, which points the Project folder at <sandbox>\recordings.
An existing file is left alone. The variables get their previous values back
when the dev application exits.

Not redirected: WebView2's own browser data (for example localStorage), which
Tauri also places through the Known Folder API. Delete the sandbox folder to
start fresh; .gitignore excludes the default .dev-sandbox folder.

Tauri needs src-tauri\resources\arduino-runtime.zip. With the CI placeholder
archive (see .github/workflows/ci.yml) the Simulator, labs and recordings work,
but firmware restore cannot.

.PARAMETER SandboxRoot
Absolute path of the sandbox folder. Defaults to <repository>\.dev-sandbox.

.PARAMETER RemoteDebuggingPort
Optional WebView2 remote-debugging port (1024-65535).

.EXAMPLE
pwsh -NoProfile -File scripts\start_dev_sandbox.ps1 -RemoteDebuggingPort 9222
#>
[CmdletBinding()]
param(
  [string]$SandboxRoot,
  [ValidateRange(1024, 65535)]
  [int]$RemoteDebuggingPort
)

$ErrorActionPreference = 'Stop'
# Resolved here, not as parameter defaults: Windows PowerShell 5.1 leaves
# $PSScriptRoot empty in parameter defaults when started with -File.
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
if ([string]::IsNullOrWhiteSpace($SandboxRoot)) {
  $SandboxRoot = Join-Path $repositoryRoot '.dev-sandbox'
}
if (-not [IO.Path]::IsPathRooted($SandboxRoot)) {
  throw "SandboxRoot must be an absolute path: $SandboxRoot"
}
$SandboxRoot = [IO.Path]::GetFullPath($SandboxRoot)

if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
  throw 'npm was not found. Install Node.js 22 or later (see docs/DEVELOPER_GUIDE.md).'
}
$runtimeArchive = Join-Path $repositoryRoot 'src-tauri\resources\arduino-runtime.zip'
if (-not (Test-Path -LiteralPath $runtimeArchive -PathType Leaf)) {
  throw "Missing $runtimeArchive. Copy the real archive or create the CI placeholder described in .github/workflows/ci.yml."
}
if (Get-Process -Name 'wvu_bioinstrumentation_studio' -ErrorAction SilentlyContinue) {
  Write-Warning 'WVU Bioinstrumentation Studio is already running. Use one application per board: two instances compete for the serial port.'
}

$appDataFolder = Join-Path $SandboxRoot 'WVU Bioinstrumentation Studio'
$recordingsFolder = Join-Path $SandboxRoot 'recordings'
New-Item -ItemType Directory -Force -Path $appDataFolder, $recordingsFolder | Out-Null
$projectFolderSetting = Join-Path $appDataFolder 'project-folder.json'
if (-not (Test-Path -LiteralPath $projectFolderSetting)) {
  $json = [pscustomobject]@{ project_folder = $recordingsFolder } | ConvertTo-Json
  [IO.File]::WriteAllText($projectFolderSetting, $json, [Text.UTF8Encoding]::new($false))
}

$overrides = [ordered]@{
  LOCALAPPDATA = $SandboxRoot
  WVU_STUDIO_DEV_LOCAL_DATA = $SandboxRoot
}
if ($PSBoundParameters.ContainsKey('RemoteDebuggingPort')) {
  $overrides['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS'] = "--remote-debugging-port=$RemoteDebuggingPort"
}
$previous = @{}
foreach ($name in $overrides.Keys) {
  $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}

Write-Host "Sandbox:        $SandboxRoot"
Write-Host "Project folder: $recordingsFolder (from $projectFolderSetting)"
if ($overrides.Contains('WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS')) {
  Write-Host "WebView2 remote debugging: http://localhost:$RemoteDebuggingPort"
}

$exitCode = 1
Push-Location -LiteralPath $repositoryRoot
try {
  foreach ($name in $overrides.Keys) {
    [Environment]::SetEnvironmentVariable($name, $overrides[$name], 'Process')
  }
  npm run tauri dev
  $exitCode = $LASTEXITCODE
} finally {
  Pop-Location
  foreach ($name in $overrides.Keys) {
    # $null removes a variable that was not set before.
    [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process')
  }
}
exit $exitCode
