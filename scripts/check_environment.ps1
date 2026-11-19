<#
.SYNOPSIS
Checks a Windows computer for the tools needed to develop and release
WVU Bioinstrumentation Studio.

.DESCRIPTION
Read-only: it installs nothing and changes nothing. Each line is OK, WARN or
FAIL. FAIL means development builds cannot work; WARN means something only
releases or hardware work need. The exit code is 1 when any check fails.

Checked: Git; Node.js 22 or later with npm; rustup with the toolchain pinned in
rust-toolchain.toml (cargo installs it on first use); the Visual Studio C++
build tools (MSVC) and a Windows 10/11 SDK; the WebView2 runtime; PowerShell 7
(required by scripts/build_student_release.ps1); and whether
src-tauri/resources/arduino-runtime.zip is the reviewed archive, a CI
placeholder, or missing.

Runs in Windows PowerShell 5.1 as well as PowerShell 7, so it works on a new
computer before PowerShell 7 is installed.

.PARAMETER ReportPath
Optional file that receives a copy of the results.
#>
[CmdletBinding()]
param(
  [string]$ReportPath
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$results = New-Object 'System.Collections.Generic.List[object]'

function Add-Result {
  param(
    [Parameter(Mandatory = $true)][ValidateSet('OK', 'WARN', 'FAIL')][string]$Status,
    [Parameter(Mandatory = $true)][string]$Name,
    [Parameter(Mandatory = $true)][string]$Detail
  )
  $results.Add([pscustomobject]@{ Status = $Status; Name = $Name; Detail = $Detail })
}

# Runs a program and returns its trimmed output, or $null when it is missing or
# fails. Never throws.
function Invoke-Probe {
  param(
    [Parameter(Mandatory = $true)][string]$Command,
    [string[]]$Arguments = @()
  )
  if (-not (Get-Command $Command -ErrorAction SilentlyContinue)) { return $null }
  # Windows PowerShell 5.1 turns redirected stderr into a terminating error
  # under 'Stop'; rustup, for one, prints informational lines there.
  $ErrorActionPreference = 'Continue'
  try {
    $output = & $Command @Arguments 2>$null
    if ($LASTEXITCODE -ne 0) { return $null }
    return (($output | Out-String).Trim())
  } catch {
    return $null
  }
}

function Get-RegistryValue {
  param([string[]]$Paths, [string]$Name)
  foreach ($path in $Paths) {
    $item = Get-ItemProperty -LiteralPath $path -Name $Name -ErrorAction SilentlyContinue
    if ($null -ne $item -and -not [string]::IsNullOrWhiteSpace([string]$item.$Name)) {
      return [string]$item.$Name
    }
  }
  return $null
}

# --- Windows -----------------------------------------------------------------

$os = Get-CimInstance -ClassName Win32_OperatingSystem -ErrorAction SilentlyContinue
if ($null -ne $os) {
  Add-Result 'OK' 'Windows' "$($os.Caption) $($os.Version) ($($os.OSArchitecture))"
} else {
  Add-Result 'WARN' 'Windows' 'Could not read the Windows version.'
}

# --- Git -----------------------------------------------------------------------

$git = Invoke-Probe 'git' @('--version')
if ($git) { Add-Result 'OK' 'Git' $git } else { Add-Result 'FAIL' 'Git' 'git was not found. Install Git for Windows.' }

# --- Node.js and npm ---------------------------------------------------------

$node = Invoke-Probe 'node' @('--version')
if (-not $node) {
  Add-Result 'FAIL' 'Node.js' 'node was not found. Install Node.js 22 or later (CI uses 22).'
} elseif ($node -match '^v(\d+)\.' -and [int]$Matches[1] -ge 22) {
  Add-Result 'OK' 'Node.js' $node
} else {
  Add-Result 'FAIL' 'Node.js' "$node is too old. package.json requires Node.js 22 or later."
}
$npm = Invoke-Probe 'npm' @('--version')
if ($npm) { Add-Result 'OK' 'npm' $npm } else { Add-Result 'FAIL' 'npm' 'npm was not found. It is installed with Node.js.' }

# --- Rust ---------------------------------------------------------------------

$toolchainFile = Join-Path $repositoryRoot 'rust-toolchain.toml'
$pinnedChannel = $null
if (Test-Path -LiteralPath $toolchainFile -PathType Leaf) {
  $toolchainText = [IO.File]::ReadAllText($toolchainFile)
  if ($toolchainText -match '(?m)^\s*channel\s*=\s*"([^"]+)"') { $pinnedChannel = $Matches[1] }
}
$rustup = Invoke-Probe 'rustup' @('--version')
if (-not $rustup) {
  Add-Result 'FAIL' 'rustup' 'rustup was not found. Install Rust with rustup (https://rustup.rs) and the MSVC host.'
} else {
  Add-Result 'OK' 'rustup' (($rustup -split "`r?`n")[0])
  # `rustup toolchain list` never installs anything, unlike running cargo here.
  $toolchains = @((Invoke-Probe 'rustup' @('toolchain', 'list')) -split "`r?`n" | Where-Object { $_ })
  if (-not $pinnedChannel) {
    Add-Result 'FAIL' 'Rust toolchain' 'rust-toolchain.toml is missing or has no channel.'
  } else {
    $installed = @($toolchains | Where-Object { $_ -like "$pinnedChannel-*" })
    if ($installed.Count -eq 0) {
      Add-Result 'WARN' 'Rust toolchain' "$pinnedChannel (from rust-toolchain.toml) is not installed yet. The first cargo command in this repository installs it, or run: rustup toolchain install"
    } elseif (@($installed | Where-Object { $_ -match '-pc-windows-msvc' }).Count -eq 0) {
      Add-Result 'FAIL' 'Rust toolchain' "$pinnedChannel is installed only for a non-MSVC host ($($installed -join ', ')). Use the x86_64-pc-windows-msvc host."
    } else {
      Add-Result 'OK' 'Rust toolchain' "$($installed[0] -replace '\s+\(.*\)$', '') (pinned by rust-toolchain.toml)"
    }
  }
}

# --- MSVC build tools and Windows SDK ----------------------------------------

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$msvc = $null
if (Test-Path -LiteralPath $vswhere -PathType Leaf) {
  $msvc = Invoke-Probe $vswhere @('-latest', '-products', '*', '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'displayName')
}
if ($msvc) {
  Add-Result 'OK' 'MSVC C++ build tools' $msvc
} else {
  Add-Result 'FAIL' 'MSVC C++ build tools' 'Not found. Install Visual Studio Build Tools with "Desktop development with C++" (MSVC x64/x86 build tools).'
}

$sdkVersion = Get-RegistryValue @(
  'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Microsoft SDKs\Windows\v10.0',
  'HKLM:\SOFTWARE\Microsoft\Microsoft SDKs\Windows\v10.0'
) 'ProductVersion'
if ($sdkVersion) {
  Add-Result 'OK' 'Windows SDK' "10.0 ($sdkVersion)"
} else {
  Add-Result 'FAIL' 'Windows SDK' 'No Windows 10/11 SDK found. Add a Windows SDK in the Visual Studio Installer.'
}

# --- WebView2 -----------------------------------------------------------------

$webViewKey = 'Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
$webView = Get-RegistryValue @(
  "HKLM:\SOFTWARE\WOW6432Node\$webViewKey",
  "HKLM:\SOFTWARE\$webViewKey",
  "HKCU:\SOFTWARE\$webViewKey"
) 'pv'
if ($webView -and $webView -ne '0.0.0.0') {
  Add-Result 'OK' 'WebView2 runtime' $webView
} else {
  Add-Result 'FAIL' 'WebView2 runtime' 'Not found. Install the Microsoft Edge WebView2 Evergreen runtime.'
}

# --- PowerShell 7 -------------------------------------------------------------

$pwsh = Invoke-Probe 'pwsh' @('-NoLogo', '-NoProfile', '-NonInteractive', '-Command', '$PSVersionTable.PSVersion.ToString()')
if ($pwsh -and $pwsh -match '^(\d+)\.' -and [int]$Matches[1] -ge 7) {
  Add-Result 'OK' 'PowerShell 7' $pwsh
} else {
  Add-Result 'WARN' 'PowerShell 7' 'pwsh 7 was not found. scripts/build_student_release.ps1 requires it.'
}

# --- Arduino runtime archive --------------------------------------------------

$runtimeArchive = Join-Path $repositoryRoot 'src-tauri\resources\arduino-runtime.zip'
$expectedRuntimeHash = $null
$releaseManifestPath = Join-Path $repositoryRoot 'release\release-manifest.json'
if (Test-Path -LiteralPath $releaseManifestPath -PathType Leaf) {
  $expectedRuntimeHash = ([IO.File]::ReadAllText($releaseManifestPath) | ConvertFrom-Json).arduino_runtime_archive_sha256
}
if (-not (Test-Path -LiteralPath $runtimeArchive -PathType Leaf)) {
  Add-Result 'WARN' 'Arduino runtime archive' 'src-tauri\resources\arduino-runtime.zip is missing. Cargo checks need the CI placeholder (see .github/workflows/ci.yml); firmware restore and releases need the reviewed archive.'
} else {
  $runtimeHash = (Get-FileHash -LiteralPath $runtimeArchive -Algorithm SHA256).Hash
  if ($expectedRuntimeHash -and $runtimeHash -ieq $expectedRuntimeHash) {
    Add-Result 'OK' 'Arduino runtime archive' 'The reviewed archive (SHA-256 matches release/release-manifest.json).'
  } else {
    Add-Result 'WARN' 'Arduino runtime archive' 'Present but not the reviewed archive (for example the CI placeholder). Source checks work; firmware restore and release packaging do not.'
  }
}

# --- Report -------------------------------------------------------------------

$lines = foreach ($result in $results) {
  '[{0,-4}] {1}: {2}' -f $result.Status, $result.Name, $result.Detail
}
foreach ($index in 0..($lines.Count - 1)) {
  $color = switch ($results[$index].Status) { 'OK' { 'Green' } 'WARN' { 'Yellow' } default { 'Red' } }
  Write-Host $lines[$index] -ForegroundColor $color
}
if ($ReportPath) {
  $header = "WVU Bioinstrumentation Studio environment check, $(Get-Date -Format o)"
  [IO.File]::WriteAllText(
    $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($ReportPath),
    (@($header) + $lines -join [Environment]::NewLine) + [Environment]::NewLine,
    (New-Object System.Text.UTF8Encoding($false))
  )
  Write-Host "Report written to $ReportPath"
}
$failed = @($results | Where-Object { $_.Status -eq 'FAIL' }).Count
if ($failed -gt 0) {
  Write-Host "$failed required item(s) missing. See docs/DEVELOPER_GUIDE.md." -ForegroundColor Red
  exit 1
}
exit 0
