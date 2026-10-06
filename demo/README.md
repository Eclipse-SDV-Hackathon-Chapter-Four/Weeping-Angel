# Battery Thermal Guardian — Safety Evidence Factory

This demo wires the **Battery Thermal Guardian** app to a **Diagnostic Fault
Manager (DFM)** and exposes the resulting faults through the **OpenSOVD**
diagnostic gateway. A Robot Framework suite injects battery-temperature and
transport faults, verifies the Guardian raises the right faults, reads them
back over the OpenSOVD HTTP interface, and produces a Markdown evidence report.

## Diagnostic chain

```
fault_injector --(uProtocol/Zenoh)--> Battery Thermal Guardian
      --(iceoryx2 IPC)--> DFM --> OpenSOVD gateway
      --> SOVD HTTP: GET /sovd/v1/apps/battery_guardian/faults
```

The Guardian is modelled as an **app** located on the `hpc` host component, so
its faults are served at `/sovd/v1/apps/battery_guardian/faults`.

## Fault catalog

Defined in [`diagnostics/catalog/battery_guardian.json`](diagnostics/catalog/battery_guardian.json):

| Fault code | Trigger |
|---|---|
| `BatteryOverTempWarning`  | Max cell temp ≥ 45 °C |
| `BatteryOverTempCritical` | Max cell temp ≥ 55 °C |
| `BatteryTempSignalStale`  | No fresh sample within the freshness deadline (delayed/dropped signal) |
| `BatteryTempSignalStuck`  | Temperature value frozen across consecutive samples |
| `BatteryTempImplausible`  | Out-of-plausible-range value or implausibly fast change |

## Quick start

```bash
# Build everything, launch the stack, run the suite, tear down:
scripts/run_demo.sh

# Skip the cargo builds (binaries already built):
scripts/run_demo.sh --no-build
```

Outputs (in `reports/`):

- `evidence_report.md` — the safety evidence report (hazard → fault → OpenSOVD → verdict)
- `report.html` / `log.html` — the Robot Framework run report
- `output.xml` — machine-readable Robot results

Runtime logs are written under `/tmp/dr-whodunit/` (`dfm.log`, `guardian.log`,
`gateway.log`, `toxiproxy.log`).

## Test scenarios

The suite ([`tests/battery_guardian.robot`](tests/battery_guardian.robot)) runs
one test per hazard. Each test resets sticky DTC state, re-establishes a clean
baseline, injects a fault, and asserts the expected fault appears on OpenSOVD.

| Scenario | Injected condition | Expected fault |
|---|---|---|
| Baseline          | Nominal temperature ramp                    | _none_ |
| Overtemperature   | Ramp through 45 °C / 55 °C                   | `BatteryOverTempWarning`, `BatteryOverTempCritical` |
| Stuck signal      | Frozen temperature value                    | `BatteryTempSignalStuck` |
| Implausible spike | Out-of-range spike (300 °C)                 | `BatteryTempImplausible` |
| Source dropout    | Publisher stops                             | `BatteryTempSignalStale` |
| Transport delay   | Zenoh link blackholed via Toxiproxy         | `BatteryTempSignalStale` |

Signal-level faults are shaped by the `fault_injector` binary
(`SCENARIO` env var). The transport-delay scenario injects a **Toxiproxy**
`timeout` toxic on the Zenoh link (proxy `zenoh`, `127.0.0.1:7448 → 7447`) so
that otherwise-valid samples stop reaching the Guardian.

## Components

| Path | Role |
|---|---|
| `services/src/bin/guardian.rs`        | Guardian: subscribes to temp events, evaluates faults, reports to DFM |
| `services/src/fault_reporter.rs`      | Guardian → DFM reporting bridge (owns iceoryx2 reporters) |
| `services/src/bin/fault_injector.rs`  | Scenario-driven temperature source |
| `diagnostics/catalog/battery_guardian.json` | DFM fault catalog |
| `fault-lib/`                          | DFM library + `dfm_bin` standalone manager |
| `opensovd-core/`                      | OpenSOVD gateway (`--dfm-fault-app battery_guardian`) |
| `tests/battery_guardian.robot`        | Robot Framework suite |
| `tests/SovdFaultLibrary.py`           | Robot library: injection, Toxiproxy, SOVD polling, report generation |
| `scripts/run_demo.sh`                 | One-command build + launch + test + teardown |

## Prerequisites

- Rust toolchain, `protoc` (`protobuf-compiler`) for the KUKSA proto build
- Python 3 with `robotframework` and `requests` (`pip install robotframework requests`)
- Toxiproxy binaries in `.tools/` (bundled)
