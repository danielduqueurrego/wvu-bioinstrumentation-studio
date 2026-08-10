# Phase 5 BP calibration acceptance

Status: deterministic simulator relationship, automated fit coverage, and UNO-only acquisition
smoke passed; manual UI calibration interaction remains pending.

The BP simulator intentionally generates a non-physiological engineering relationship:

```text
MPXV_mmHg = 120 × XGZP_volts − 10
```

This allows the student XGZP linear-fit path to recover slope, offset, R², and paired-sample count
without claiming a physical sensor validation. The real class workflow remains responsible for
selecting an interval and deciding how to use any student calibration.

UNO-only BP smoke completed on COM12 with floating inputs: 1,990 synchronized frames at
200.005 Hz, 232 valid packets, no CRC/sequence/overflow/disconnect/reconnect error, D4 HIGH
during capture and LOW after Stop. This confirms the acquisition path only; it does not claim a
physical cuff/XGZP calibration.
