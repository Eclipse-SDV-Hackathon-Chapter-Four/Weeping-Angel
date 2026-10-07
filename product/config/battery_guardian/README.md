# Battery Guardian configuration

| File | Defines |
|---|---|
| `guardian_model.yaml` | Guardian physical-model parameters |
| `guardian_diagnostics.json` | Guardian detection to DFM diagnostic mapping |
| `fault_injection_model.yaml` | Fault-generator mutation ground truth |

- `guardian_model.yaml` answers: **What is admissible?** It contains only
  numerical parameters consumed by the Guardian model.
- `guardian_diagnostics.json` answers: **How is a detected violation
  reported?** It is the catalog consumed by the existing DFM reporter.
- `fault_injection_model.yaml` answers: **What fault is deliberately
  injected?** It belongs to the Fault Generator/Mutator and is not read by the
  Guardian.

```text
                    guardian_model.yaml
                           |
                           | parameters
                           v
CAN/VSS -----------> +-----------+
sensor values        | Guardian  |
                     |   model   |
                     +-----------+
                           |
                           | DetectionClass
                           v
                 +-------------------+
                 | existing DFM      |
                 | reporter          |
                 +-------------------+
                           |
                           | mapping
                           v
                 guardian_diagnostics.json


fault_injection_model.yaml
           |
           | injection specification
           v
   +-----------------+
   | Fault Generator |
   | / Mutator       |
   +-----------------+
           |
           | mutated CAN / transport / source
           v
        system
```

```text
               injection ground truth
Fault Generator -----------------------------+
                                              |
                                              v
                                       +--------------+
Guardian -------- DetectionClass ----------> | Evidence |
DFM/OpenSOVD ------------------------------> | Collector|
                                       +--------------+
```

## Diagnostic mapping

This is a Guardian detection to DFM fault mapping, not an injected-fault to
DFM-fault mapping.

| Guardian `DetectionClass` | DFM fault |
|---|---|
| `STREAM_STALE` | `BatteryTempStreamStale` |
| `PHYSICAL_TEMP_ABSOLUTE_LIMIT` | `BatteryTempAbsoluteLimit` |
| `PHYSICAL_TEMP_ORDERING` | `BatteryTempOrdering` |
| `PHYSICAL_TEMP_SPREAD` | `BatteryTempSpread` |
| `PHYSICAL_TEMP_HOTSPOT` | `BatteryTempHotspot` |
| `PHYSICAL_TEMP_RATE` | `BatteryTempRate` |
| `PHYSICAL_SOC_RANGE` | `BatterySocRange` |
| `PHYSICAL_SOC_RATE` | `BatterySocRate` |
| `SIGNAL_STUCK` | `BatterySignalStuck` |

## Injection ground truth versus observation

The Fault Generator knows the injected cause. The Guardian detects violations
of its model. The Guardian must not infer the injected fault class.

```text
transport.drop ---+
transport.delay --+--> STREAM_STALE
source.dropout  ---+

signal.spike ------> usually PHYSICAL_TEMP_RATE

signal.combination -> zero, one, or multiple DetectionClass values
```

Every single-signal fault uses one `mutations[]` entry. A
`signal.combination` uses at least two entries targeting distinct canonical
signals; each component operator is `stuck`, `spike`, `drift`, or
`out_of_range`. Transport and source faults use `action` instead of pretending
to mutate a sensor value.

At execution time the generator's ground-truth record contains
`injection_id`, `injected_class`, `started_at`, and either `finished_at` or
`duration_ms`. Signal records also contain the complete `mutations[]` list,
including every component of a combination.
