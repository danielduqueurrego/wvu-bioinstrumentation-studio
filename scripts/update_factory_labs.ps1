<#
.SYNOPSIS
Checks, re-hashes, or promotes the factory (built-in) course lab files in profiles/.

.DESCRIPTION
Runs the maintainer tool src-tauri/examples/factory_labs.rs with
`cargo run --locked --example factory_labs`. The first run compiles the
application library, which takes a few minutes.

  check
      Validates every profiles/*.profile.json (and profiles/legacy/): the
      lab rules, its canonical hash, version MAJOR.MINOR.0, source built_in,
      status locked, that it is compiled into the application,
      profiles/factory-labs.lock.json, and that every published version is
      still held by a file.
  fix <file> [--bump-minor]
      Re-hashes a factory lab file in profiles/ after a hand edit and records
      its version in the lock. A version already in the lock never changes;
      --bump-minor (or -BumpMinor) publishes the change as the next MINOR
      version (X.Y+1.0).
  promote <exported.lab.json> [--name <file-name>]
      Turns a lab exported from Lab Manager into a factory lab file with the
      next factory version (one MINOR above the published versions and the
      export's own version; a new lab exported as 1.0.x becomes 1.1.0),
      appends it to the lock, and prints the line that compiles it in.
      --name (or -Name) chooses the file name of a new lab.

fix and promote keep the published version a file held as
profiles/legacy/<name>_<version>.profile.json, from the file or, when it has
already changed, from the copy committed in Git (HEAD).

See docs/LAB_CONFIGURATION.md, "How to change a built-in lab". Works in
PowerShell 7 and Windows PowerShell 5.1. Teaching use only; not a medical
device.

Exit codes: 0 success, 1 problems found or change refused, 2 usage error
(or the tool could not be built or started).

.EXAMPLE
pwsh -NoProfile -File scripts\update_factory_labs.ps1 check

.EXAMPLE
pwsh -NoProfile -File scripts\update_factory_labs.ps1 fix profiles\ecg_course_capture.profile.json --bump-minor

.EXAMPLE
pwsh -NoProfile -File scripts\update_factory_labs.ps1 promote "$HOME\Desktop\wvu.bmeg420l.ecg.course.capture.v1_1.0.1.lab.json"
#>
param(
  [Parameter(Position = 0)][string]$Command,
  [Parameter(Position = 1, ValueFromRemainingArguments = $true)][string[]]$Arguments
)

$ErrorActionPreference = 'Stop'

$usage = @'
Usage:
  scripts\update_factory_labs.ps1 check
  scripts\update_factory_labs.ps1 fix <profiles\name.profile.json> [--bump-minor]
  scripts\update_factory_labs.ps1 promote <exported.lab.json> [--name <file-name>]

  check    Validate profiles\*.profile.json, the compiled-in list, and
           profiles\factory-labs.lock.json.
  fix      Re-hash a factory lab file after a hand edit. A published version
           never changes: --bump-minor publishes the change as X.Y+1.0.
  promote  Turn a Lab Manager export into a factory lab file (next MINOR
           version; a new lab exported as 1.0.x becomes 1.1.0) and print the
           line to compile it in.

fix and promote keep a replaced published version in profiles\legacy\.

Exit codes: 0 success, 1 problems found or change refused, 2 usage error.
See docs\LAB_CONFIGURATION.md, "How to change a built-in lab".
'@

function Exit-Usage([string]$Message) {
  if ($Message) { [Console]::Error.WriteLine("error: $Message") }
  [Console]::Error.WriteLine($usage)
  exit 2
}

$helpWords = @('help', '-h', '--help', '-help', '/?')
$rest = @()
if ($null -ne $Arguments) { $rest = @($Arguments) }
if (-not $Command) {
  # A leading option such as --help is not bound to -Command.
  if ($rest.Count -gt 0 -and $helpWords -contains $rest[0]) {
    Write-Output $usage
    exit 0
  }
  Exit-Usage 'choose check, fix, or promote.'
}
if ($helpWords -contains $Command) {
  Write-Output $usage
  exit 0
}
$Command = $Command.ToLowerInvariant()
if (@('check', 'fix', 'promote') -notcontains $Command) {
  Exit-Usage "unknown command '$Command'."
}

# Translate the PowerShell-style switches and resolve the file argument
# against the caller's folder, so the tool reports a missing file clearly.
$toolArguments = New-Object System.Collections.Generic.List[string]
$toolArguments.Add($Command)
$fileArgument = $null
for ($index = 0; $index -lt $rest.Count; $index++) {
  $argument = $rest[$index]
  switch -Regex ($argument) {
    '^(--bump-minor|-BumpMinor)$' {
      if ($Command -ne 'fix') { Exit-Usage "$argument applies only to fix." }
      $toolArguments.Add('--bump-minor')
      continue
    }
    '^(--name|-Name)$' {
      if ($Command -ne 'promote') { Exit-Usage "$argument applies only to promote." }
      if ($index + 1 -ge $rest.Count) { Exit-Usage "$argument needs a file name." }
      $index++
      $toolArguments.Add('--name')
      $toolArguments.Add($rest[$index])
      continue
    }
    '^-' {
      Exit-Usage "unknown option '$argument'."
    }
    default {
      if ($null -ne $fileArgument) { Exit-Usage "unexpected argument '$argument'." }
      $fileArgument = $argument
    }
  }
}

if ($Command -eq 'check') {
  if ($null -ne $fileArgument) { Exit-Usage 'check takes no file.' }
} else {
  if ($null -eq $fileArgument) { Exit-Usage "$Command needs a file." }
  if (-not (Test-Path -LiteralPath $fileArgument -PathType Leaf)) {
    [Console]::Error.WriteLine("error: file not found: $fileArgument")
    exit 1
  }
  $toolArguments.Insert(1, (Resolve-Path -LiteralPath $fileArgument).ProviderPath)
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
  [Console]::Error.WriteLine('error: cargo was not found. Install Rust through rustup (see docs\DEVELOPER_GUIDE.md) and try again.')
  exit 2
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$manifest = Join-Path $repositoryRoot 'src-tauri\Cargo.toml'
$cargoArguments = @('run', '--locked', '--manifest-path', $manifest, '--example', 'factory_labs', '--') + $toolArguments.ToArray()

# cargo prints its build progress on stderr; let it through unchanged.
$ErrorActionPreference = 'Continue'
& cargo @cargoArguments
$exitCode = $LASTEXITCODE
if ($exitCode -eq 0 -or $exitCode -eq 1 -or $exitCode -eq 2) {
  exit $exitCode
}
[Console]::Error.WriteLine("error: the factory lab tool could not be built or run (cargo exit code $exitCode).")
exit 2
