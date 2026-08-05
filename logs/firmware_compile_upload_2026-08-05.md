# Firmware compile and upload log

## Commands

```powershell
arduino-cli compile --fqbn arduino:renesas_uno:unor4wifi firmware\reference_unor4wifi
arduino-cli upload --fqbn arduino:renesas_uno:unor4wifi --port COM12 firmware\reference_unor4wifi
```

## Result

- Compile succeeded against `arduino:renesas_uno` 1.5.3.
- Final firmware size: 53,444 bytes (20% of 262,144-byte program storage).
- Global variables: 7,852 bytes (23% of 32,768-byte dynamic memory).
- Upload to COM12 succeeded; Arduino CLI reported `New upload port: COM12 (serial)`.

The initial build was 53,508 bytes. It was replaced after correcting contiguous frame emission, then rebuilt and re-uploaded successfully.
