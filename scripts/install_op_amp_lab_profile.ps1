#Requires -Version 5.1

[CmdletBinding()]
param(
  [string]$CatalogRoot = (Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'WVU Bioinstrumentation Studio\profiles')
)

$ErrorActionPreference = 'Stop'
$profileId = 'wvu.bmeg420l.general.analog.development.v2'
$profileVersion = '2.0.1'
$expectedImportedHash = 'c73cc1f2941910696d88f934b7b4c09b8b110438a4886d7d799b13807f61471d'
$expectedInstructorHash = 'bcc08868305ca55e81920e3a5805b020cfa612a0374e75fd5a290fa11f918ad9'

# This is the locked profile exported from WVU Bioinstrumentation Studio and
# normalized to the same `imported` source that the application's Import action
# uses. Keeping it embedded makes this a single-file student installer.
$profileJson = @'
{
  "schema_version": 1,
  "profile_id": "wvu.bmeg420l.general.analog.development.v2",
  "profile_version": "2.0.1",
  "display_name": "Op Amp Lab",
  "category": "development",
  "status": "locked",
  "source": "imported",
  "description": "Arduino analog acquisition for the operational amplifiers lab.",
  "target_board": "Arduino UNO R4 WiFi",
  "fqbn": "arduino:renesas_uno:unor4wifi",
  "required_firmware": {
    "protocol_major": 0,
    "protocol_minor_min": 4,
    "build": "0x00010006",
    "device": "0x554E4F34"
  },
  "acquisition": {
    "analog_pin": "A0",
    "adc_resolution_bits": 14,
    "sample_rate_hz": 100,
    "allowed_duration_modes": [
      "timed",
      "until_stopped"
    ],
    "timed_presets_seconds": [
      10,
      30,
      60,
      300,
      600
    ],
    "minimum_custom_duration_seconds": 10,
    "acquisition_mode": "simultaneous",
    "channels": [
      {
        "pin": "A0",
        "id": "Vmid",
        "label": "A0",
        "csv_name": "a0_counts",
        "units": "ADC counts",
        "allowed_conversions": [
          "counts_volts"
        ],
        "default_display_unit": "counts"
      },
      {
        "pin": "A1",
        "id": "Vout",
        "label": "A1",
        "csv_name": "01_counts",
        "units": "ADC counts",
        "allowed_conversions": [
          "counts_volts"
        ],
        "default_display_unit": "counts"
      }
    ],
    "analog_inputs": null,
    "led_outputs": null,
    "state_dwell_us": null
  },
  "display": {
    "primary_quantity": "arduino_input_volts",
    "channel_label": "General analog input",
    "raw_units_label": "ADC counts",
    "voltage_units_label": "V",
    "voltage_reference_v": 5.0,
    "plot_min_v": 0.0,
    "plot_max_v": 5.0
  },
  "safety": {
    "bench_only": true,
    "human_connection_authorized": false,
    "not_medical_device": true,
    "notices": [
      "Teaching use only \u2014 not a medical device.",
      "Use simulator, UNO alone, or a safe 0\u20135 V bench signal for development."
    ]
  },
  "export": {
    "signal_name": "general_analog_raw",
    "include_profile_snapshot": true
  },
  "integrity": {
    "canonical_hash_algorithm": "SHA-256",
    "canonical_hash": "c73cc1f2941910696d88f934b7b4c09b8b110438a4886d7d799b13807f61471d"
  },
  "plot_defaults": {
    "groups": [
      {
        "channel_ids": [
          "Vmid"
        ]
      },
      {
        "channel_ids": [
          "Vout"
        ]
      }
    ]
  }
}
'@

function Get-CanonicalProfileHash {
  param([Parameter(Mandatory = $true)]$Profile)

  $savedHash = [string]$Profile.integrity.canonical_hash
  try {
    $Profile.integrity.canonical_hash = ''
    $canonicalJson = $Profile | ConvertTo-Json -Depth 100 -Compress
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($canonicalJson)
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
      return ([BitConverter]::ToString($sha256.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    } finally {
      $sha256.Dispose()
    }
  } finally {
    $Profile.integrity.canonical_hash = $savedHash
  }
}

function Test-HasProperty {
  param(
    [Parameter(Mandatory = $true)]$Object,
    [Parameter(Mandatory = $true)][string]$Name
  )
  return $null -ne $Object.PSObject.Properties[$Name]
}

try {
  if ($PSVersionTable.PSEdition -eq 'Core' -and -not $IsWindows) {
    throw 'This installer is intended for Windows.'
  }
  if ([string]::IsNullOrWhiteSpace($CatalogRoot)) {
    throw 'The current-user AppData path is unavailable.'
  }
  if (Get-Process -Name 'wvu_bioinstrumentation_studio' -ErrorAction SilentlyContinue) {
    throw 'Close WVU Bioinstrumentation Studio before installing this lab profile.'
  }

  $incoming = $profileJson | ConvertFrom-Json
  $computedIncomingHash = Get-CanonicalProfileHash -Profile $incoming
  if ($incoming.integrity.canonical_hash_algorithm -ne 'SHA-256' -or
      $incoming.integrity.canonical_hash -ne $expectedImportedHash -or
      $computedIncomingHash -ne $expectedImportedHash) {
    throw 'The embedded Op Amp Lab profile failed its integrity check.'
  }

  New-Item -ItemType Directory -Path $CatalogRoot -Force | Out-Null
  $catalogPath = Join-Path $CatalogRoot 'lab_state.json'
  $catalogExisted = Test-Path -LiteralPath $catalogPath
  if ($catalogExisted) {
    try {
      # Windows PowerShell 5.1 otherwise treats UTF-8 without a BOM as the
      # system ANSI code page, which can corrupt profile notices and hashes.
      $catalog = [IO.File]::ReadAllText($catalogPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
    } catch {
      throw "The existing lab catalog is not valid JSON. It was not changed: $catalogPath"
    }
  } else {
    $catalog = [pscustomobject][ordered]@{
      profiles = @()
      retired = @()
      active_versions = [pscustomobject]@{}
      completed_save_requests = [pscustomobject]@{}
    }
  }

  foreach ($requiredProperty in @('profiles', 'retired', 'active_versions', 'completed_save_requests')) {
    if (-not (Test-HasProperty -Object $catalog -Name $requiredProperty)) {
      throw "The existing lab catalog is missing '$requiredProperty'. It was not changed."
    }
  }

  $profiles = @($catalog.profiles)
  $matching = @($profiles | Where-Object {
    $_.profile_id -eq $profileId -and $_.profile_version -eq $profileVersion
  })
  if ($matching.Count -gt 1) {
    throw 'The existing catalog contains duplicate copies of the target lab version. It was not changed.'
  }

  $alreadyInstalled = $matching.Count -eq 1
  if ($alreadyInstalled) {
    $existingHash = Get-CanonicalProfileHash -Profile $matching[0]
    if ($existingHash -notin @($expectedImportedHash, $expectedInstructorHash)) {
      throw "A different lab already uses ID '$profileId' and version '$profileVersion'. Nothing was changed."
    }
  } else {
    $profiles += $incoming
    $catalog.profiles = @($profiles)
  }

  $activeProperty = $catalog.active_versions.PSObject.Properties[$profileId]
  $alreadyActive = $null -ne $activeProperty -and [string]$activeProperty.Value -eq $profileVersion
  if ($alreadyInstalled -and $alreadyActive) {
    Write-Host 'Op Amp Lab is already installed and active for this Windows user.' -ForegroundColor Green
    Write-Host "Catalog: $catalogPath"
    exit 0
  }
  $catalog.active_versions | Add-Member -MemberType NoteProperty -Name $profileId -Value $profileVersion -Force

  $retired = @()
  foreach ($entry in @($catalog.retired)) {
    $parts = @($entry)
    if ($parts.Count -eq 2 -and $parts[0] -eq $profileId -and $parts[1] -eq $profileVersion) {
      continue
    }
    $retired += ,$entry
  }
  $catalog.retired = @($retired)

  $temporaryPath = Join-Path $CatalogRoot ("lab_state.install-{0}.tmp" -f [Guid]::NewGuid().ToString('N'))
  $backupPath = $null
  try {
    $serialized = $catalog | ConvertTo-Json -Depth 100
    [IO.File]::WriteAllText($temporaryPath, $serialized, [Text.UTF8Encoding]::new($false))
    $null = [IO.File]::ReadAllText($temporaryPath, [Text.Encoding]::UTF8) | ConvertFrom-Json

    if ($catalogExisted) {
      $backupPath = Join-Path $CatalogRoot ("lab_state.backup-{0}.json" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
      [IO.File]::Replace($temporaryPath, $catalogPath, $backupPath, $true)
    } else {
      Move-Item -LiteralPath $temporaryPath -Destination $catalogPath
    }
  } finally {
    if (Test-Path -LiteralPath $temporaryPath) {
      Remove-Item -LiteralPath $temporaryPath -Force
    }
  }

  $operation = if ($alreadyInstalled) { 'student_profile_activate' } else { 'student_profile_install' }
  $auditLine = "{0}`tLAB_WRITE`t{1}`t{2}`t-`t{3}`trequest=student-script{4}" -f `
    [DateTime]::UtcNow.ToString('o'), $operation, $profileId, $profileVersion, [Environment]::NewLine
  [IO.File]::AppendAllText(
    (Join-Path $CatalogRoot 'lab_write_audit.log'),
    $auditLine,
    [Text.UTF8Encoding]::new($false)
  )

  Write-Host 'Op Amp Lab was installed and selected successfully.' -ForegroundColor Green
  Write-Host "Catalog: $catalogPath"
  if ($backupPath) {
    Write-Host "Backup:  $backupPath"
  }
  Write-Host 'Open WVU Bioinstrumentation Studio and select Op Amp Lab.'
  exit 0
} catch {
  # Write-Error would become a new terminating error here (the preference is
  # Stop), print a full PowerShell error record, and skip `exit 1`. Show only
  # the message and return the documented failure code instead.
  Write-Host $_.Exception.Message -ForegroundColor Red
  exit 1
}
