# Battery Thermal Guardian — Safety Evidence Report

_Generated: 2026-09-10 13:58:50Z_

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

**5/5 scenarios passed.**

| Scenario | Injected condition | Expected fault(s) | OpenSOVD observed | Verdict |
|---|---|---|---|---|
| Baseline | Nominal temperature ramp (30..44 C) | _none_ | _none (clear)_ | PASS |
| Overtemperature | Temp ramp through 45 C / 55 C | `BatteryOverTempWarning`, `BatteryOverTempCritical` | `BatteryOverTempCritical`, `BatteryOverTempWarning` | PASS |
| Implausible spike | Out-of-range spike (300 C) | `BatteryTempImplausible` | `BatteryTempImplausible` | PASS |
| Source dropout | Publisher stops (no fresh samples) | `BatteryTempSignalStale` | `BatteryTempSignalStale` | PASS |
| Transport delay | Zenoh link blackholed (delayed/dropped signal) | `BatteryTempSignalStale` | `BatteryTempSignalStale` | PASS |

## The detective story — from symptom to culprit

A DTC is only a *symptom*. This section walks each clue back along the diagnostic chain to the **culprit**: the layer where the fault entered, the means by which it was introduced, and the **witness** (the Guardian detector) that caught it. The OpenSOVD read-back is the confession on record.

| Case | Symptom (DTC on OpenSOVD) | Origin layer | Culprit (root cause) | How it entered | Caught by (witness) |
|---|---|---|---|---|---|
| Baseline | _none (cleared)_ | — (healthy system) | No culprit — nominal operation | Nominal 30..44 C ramp with fresh samples throughout | _no detector tripped_ |
| Overtemperature | `BatteryOverTempCritical`, `BatteryOverTempWarning` | Physical — battery pack (Hardware) | Genuine cell over-temperature — the pack really crossed 45 C then 55 C | fault_injector shapes a true rising temperature ramp | Guardian threshold monitor — temp_max >= 55 C (CRIT band); Guardian threshold monitor — temp_max >= 45 C (WARN band) |
| Implausible spike | `BatteryTempImplausible` | Sensor / plausibility (Configuration) | Corrupt reading — 300 C is physically impossible | fault_injector emits a single out-of-range spike (300 C) | Guardian plausibility check — value outside [-40, 125] C or a jump > 20 C between consecutive samples |
| Source dropout | `BatteryTempSignalStale` | Publisher — application source (Communication) | Silent publisher — the temperature source stopped emitting | fault_injector sends a few samples then exits; the wire goes quiet | Guardian freshness watchdog — no fresh sample for > 2000 ms (polled every 500 ms) |
| Transport delay | `BatteryTempSignalStale` | Transport — Zenoh link (Communication) | Broken link — a healthy publisher's samples are dropped in transit | Toxiproxy blackholes the Zenoh downstream while the injector keeps publishing | Guardian freshness watchdog — no fresh sample for > 2000 ms (polled every 500 ms) |

### The twist — one DTC, two culprits

`BatteryTempSignalStale` is raised in **2** different cases (*Source dropout*, *Transport delay*) with an *identical* fault code. The DTC alone cannot tell you **who did it** — the Guardian's freshness watchdog only knows that no fresh sample arrived in time.

The evidence that distinguishes the culprits:

- **Source dropout** — the publisher process has *exited*; nothing is on the wire at all. Root cause lives at the **application/source** layer.
- **Transport delay** — the publisher is *still alive and emitting*, but the Zenoh link is blackholed by Toxiproxy, so samples never arrive. Root cause lives at the **transport** layer.

> Detective's note: to close the case you must correlate the DTC with liveness of the publisher and traffic on the Zenoh link — the read-back confirms *that* the signal went stale, not *why*.

## Appendix — OpenSOVD fault status per scenario

### Baseline
_captured 2026-09-10 13:57:39Z_

| Fault code | testFailed | confirmedDtc | warningIndicator |
|---|---|---|---|
| `BatteryOverTempCritical` | False | False | False |
| `BatteryOverTempWarning` | False | False | False |
| `BatteryTempImplausible` | False | False | False |
| `BatteryTempSignalStale` | False | False | False |
| `BatteryTempSignalStuck` | False | False | False |

### Overtemperature
_captured 2026-09-10 13:57:54Z_

| Fault code | testFailed | confirmedDtc | warningIndicator |
|---|---|---|---|
| `BatteryOverTempCritical` | True | True | True |
| `BatteryOverTempWarning` | True | True | True |
| `BatteryTempImplausible` | False | False | False |
| `BatteryTempSignalStale` | False | False | False |
| `BatteryTempSignalStuck` | False | False | False |

### Implausible spike
_captured 2026-09-10 13:58:21Z_

| Fault code | testFailed | confirmedDtc | warningIndicator |
|---|---|---|---|
| `BatteryOverTempCritical` | False | False | False |
| `BatteryOverTempWarning` | False | False | False |
| `BatteryTempImplausible` | True | True | True |
| `BatteryTempSignalStale` | False | False | True |
| `BatteryTempSignalStuck` | False | False | False |

### Source dropout
_captured 2026-09-10 13:58:34Z_

| Fault code | testFailed | confirmedDtc | warningIndicator |
|---|---|---|---|
| `BatteryOverTempCritical` | False | False | False |
| `BatteryOverTempWarning` | False | False | False |
| `BatteryTempImplausible` | False | False | False |
| `BatteryTempSignalStale` | True | True | True |
| `BatteryTempSignalStuck` | False | False | False |

### Transport delay
_captured 2026-09-10 13:58:50Z_

| Fault code | testFailed | confirmedDtc | warningIndicator |
|---|---|---|---|
| `BatteryOverTempCritical` | False | False | False |
| `BatteryOverTempWarning` | False | False | False |
| `BatteryTempImplausible` | False | False | False |
| `BatteryTempSignalStale` | True | True | True |
| `BatteryTempSignalStuck` | False | False | False |

