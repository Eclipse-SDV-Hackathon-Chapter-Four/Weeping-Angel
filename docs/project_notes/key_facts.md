# Key Facts

Non-sensitive project reference data. Verified against the checkout
2026-10-06 (see also `demo/README.md`, `demo/docs/README.md`, `demo/tutorial.md`).
The `demo/` tree is a reference implementation, not authoritative (ADR-003):
the facts below describe the current checkout, not requirements.
Never store credentials here — this file is committed to git.

## Repository

- Challenge spec: root `README.md` (its Definition of Done = acceptance list)
- Demo app: `demo/` — self-documenting via `demo/README.md`, `demo/docs/README.md`, `demo/tutorial.md`
- Idea input (no authority over ADRs, see `decisions.md` ADR-002): `PLAN.md`
- Build environment: root Nix flake (`nix develop`); build through the devshell, not the user nix profile
- One-command evidence run: `demo/scripts/run_demo.sh` (needs Toxiproxy binaries in `demo/.tools/`, not committed)

## uProtocol Topics (over Zenoh; `up-rust` + `up-transport-zenoh`)

- `battery-vss/9001/1/9001` — BatteryTempEvent JSON (source → Guardian and Evidence Collector); carries the unchanged zero-based CAN generation time as `timestamp_ms`
- `battery-vss/9001/1/9003` — HighTempAlert JSON
- `guardian-vss/9000/1/9002` — GuardianSnapshot (defined in `demo/services/src/lib.rs`, not yet published)
- Guardian raw-decision topic (URI/RID still to be fixed in the interface contract) — `GuardianEvidenceEvent` JSON (product Guardian → Evidence Collector); carries every internal detection transition before and independently of DFM mapping (ADR-007)
- `guardian/1001/1/8001` — currently implemented mapped `GuardianFaultEvent`; superseded as the Evidence Collector's original-view contract by ADR-007 and to be replaced or kept only as a separate mapped stream

## Guardian Detection Constants (`demo/services/src/bin/guardian.rs`)

- WARNING ≥ 45 °C, CRITICAL ≥ 55 °C (separate HighTempAlert event ≥ 50 °C)
- Plausible range −40…125 °C; max step between consecutive samples 20 °C
- Stuck: ≥ 5 consecutive identical samples; stale: no sample for 2000 ms (watchdog polls every 500 ms)
- States: Clear/Monitoring/Warning/Critical — `Mitigating` defined in the enum but never entered; Guardian state not published over uProtocol

## Product Guardian

- Component: `product/components/guardien/`
- Rust crate: `Cargo.toml`; source files live in `source/`; binary name is `guardian`
- Model configuration: `product/config/battery_guardian/guardian_model.yaml`; startup fails on missing or inconsistent mandatory parameters
- DFM catalog: `product/config/battery_guardian/guardian_diagnostics.json`; consumed by the existing Guardian DFM reporter
- Injection model: `product/config/battery_guardian/fault_injection_model.yaml`; canonical signals are `temp_min`, `temp_avg`, `temp_max`, and `soc`
- Supported injected classes: `transport.delay`, `transport.drop`, `source.dropout`, `signal.stuck`, `signal.spike`, `signal.drift`, `signal.out_of_range`, and `signal.combination`
- Decided input contract: BatteryTempEvent on `battery-vss/9001/1/9001` carries `temp_min`, `temp_avg`, `temp_max`, `soc`, and the original CAN generation `timestamp_ms`; timestamps are integer milliseconds starting at 0 and must be preserved unchanged from the timestamped product frame to uProtocol (ADR-008/ADR-011; bridge implementation pending)
- Target Guardian timing: local monotonic receive time remains authoritative for temperature-rate and staleness checks; source `timestamp_ms` is retained for identity, evidence correlation, and transport-drop detection from unexpected gaps in the expected generation cadence, without replacing the receive-time model base
- Evaluation: 100 ms periodic cycle, 500 ms missing-packet timeout, each sample generation evaluated at most once (ADR-004)
- Guardian observations are `DetectionClass × DetectionLevel`: class identifies the model rule; level is `WARNING`, `VIOLATION`, or `CRITICAL`
- Thermal observation: `THERMAL_LIMIT / WARNING` from 60 °C to below 70 °C and `THERMAL_LIMIT / CRITICAL` from 70 °C; the warning threshold is derived from `absolute_max_c - warning_margin_c`
- Continuous checks: spread, hotspot, and temperature rate emit `WARNING` from 80% through 100% utilization and `VIOLATION` above the model limit; utilization and residual are retained
- Binary checks: stream stale; absolute temperature and ordering; SoC range/rate; excitation-gated stuck signals emit `VIOLATION` only
- SoC rate limit: 5 pp/s using actual elapsed receive time; the obsolete fixed `max_step_pp` rule has been removed
- Guardian evidence stream (decided target): every internal `Detection` transition, including unmapped utilization warnings, is published unchanged to the Evidence Collector as `GuardianEvidenceEvent`; it contains class/level, active/cleared state, signal, and available observed/limit/residual/utilization evidence, without requiring a DFM fault ID (ADR-007)
- DFM reporting: independently projects only configured class/level pairs and may aggregate signal-level detections; thermal warning/critical retain `BatteryOverTempWarning`/`BatteryOverTempCritical`, while continuous-model warnings have no DFM fault
- Current implementation gap: `guardian_faults.rs` aggregates mapped detections into `Failed`/`Passed` changes and sends those independently to DFM and as mapped `GuardianFaultEvent` messages on `guardian/1001/1/8001`; ADR-007 requires this uProtocol path to be replaced by, or separated from, the raw decision stream
- Current GuardianFaultEvent fields: `fault_id`, `detection_class`, `level`, `stage`, `baseline`, `sovd_path`, `source`, `evidence` (`signal`/`observed`/`limit`/`residual`/`utilization`, numeric); shape in `product/interfaces/battery_fault_contract.yaml`; this mapped shape is not the target original-view contract
- HTTP: port 8080 by default, `/health` and `/state`
- Dev-container workflow: `make test`, `make check`, and `make run` from the component directory invoke Cargo directly; the repository-mounted `target/` and Cargo home provide the caches
- The shared `.devcontainer` initializes the Guardian's `fault-lib` submodule, installs rustfmt and Clippy, preinstalls the Codex VS Code extension, and forwards Guardian HTTP port 8080
- `make check` also validates the canonical injection model and runs its malformed-configuration tests
- Verification on 2026-10-07: dev-container `make check` passed formatting, Clippy with warnings denied, all 49 Rust tests, injection-model validation, and all 10 validator tests

## Product Case Mutator

- Component: `product/components/case_mutator/`; Rust binary `case-mutator`
- Reads an explicit generation request, the canonical fault-injection instance and the exact `guardian_model.yaml`; records the model SHA-256 in every case
- Uses the real `battery-guardian` Rust model/runtime as its forward oracle rather than maintaining a second detection implementation
- Supports all canonical v1 classes: stuck, spike, drift, out-of-range, signal combination, transport delay/drop, and source dropout
- Emits mutated ASC, independent injection ground truth, and a Guardian test-oracle sidecar; impossible goals produce structured `UNSATISFIABLE`
- Preserves non-target ASC lines byte-exactly and never rebases embedded source timestamps on drop; transport delay changes replay time while preserving generation time
- Dev-container `make check` passes formatting, Clippy with warnings denied, 4 unit tests and 5 end-to-end generation tests

## Evidence Collector

- Subscribes directly to `battery-vss/9001/1/9001` and receives the same BatteryTempEvent payloads, including zero-based `timestamp_ms`, as the Guardian
- Subscribes to the raw `GuardianEvidenceEvent` stream and receives every original Guardian decision, including decisions omitted from or aggregated by the DFM projection; its URI/RID remains to be fixed in the interface contract
- Correlates source battery input, original Guardian decisions, injection ground truth, and DFM/OpenSOVD visibility; the direct subscriptions do not replace the diagnostic chain

## CAN Assets (`demo/can/`)

- Frame 0x100 (256) `BatteryTemperature`, 8 bytes, 100 ms cycle: CellTempAvg (bits 0–15), CellTempMax (16–31), CellTempMin (32–47), StateOfCharge (48–63); scale 0.5, offset −40 (SoC offset 0)
- `battery_temp.asc` = Vector ASC replay input for kuksa-can-provider `--dumpfile`
- `vss_dbc.json` maps signals to `Vehicle.Powertrain.TractionBattery.*` (interval 100 ms)

## Product CAN Assets (`product/config/`)

- Frame 0x100 `BatteryTemperature` is the timestamped 16-byte product frame: little-endian 32-bit `TimeStamp`, four little-endian 16-bit battery signals, and four reserved bytes preserved unchanged (ADR-011)
- `battery_temp_with_ts.asc` starts source time at 0 ms; value mutation and frame deletion preserve all remaining embedded timestamps unchanged
- Temperature and SoC quantization are 0.5 °C and 0.5 pp; nominal generation period is 100 ms

## Ports (demo)

- 7447 Zenoh (uProtocol bus) · 7448 Toxiproxy(zenoh) · 7690 OpenSOVD gateway · 8080 Guardian HTTP (`/health`, `/state`) · 8474 Toxiproxy API · 55555 kuksa-databroker

## Fault Catalog (`demo/diagnostics/catalog/battery_guardian.json`)

- `BatteryOverTempWarning`, `BatteryOverTempCritical`, `BatteryTempSignalStale`, `BatteryTempSignalStuck`, `BatteryTempImplausible`

## Known Upstream Blocker

- Vendored `demo/opensovd-core` references nonexistent crates `opensovd-cli/lib` + `opensovd-cli/build` → `cargo build -p opensovd-gateway` aborts until fixed upstream; other binaries build cleanly through the devshell
