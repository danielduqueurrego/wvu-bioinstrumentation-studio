$ErrorActionPreference = "Continue"
$report = Join-Path $PSScriptRoot "..\environment-report.txt"

function Section($name) {
    "`n===== $name =====" | Tee-Object -FilePath $report -Append
}

function Check-Command($name, $versionArgs) {
    Section $name
    $cmd = Get-Command $name -ErrorAction SilentlyContinue
    if (-not $cmd) {
        "NOT FOUND" | Tee-Object -FilePath $report -Append
        return
    }
    "Path: $($cmd.Source)" | Tee-Object -FilePath $report -Append
    try {
        & $name @versionArgs 2>&1 | Out-String | Tee-Object -FilePath $report -Append
    } catch {
        "ERROR: $($_.Exception.Message)" | Tee-Object -FilePath $report -Append
    }
}

"Environment report generated: $(Get-Date -Format o)" | Set-Content -Path $report -Encoding UTF8

Section "Windows"
Get-ComputerInfo | Select-Object WindowsProductName, WindowsVersion, OsBuildNumber, CsSystemType |
    Format-List | Out-String | Tee-Object -FilePath $report -Append

Check-Command "git" @("--version")
Check-Command "arduino-cli" @("version")
Check-Command "rustc" @("--version")
Check-Command "cargo" @("--version")
Check-Command "node" @("--version")
Check-Command "npm" @("--version")
Check-Command "code" @("--version")

Section "Arduino board list"
if (Get-Command arduino-cli -ErrorAction SilentlyContinue) {
    arduino-cli board list 2>&1 | Out-String | Tee-Object -FilePath $report -Append
}

Section "Arduino cores"
if (Get-Command arduino-cli -ErrorAction SilentlyContinue) {
    arduino-cli core list 2>&1 | Out-String | Tee-Object -FilePath $report -Append
}

Section "Serial ports"
Get-CimInstance Win32_SerialPort -ErrorAction SilentlyContinue |
    Select-Object DeviceID, Name, PNPDeviceID |
    Format-Table -AutoSize | Out-String | Tee-Object -FilePath $report -Append

Section "WebView2"
$webviewPaths = @(
    "${env:ProgramFiles(x86)}\Microsoft\EdgeWebView\Application",
    "${env:ProgramFiles}\Microsoft\EdgeWebView\Application"
)
$foundWebView = $false
foreach ($p in $webviewPaths) {
    if (Test-Path $p) {
        "Found: $p" | Tee-Object -FilePath $report -Append
        $foundWebView = $true
    }
}
if (-not $foundWebView) {
    "WebView2 installation directory not detected by this simple check." |
        Tee-Object -FilePath $report -Append
}

"`nReport written to: $report"
