# Battery Thermal Guardian — Safety Evidence Report

_Generated: 2026-09-10 07:28:41Z_

This report is produced automatically by the Robot Framework suite. Each scenario injects a battery-temperature or transport fault, the Guardian evaluates it and reports to the Diagnostic Fault Manager (DFM), and the faults are read back through the **OpenSOVD** fault interface.

## Diagnostic chain

```
fault_injector --(uProtocol/Zenoh)--> Battery Thermal Guardian
      --(iceoryx2 IPC)--> DFM --> OpenSOVD gateway
      --> SOVD HTTP: GET http://127.0.0.1:7690/sovd/v1/apps/battery_guardian/faults
```

## Fault catalog

| Fault code | Category | Severity | Description |
|---|---|---|---|
| `BatteryOverTempWarning` | Hardware | Warn | Max cell temperature crossed the WARNING threshold (>=45 C). |
| `BatteryOverTempCritical` | Hardware | Fatal | Max cell temperature crossed the CRITICAL threshold (>=55 C). |
| `BatteryTempSignalStale` | Communication | Error | No fresh battery temperature sample received within the freshness deadline (delayed/dropped signal). |
| `BatteryTempSignalStuck` | Communication | Error | Battery temperature value has not changed across consecutive samples (stuck signal). |
| `BatteryTempImplausible` | Configuration | Error | Battery temperature is out of plausible range or changed implausibly fast (spike/out-of-range). |

## Scenario results

**6/6 scenarios passed.**

| Scenario | Injected condition | Expected fault(s) | OpenSOVD observed | Verdict |
|---|---|---|---|---|
| Baseline | Nominal temperature ramp (30..44 C) | _none_ | _none (clear)_ | PASS |
| Overtemperature | Temp ramp through 45 C / 55 C | `BatteryOverTempWarning`, `BatteryOverTempCritical` | `BatteryOverTempCritical`, `BatteryOverTempWarning` | PASS |
| Stuck signal | Frozen temperature value (40 C repeated) | `BatteryTempSignalStuck` | `BatteryTempSignalStuck` | PASS |
| Implausible spike | Out-of-range spike (300 C) | `BatteryTempImplausible` | `BatteryTempImplausible` | PASS |
| Source dropout | Publisher stops (no fresh samples) | `BatteryTempSignalStale` | `BatteryTempSignalStale` | PASS |
| Transport delay | Zenoh link blackholed (delayed/dropped signal) | `BatteryTempSignalStale` | `BatteryTempSignalStale` | PASS |

## Appendix — OpenSOVD fault status per scenario

### Baseline
_captured 2026-09-10 07:27:33Z_

| Fault code | testFailed | confirmedDtc | warningIndicator |
|---|---|---|---|
| `BatteryOverTempCritical` | False | False | False |
| `BatteryOverTempWarning` | False | False | False |
| `BatteryTempImplausible` | False | False | False |
| `BatteryTempSignalStale` | False | False | False |
| `BatteryTempSignalStuck` | False | False | False |

### Overtemperature
_captured 2026-09-10 07:27:48Z_

| Fault code | testFailed | confirmedDtc | warningIndicator |
|---|---|---|---|
| `BatteryOverTempCritical` | True | True | True |
| `BatteryOverTempWarning` | True | True | True |
| `BatteryTempImplausible` | False | False | False |
| `BatteryTempSignalStale` | False | False | True |
| `BatteryTempSignalStuck` | False | False | False |

### Stuck signal
_captured 2026-09-10 07:28:01Z_

| Fault code | testFailed | confirmedDtc | warningIndicator |
|---|---|---|---|
| `BatteryOverTempCritical` | False | False | False |
| `BatteryOverTempWarning` | False | False | False |
| `BatteryTempImplausible` | False | False | False |
| `BatteryTempSignalStale` | False | False | True |
| `BatteryTempSignalStuck` | True | True | True |

### Implausible spike
_captured 2026-09-10 07:28:12Z_

| Fault code | testFailed | confirmedDtc | warningIndicator |
|---|---|---|---|
| `BatteryOverTempCritical` | False | False | False |
| `BatteryOverTempWarning` | False | False | False |
| `BatteryTempImplausible` | True | True | True |
| `BatteryTempSignalStale` | False | False | False |
| `BatteryTempSignalStuck` | False | False | False |

### Source dropout
_captured 2026-09-10 07:28:25Z_

| Fault code | testFailed | confirmedDtc | warningIndicator |
|---|---|---|---|
| `BatteryOverTempCritical` | False | False | False |
| `BatteryOverTempWarning` | False | False | False |
| `BatteryTempImplausible` | False | False | False |
| `BatteryTempSignalStale` | True | True | True |
| `BatteryTempSignalStuck` | False | False | False |

### Transport delay
_captured 2026-09-10 07:28:41Z_

| Fault code | testFailed | confirmedDtc | warningIndicator |
|---|---|---|---|
| `BatteryOverTempCritical` | False | False | False |
| `BatteryOverTempWarning` | False | False | False |
| `BatteryTempImplausible` | False | False | False |
| `BatteryTempSignalStale` | True | True | True |
| `BatteryTempSignalStuck` | False | False | False |

