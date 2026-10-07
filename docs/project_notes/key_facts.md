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
- Legacy demo evidence run: `demo/scripts/run_demo.sh`; its Toxiproxy-based transport scenario is historical and is not part of the product harness

## uProtocol Topics (over Zenoh; `up-rust` + `up-transport-zenoh`)

- `battery-vss/9001/1/9001` — BatteryTempEvent JSON (source → Guardian and Evidence Collector); carries the unchanged zero-based CAN generation time as `timestamp_ms`
- `battery-vss/9001/1/9003` — HighTempAlert JSON
- `guardian-vss/9000/1/9002` — GuardianSnapshot (defined in `demo/services/src/lib.rs`, not yet published)
- `guardian-vss/9000/1/9003` — GuardianEvidenceEvent JSON (product Guardian → Evidence Collector); raw evidence stream with every internal detection transition, mapped or not, published before and independently of the DFM projection; required fields `run_id` (Guardian startup configuration, `GUARDIAN_RUN_ID`), `detection_class`, `level`, `stage` (`active`/`cleared`), optional `signal`, `evidence` (observed/limit/residual/utilization/interval_ms) and `timestamp_ms` (ADR-017: omitted when there is no causing sample); implemented in `product/components/guardien/source/guardian_uprotocol.rs` (`spawn_evidence`, ADR-007)
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
- Decided input contract: BatteryTempEvent on `battery-vss/9001/1/9001` carries `temp_min`, `temp_avg`, `temp_max`, `soc`, and the original CAN generation `timestamp_ms`; the product frame 0x100 carries it explicitly (ADR-011). Timestamps are integer milliseconds starting at 0, are the pipeline-wide common time base, and must be preserved unchanged from the product frame to uProtocol and beyond (ADR-013; bridge implementation pending)
- Target Guardian timing: the relative `timestamp_ms` is the common time base; the model distinguishes the source/generation interval $\Delta\tau$ (from `ts`, for rate/dynamics) from the receive interval $\Delta t^{\mathrm{recv}}$ projected onto the same base (for freshness/age, drop/jitter, duplicate/reorder). Local receive time is not a separate model base (ADR-013)
- Evaluation: 100 ms periodic cycle, 500 ms missing-packet timeout, received samples queued and each evaluated exactly once; all timing on the relative base (ADR-013)
- Lost samples: Δτ > `max_generation_interval_ms` (150 ms) → `STREAM_GENERATION_GAP / VIOLATION` → DFM `BatteryTempGenerationGap` (ADR-015)
- Guardian observations are `DetectionClass × DetectionLevel`: class identifies the model rule; level is `WARNING`, `VIOLATION`, or `CRITICAL`
- Thermal observation: `THERMAL_LIMIT / WARNING` from 60 °C to below 70 °C and `THERMAL_LIMIT / CRITICAL` from 70 °C; the warning threshold is derived from `absolute_max_c - warning_margin_c`
- Continuous checks: spread, hotspot, and temperature rate emit `WARNING` from 80% through 100% utilization and `VIOLATION` above the model limit; utilization and residual are retained
- Binary checks: stream stale; absolute temperature and ordering; SoC range/rate; excitation-gated stuck signals emit `VIOLATION` only
- SoC rate limit: 5 pp/s using the source/generation interval $\Delta\tau$ (ADR-013); the obsolete fixed `max_step_pp` rule has been removed
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
- Reads an explicit generation request, the canonical fault-injection instance and the configured `guardian_model.yaml`; ADR-014 removes model, configuration, trace, and artifact hashes from the target campaign artifacts, while the current implementation still emits a model SHA-256 pending reconciliation
- Uses the real `battery-guardian` Rust model/runtime as its forward oracle rather than maintaining a second detection implementation
- Supports all canonical v1 classes: stuck, spike, drift, out-of-range, signal combination, transport delay/drop, and source dropout
- Emits mutated ASC, independent injection ground truth, and a Guardian test-oracle sidecar; impossible goals produce structured `UNSATISFIABLE`
- Preserves non-target ASC lines byte-exactly and never rebases embedded source timestamps on drop; transport delay changes replay time while preserving generation time
- Product transport delay/drop operate on the ASC/CAN replay path before the CAN Provider; no Toxiproxy or Zenoh-link mutation is used
- Current Rust tests pass 6 unit tests and 5 end-to-end generation tests; the current checkout still needs `cargo fmt` before the complete `make check` is green

## Battery Campaign Test Harness

- Normative discussion specification: `product/doc/testing/battery_campaign_test_harness.md`; CAN FD replay contract: `product/doc/can/battery_can_fd_replay.md`
- Five version-controlled 20-second reference scenarios at the nominal 100-ms cycle: three nominal operating states (`cold_nominal`, `warm_nominal`, `hot_nominal`) and two genuine fault states (`overtemp_fault`, `hotspot_fault`)
- Golden trajectories are deterministic committed ASC data with same-prefix Ground Truth and Oracle YAMLs. Ground Truth is empty because nothing is injected; Oracle YAMLs carry the exact expected transitions, including recurring Overtemp Rate warnings. `validation.json` is a derived review summary. Hot nominal intentionally carries a Thermal Warning, while the two genuine-fault regressions may remain active at replay end
- Initial elementary-fault campaigns mutate only the three nominal scenarios; the two fault scenarios are unmodified positive regression runs until combined-fault testing is explicitly specified
- Seven elementary campaigns run across the three nominal scenarios, plus one initially configured combined campaign on `warm_nominal`, for 22 experiments; each experiment contains five separated incidents with explicit recovery and expectations
- Standard variation is class-specific and uses five fixed positions; even scheduling provides a 1-s lead-in, five 3.5-s incident/recovery slots, and a 1.5-s final drain
- Warning expectations are valid only for Guardian checks that define warnings; binary checks use subthreshold/boundary/violation-style cases rather than inventing warning levels
- The harness pre-generates experiment bundles containing the ASC replay, injection ground truth, and oracle. No hash or provenance artifact is generated
- Harness source configuration lives in `product/config/battery_campaign`: `harness.yaml` selects scenarios and campaign files, `default_campaigns.yaml` expresses frequency and variation compactly, and explicit combined campaigns list their individual incidents
- Experiment bundles share the Collector-compatible `case` prefix and keep runtime results in an `evidence/` subdirectory
- The v1 campaign runner is Python inside the DevContainer. Each experiment restarts the stateful chain, uses fresh DFM storage, waits for machine-readable readiness, starts the Collector before replay, and drains evidence for 3 s after confirmed replay completion
- Evidence timing uses exact battery source timestamps, 100-ms Guardian slack, and 500-ms DFM projection slack. Infrastructure failures are `INCONCLUSIVE`; campaign execution continues by default
- Every execution gets a unique `evidence/<run-id>/` directory containing Collector JSON, normative `verdict.json`, derived `report.md`, logs, and DFM state; existing runs are not overwritten
- Future execution runs one experiment at a time and correlates battery input, raw Guardian decisions, and DFM/OpenSOVD evidence; verdicts are `PASS`, `FAIL`, or `INCONCLUSIVE`

## Evidence Collector

- Subscribes directly to `battery-vss/9001/1/9001` and receives the same BatteryTempEvent payloads, including zero-based `timestamp_ms`, as the Guardian
- Subscribes to the raw `GuardianEvidenceEvent` stream and receives every original Guardian decision, including decisions omitted from or aggregated by the DFM projection; its URI/RID remains to be fixed in the interface contract
- Correlates source battery input, original Guardian decisions, injection ground truth, and DFM/OpenSOVD visibility; the direct subscriptions do not replace the diagnostic chain
- Live Scenario Observer: feature-flagged module (`observer`) plus CLI option `--observer` inside the collector binary; serves a read-only static SSE frontend from in-process collector state (ADR-016, `product/doc/observer/live_observer.md`); the end2end-runner starts it with `E2E_OBSERVER=1`, and `tools/run_campaign.sh` starts it by default (`E2E_OBSERVER=0` disables)

## Evidence Reporter (implemented v1 — ADR-018)

- Component: `product/components/evidence_reporter/` (Python, stdlib; PyYAML optional); CLI `source/evidence_reporter.py run|campaign`; normative spec `product/doc/reporting/evidence_report.md`
- Renders the current collector `report.json` (identities derived from `reports/campaign-<ts>/<campaign>--<scenario>/` paths + `experiment.yaml`) into per-run `report.md` and campaign `evidence_report.md`; outputs not committed
- Joins the fault catalog from `product/config/battery_guardian/guardian_diagnostics.json`; GitHub-safe Markdown, presentation only
- Wired into `tools/run_case.sh` (per run) and `tools/run_campaign.sh` (campaign)
- Observer export (ADR-018 amendment to ADR-016): `GET /snapshot.json`, `GET /export.html`, `--dump-html FILE` on port 8090; `run_case.sh` passes `--dump-html`, the reporter links `observer.html` and embeds `observer.png`
- Missing chain links are explicit placeholders: mitigation (event-only), SOVD `testFailed`/`confirmedDtc`/`warningIndicator` (not captured), unmapped Guardian warnings

## CAN Assets (`demo/can/`)

- Frame 0x100 (256) `BatteryTemperature`, 8 bytes, 100 ms cycle: CellTempAvg (bits 0–15), CellTempMax (16–31), CellTempMin (32–47), StateOfCharge (48–63); scale 0.5, offset −40 (SoC offset 0)
- `battery_temp.asc` = Vector ASC replay input for kuksa-can-provider `--dumpfile`
- `vss_dbc.json` maps signals to `Vehicle.Powertrain.TractionBattery.*` (interval 100 ms)

## Product CAN Assets (`product/config/`)

- Frame 0x100 `BatteryTemperature` is CAN FD with standard 11-bit ID, DLC code `0xA`, and a 16-byte payload: little-endian 32-bit `TimeStamp`, four little-endian 16-bit battery signals, and four reserved bytes preserved unchanged; `TimeStamp` is the pipeline-wide common time base (ADR-011, ADR-013); the normative ASC form is `CANFD … 0 0 a 16 <bytes>`
- `vss_dbc.json` maps the CAN `TimeStamp` to `Vehicle.Powertrain.TractionBattery.SourceTimestamp` (`uint32`, ms, `Datapoint.uint32`); the path is not in VSS 6.0 and comes from `vss_overlay.json`, which `tools/start_databroker.sh` loads via `--vss <catalogue>,<overlay>`; all product mappings use `interval_ms: 50` (100 dropped jittered frames)
- `battery_temp_with_ts.asc` starts source time at 0 ms; value mutation and frame deletion preserve all remaining embedded timestamps unchanged
- Temperature and SoC quantization are 0.5 °C and 0.5 pp; nominal generation period is 100 ms

## Ports (legacy demo)

- 7447 Zenoh (uProtocol bus) · 7448 Toxiproxy(zenoh) · 7690 OpenSOVD gateway · 8080 Guardian HTTP (`/health`, `/state`) · 8474 Toxiproxy API · 55555 kuksa-databroker
- 8090 Live Scenario Observer HTTP/SSE (`0.0.0.0`, ADR-016), only when the collector runs with `--observer`

## Fault Catalog (`demo/diagnostics/catalog/battery_guardian.json`)

- `BatteryOverTempWarning`, `BatteryOverTempCritical`, `BatteryTempSignalStale`, `BatteryTempSignalStuck`, `BatteryTempImplausible`

## Known Upstream Blocker

- Vendored `demo/opensovd-core` references nonexistent crates `opensovd-cli/lib` + `opensovd-cli/build` → `cargo build -p opensovd-gateway` aborts until fixed upstream; other binaries build cleanly through the devshell
