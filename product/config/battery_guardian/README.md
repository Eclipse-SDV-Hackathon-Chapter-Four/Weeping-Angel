# Battery Guardian configuration

| File | Defines |
|---|---|
| `guardian_model.yaml` | Guardian physical-model parameters |
| `guardian_diagnostics.json` | Configured Guardian-to-DFM projection |
| `fault_injection_model.yaml` | Fault-generator mutation ground truth |

The files answer three separate questions: what observations are admissible,
how selected detections are represented diagnostically, and what fault was
deliberately injected. The Guardian never infers the injected class.

## Detection semantics

A Guardian observation is the orthogonal pair:

```text
Guardian observation = DetectionClass × DetectionLevel
```

`DetectionClass` identifies which model rule reacted. `DetectionLevel`
identifies whether that observation is a `WARNING`, `VIOLATION`, or
`CRITICAL`.

`THERMAL_LIMIT` uses the maximum temperature. The critical threshold is the
configured `absolute_max_c`; the warning threshold is derived as
`absolute_max_c - warning_margin_c`:

| Observation | Meaning |
|---|---|
| `THERMAL_LIMIT / WARNING` | Maximum temperature entered the warning band |
| `THERMAL_LIMIT / CRITICAL` | Maximum temperature reached or exceeded the absolute-maximum threshold |

The levels are mutually exclusive. At exactly 70 °C, `THERMAL_LIMIT /
CRITICAL` is active without `PHYSICAL_TEMP_ABSOLUTE_LIMIT`; above 70 °C both
the critical thermal observation and the independent absolute-limit
`VIOLATION` are active.

Spread, hotspot, and temperature-rate checks use
`utilization = observed / limit`. Below `warning.utilization_threshold` they
produce no detection, from that threshold through 1.0 they produce `WARNING`,
and above 1.0 they produce `VIOLATION`. Their detections carry both
`residual = observed - limit` and utilization. Cooling rate uses positive
magnitudes for both values.

Absolute temperature, ordering, SoC range/rate, stuck, and stale checks remain
binary and produce `VIOLATION` only.

## DFM projection

Guardian detections do not necessarily become DFM faults. The existing
reporter projects configured `DetectionClass × DetectionLevel` pairs:

| Guardian observation | DFM fault |
|---|---|
| `THERMAL_LIMIT / WARNING` | `BatteryOverTempWarning` |
| `THERMAL_LIMIT / CRITICAL` | `BatteryOverTempCritical` |
| `STREAM_STALE / VIOLATION` | `BatteryTempStreamStale` |
| `PHYSICAL_TEMP_ABSOLUTE_LIMIT / VIOLATION` | `BatteryTempAbsoluteLimit` |
| `PHYSICAL_TEMP_ORDERING / VIOLATION` | `BatteryTempOrdering` |
| `PHYSICAL_TEMP_SPREAD / VIOLATION` | `BatteryTempSpread` |
| `PHYSICAL_TEMP_HOTSPOT / VIOLATION` | `BatteryTempHotspot` |
| `PHYSICAL_TEMP_RATE / VIOLATION` | `BatteryTempRate` |
| `PHYSICAL_SOC_RANGE / VIOLATION` | `BatterySocRange` |
| `PHYSICAL_SOC_RATE / VIOLATION` | `BatterySocRate` |
| `SIGNAL_STUCK / VIOLATION` | `BatterySignalStuck` |

Warnings for spread, hotspot, and rate remain available as internal evidence
but intentionally have no DFM mapping. The diagnostic catalog contains no
model thresholds.

Every mapped fault change is also mirrored, with the same fault ids, as
`GuardianFaultEvent` JSON on uProtocol topic `//guardian/1001/1/8001`
(ADR-007). Unmapped warnings are published on neither channel.

## Injection ground truth versus observation

```text
Injected fault class
        |
        | affects system
        v
Guardian model
        |
        v
DetectionClass × DetectionLevel
        |
        +------> Evidence / internal observation
        |
        +------> configured DFM mapping
```

Examples:

```text
signal.spike
    -> PHYSICAL_TEMP_RATE / WARNING
    -> later possibly PHYSICAL_TEMP_RATE / VIOLATION

signal.drift
    -> PHYSICAL_TEMP_HOTSPOT / WARNING
    -> PHYSICAL_TEMP_SPREAD / WARNING
    -> later possibly corresponding VIOLATIONs

coherent temperature increase
    -> THERMAL_LIMIT / WARNING
    -> THERMAL_LIMIT / CRITICAL
```

`signal.spike` remains an injected class; there is no Guardian `SPIKE`
detection. Signal combinations retain the complete `mutations[]` ground truth
and may lead to zero, one, or multiple Guardian observations.
