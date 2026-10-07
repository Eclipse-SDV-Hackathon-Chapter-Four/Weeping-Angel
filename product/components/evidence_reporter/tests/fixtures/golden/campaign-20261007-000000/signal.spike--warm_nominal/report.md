# Evidence Report - signal.spike--warm_nominal

- **Campaign:** signal.spike
- **Scenario:** warm_nominal
- **Run ID:** signal.spike--warm_nominal
- **Generated:** 1970-01-01T00:00:00Z
- **Verdict:** **PASS**

## Provenance and reproduction

- Case (ASC): [case.asc](../experiments/signal.spike/warm_nominal/case.asc)
- Ground truth: [case.ground_truth.yaml](../experiments/signal.spike/warm_nominal/case.ground_truth.yaml)
- Oracle: [case.oracle.yaml](../experiments/signal.spike/warm_nominal/case.oracle.yaml)
- Experiment: [experiment.yaml](../experiments/signal.spike/warm_nominal/experiment.yaml)
- Logs: -
- Case path (as recorded): `/tmp/case`

## Diagnostic chain

```text
replay (ASC) -> KUKSA CAN Provider -> KUKSA Data Broker -> VSS uProtocol Publisher -> Battery Thermal Guardian -> DFM -> OpenSOVD -> Evidence Collector
```

## Fault catalog

| Fault code | Category | Severity | Description |
|---|---|---|---|
| BatteryTempRate | Configuration | Error | Temperature changed faster than physically possible. |

## Injections

### signal-spike-1 - signal.spike

- Verdict: **PASS** (2/2 matched, 0 missing, 0 unexpected, 0 not in OpenSOVD)
- Source window: 1000..1100 ms (slot 1000..4500)
- `temp_max` / `spike`: requested duration_samples=1, delta=2; executed [42]

## Expected vs. observed

Oracle: 2 expected, 2 matched, 0 missing, 0 not reached, 0 unexpected, 0 not applicable

| Status | Fault | Class | Level | Stage | Expected ms | Observed ms | Signal | OpenSOVD ms |
|---|---|---|---|---|---|---|---|---|
| MATCHED | BatteryTempRate | PHYSICAL_TEMP_RATE | VIOLATION | Failed | 1000 | 1000 | temp_max | 1076 |
| MATCHED | BatteryTempRate | PHYSICAL_TEMP_RATE | VIOLATION | Passed | 1200 | 1200 | temp_max | - |

## Detection evidence

| t_ms | Fault | Class | Level | Stage | Signal | Observed | Limit | Residual | Utilization | Interval ms |
|---|---|---|---|---|---|---|---|---|---|---|
| 1000 | BatteryTempRate | PHYSICAL_TEMP_RATE | VIOLATION | Failed | temp_max | 10 | 8 | 2 | 1.25 | 100 |
| 1200 | BatteryTempRate | PHYSICAL_TEMP_RATE | VIOLATION | Passed | temp_max | - | - | - | - | - |

## DFM / OpenSOVD correlation

- URL: `http://127.0.0.1:7690/sovd/v1/components/battery_guardian/data/faults`
- Polls: 10 (0 failed)
- Activations: 1; not visible: 0; unexplained: 0

| t_ms | Code |
|---|---|
| 1076 | BatteryTempRate |

## Timing

- Replay end: 5000 ms; battery stream end: 4900 ms

| Injection | Fault | Injected ms | Detected ms | Detection latency ms | OpenSOVD latency ms |
|---|---|---|---|---|---|
| signal-spike-1 | BatteryTempRate | 1000 | 1000 | 0 | 76 |
| signal-spike-1 | BatteryTempRate | 1000 | 1200 | 200 | - |

## Verdict rationale

All 2 expected Guardian change(s) matched and no unexpected change was observed.

## Observer artifacts

_No observer artifacts captured for this run._

Observer artifacts are illustrative only and never a source of evaluation facts.

## Gaps and limitations

- **Mitigation:** not instrumented (v1 is event-only, ADR-007) - no mitigation event is recorded; this chain link is intentionally absent.
- **OpenSOVD fault status:** only activation times are captured; the `testFailed`/`confirmedDtc`/`warningIndicator` triple is not observable.
- **Unmapped Guardian warnings:** only DFM-mapped `class/level` pairs reach the collector stream (ADR-016 section 11); utilization warnings may be invisible.

## Notes

- test note
