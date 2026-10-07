# Issues / Work Log

Short work log; details live in git history. Status: Open / In Progress / Resolved.

## Format

### YYYY-MM-DD - Short title
- **Status**: Open / In Progress / Resolved
- **Description**: 1–2 line summary
- **Notes**: Context worth remembering

### 2026-10-07 - Map CAN TimeStamp into VSS (SourceTimestamp)
- **Status**: In Progress
- **Description**: `product/config/vss_dbc.json` maps the DBC `TimeStamp` to `Vehicle.Powertrain.TractionBattery.SourceTimestamp` (`uint32`, ms); `val.proto` documents it as `Datapoint.uint32`; `product/config/vss_overlay.json` is loaded by `tools/start_databroker.sh`. `vss_bridge` now sets `timestamp_ms` from `SourceTimestamp` (one event per frame, incomplete samples dropped and logged) instead of `now_ms()`.
- **Notes**: Verified in the dev container: 5 bridge unit tests pass; replay of `battery_temp_with_ts.asc` (140 frames) yields correct source timestamps 0–13900 ms. With `interval_ms: 100` only 91 frames arrive (49 whole frames throttled by receive jitter → 200-ms gaps → false `STREAM_GENERATION_GAP`); with `interval_ms: 0` all 140 arrive. Set to `interval_ms: 50` (2026-10-07): 140/140 frames, all 100-ms deltas. Frames arriving < 50 ms apart (e.g. bursts after replay delay) are still throttled. Open: bridge proto copy differs from `product/config/proto` by the comment only.

### 2026-10-07 - run_campaign/run_case hardened with run_golden's fixes; verified end-to-end in container
- **Status**: Resolved
- **Description**: Ported the golden-run fixes into `tools/run_campaign.sh`/`tools/run_case.sh` via a shared `tools/ensure_python_env.sh` (self-provisions pyyaml+cantools/python-can venv for harness+replay): zenoh/broker port readiness (fail-closed instead of silent skips), `vss_publisher` liveness check, collector subscription wait ("replay lasts" before replaying, no more sleep-1 race), per-case collector deadline (`E2E_CASE_TIMEOUT_S`, timeout = INCONCLUSIVE fail-closed), campaign infra logs into `reports/campaign-<ts>/logs/` (`ZENOH_LOG`/`DATABROKER_LOG`). Additional enablers found during container verification: `tools/build_dfm.sh` reduced to the cargo build (the upstream `bazel build //src/...` step failed here — iceoryx2-pal-posix `socket_macros` static lib — and turned out to be dead code after the cargo build anyway; removed with an explanatory comment); `harness.py binary_paths` now probes executability (`_runnable`) instead of trusting file existence (cross-toolchain/nix binaries shared via the bind-mount exec-fail with ENOENT).
- **Notes**: Verified in container 889868c6f863 (`run_campaign --campaign signal.out_of_range --scenario warm_nominal`): build → generation → replay → verdict FAIL → summary/exit 1, per-case dump complete (collector.log shows "replay lasts 19900 ms, 5 injection(s)"). Gotchas hit and resolved on the way: docker overlay was 100% full (cleaned `docker builder prune`, 8.5 GB), root-owned build artifacts left by earlier root execs blocked uid-1100 cargo (removed), stale iceoryx2 SHM from root runs (`/tmp/iceoryx2`, `/dev/shm/iceoryx2*`) made node creation fail with InternalError (cleaned). When exec'ing builds vs runtime as different uids, artifact ownership and iceoryx2 state must be uniform or cleaned first. Later same day, tightened `tools/build_dfm.sh`: after bisecting it turned out the cargo-skip guard made the upstream `bazel build //src/...` step dead code in every path (a successful workspace build always leaves target/debug/dfm_bin). Removed bazel + USER fallback from the script with an explanatory comment; the cargo-built dfm_bin is the artifact of record.
- **Follow-up**: The campaign run's FAIL verdict is REAL collector-vs-experiment semantics, not the runner: the collector still evaluates single-class ground-truth records against the mapped 8001 stream — it does not consume the per-incident goals/`case.oracle.yaml` yet (ADR-014 open), so not-detected incidents (out_of_range incident 1/3 = exact-bound attacks) count as "no expected failure", and `dfm_slack_ms: 500` was exceeded by 15 of 18 DFM→SOVD visibilities (SOVD bridge latency, worth its own issue).

### 2026-10-07 - Campaign harness standard matrix: 8/22 experiments UNSATISFIABLE
- **Status**: Open
- **Description**: First full `harness.py generate` run over the standard matrix produced 14 GENERATED / 8 UNSATISFIABLE: `signal.stuck` (all 3 nominal scenarios) fails at incident 1 with ENCODING_LIMIT ("no DBC-representable candidate trajectory"), `signal.spike` (all 3) at incident 1 with FORBIDDEN_CODETECTION (`PHYSICAL_TEMP_RATE` fires even for the sub-threshold 1-sample delta-0.5 spike), and `signal.drift` (warm/hot) at incident 3 (the exact-threshold WARNING@1.0-touch incident).
- **Notes**: The 8 are exactly the sub-detector-threshold incident designs — the goal demands an injected, NOT-detected mutation, and the mutator either cannot encode the hold trajectory (stuck) or the nominal slope plus the smallest quantum already trips the rate detector (spike/drift). Either the mutator needs a hold-stuck encoding path, or these incidents must be respecified (e.g. larger delta at 0.5-quanta granularity). Harness itself validated + ran cleanly (`validate` = 5 scenarios/22 experiments; refuses to overwrite existing experiment dirs).

### 2026-10-07 - Golden-run orchestration runner (end2end-runner)
- **Status**: Resolved
- **Description**: Added `product/components/end2end-runner/` with `run_golden.sh` (bash, dev-container target) + README: builds components, starts zenoh/databroker/vss_publisher once, resets Guardian+DFM+SOVD per case, runs the collector per case, aggregates tri-state verdicts. Registered cases: `baseline` (nominal template, empty ground truth) and `signal_out_of_range` (mutator-generated).
- **Notes**: First full runs verified in the dev container (docker image `codium-devcontainer-weeping-angel`, repo bind-mounted, run as repo uid with PATH incl. the venv). Runner bug found+fixed: `log()` wrote to stdout and corrupted the command-substitution verdict capture (produced a false overall PASS). Verdict plane is ADR-012 (mapped stream 8001); ADR-007 raw-stream correlation and DFM/SOVD verdict dimensions are not wired yet.

### 2026-10-07 - Collector places fault events one sample early (anchoring artifact)
- **Status**: Open
- **Description**: The collector stamps `GuardianFaultEvent`s with the `timestamp_ms` of the latest battery event seen before arrival (Zenoh has no cross-topic ordering; combined with jitter, detections on the frame at a window edge land one sample behind, e.g. 1997 ms against window start 2000 ms). Identical reruns flip FAIL/PASS; golden runs made this systematic anchoring artifact visible.
- **Notes**: Decision recorded as ADR-017 — `GuardianFaultEvent`/`GuardianEvidenceEvent` gain a required `timestamp_ms` copied from the causing battery sample, and the bridge preserves the CAN `TimeStamp` instead of stamping wall-clock `now_ms()` (closes the ADR-015 ❌ bridge open point). Until implemented: contract YAML, Guardian emit path, collector `timeline()` decoding, and the live observer (ADR-016) must change together; collector keeps anchoring-based `t_ms` only as a reported fallback.

### 2026-10-07 - Golden run findings: collector timing flake + dirty nominal baseline
- **Status**: Open
- **Description**: Two real product issues surfaced from the first golden runs. (1) The collector has no timing tolerance around the injection window: one run missed `PHYSICAL_TEMP_ABSOLUTE_LIMIT` by 3 ms (detection event placed at the latest battery event, arrival jitter put it at t=1997 against window start 2000) → FAIL; an immediate rerun PASSed → verdicts are flaky without tolerance (harness spec §8 open point "accepted timing tolerance", now evidence-backed). (2) The nominal template `battery_temp_with_ts.asc` is not a clean baseline: the Guardian reports `PHYSICAL_SOC_RATE Failed` repeatedly, one `PHYSICAL_TEMP_RATE`, one `THERMAL_LIMIT` (trajectory crosses 60 °C) and one `STREAM_STALE` per replay, so a `[]`-ground-truth baseline can never PASS.
- **Notes**: (1) superseded by ADR-017 (timestamp carried on fault events; no tolerance needed). (2) candidate causes: the bridge still stamps wall-clock `now_ms()` (ADR-013 migration open, now folded into ADR-017's bridge change; jittered Δτ makes the SoC-rate limit trip), and the trajectory itself crosses THERMAL_LIMIT/WARNING — a clean baseline needs either a different nominal trace or an explicit baseline-oracle definition.

### 2026-10-06 - Evidence-report defects found during demo analysis
- **Status**: Withdrawn
- **Description**: Demo-derived observations (missing Stuck row in the checked-in report; sticky `BatteryTempSignalStale` indicator across scenarios) were withdrawn per ADR-003 — the demo is not authoritative, so demo-sample quirks do not become project issues.
- **Notes**: Kept here as a withdrawal record; the observations remain informational for the demo rebuild.

Observations against demo artifacts are informational only (ADR-003: the demo
is not authoritative) — they do not become open issues here.

### 2026-10-06 - Implement physical-model Battery Guardian
- **Status**: Resolved
- **Description**: Added the standalone product Guardian under `product/components/guardien` with validated YAML configuration, periodic receive-time evaluation, typed detections, and a dummy reporter.
- **Notes**: The initial nested-Docker Make workflow passed formatting, Clippy, and all 23 tests. On 2026-10-07 it was replaced with direct Cargo targets for the shared VS Code dev container; the component-specific `Dockerfile.dev` was removed. The initial dummy reporter was subsequently replaced by the working DFM reporter retained in ADR-005.

### 2026-10-07 - Separate Guardian configuration and injection ground truth
- **Status**: Resolved
- **Description**: Split model parameters, the existing DFM catalog, and injection ground truth into canonical artifacts under `product/config/battery_guardian`.
- **Notes**: Added `signal.combination`, a strict injection-model validator, and 10 validator tests; retained the existing DFM reporter. Dev-container `make check` passes all 27 Rust tests and the configuration checks.

### 2026-10-07 - Add orthogonal Guardian detection levels
- **Status**: Resolved
- **Description**: Added `Warning`, `Violation`, and `Critical` levels orthogonal to detection classes; thermal state now uses one `THERMAL_LIMIT` class and continuous model bounds produce utilization warnings.
- **Notes**: Thermal DFM IDs and existing physical-violation IDs remain unchanged. Spread/hotspot/rate warnings remain Guardian observations without new DFM entries and are published in the raw Guardian evidence stream under ADR-007. Thermal normalization now validates `absolute_min_c <= reference_c < hot_state_c <= absolute_max_c` without relaxing disabled SoC-coupling validation; boundary, transition, catalog, projection, and configuration tests pass in the dev-container suite.

### 2026-10-07 - Publish Guardian fault events over uProtocol
- **Status**: Resolved
- **Description**: Every DFM fault change is also published as JSON `GuardianFaultEvent` on `//guardian/1001/1/8001`; aggregation moved to `guardian_faults.rs`, DFM and uProtocol channels independent (ADR-007).
- **Notes**: Transitions plus startup baseline only. 41 Rust tests pass; Clippy (`-D warnings`) and rustfmt clean in the dev-container image. The corrected ADR-007 supersedes this mapped event as the Evidence Collector's original Guardian view; the implementation remains valid only as a separate mapped stream, not as a substitute for raw decisions.

### 2026-10-07 - Detect lost battery samples from source timestamps
- **Status**: Resolved
- **Description**: Guardian evaluates `timestamp_ms`: new `STREAM_GENERATION_GAP / VIOLATION` → DFM `BatteryTempGenerationGap` (ADR-015), rates on Δτ, sample queue instead of overwrite, `interval_ms`/`timestamp_ms` evidence.
- **Notes**: 59 Guardian tests, fmt and Clippy clean in `battery-guardian-dev:local`; mutator oracle and drop goals allow the gap co-detection; Evidence Collector accepts it for `transport.drop`. Open: VSS bridge wall-clock stamping causes false gaps live; duplicate/out-of-order detection not implemented.

### 2026-10-07 - Align source timestamps and Evidence Collector subscriptions
- **Status**: In Progress
- **Description**: Decided that every battery CAN message has a zero-based millisecond generation timestamp preserved into BatteryTempEvent; the Evidence Collector subscribes to battery events, every raw Guardian decision, and the independently produced DFM/OpenSOVD messages.
- **Notes**: ADR-004/007/008/009/010 and key facts now define the three evidence planes. Implementation still needs to replace bridge wall-clock timestamps, retain the source timestamp in the Guardian, use source-timestamp gaps for drop detection, define the raw GuardianEvidenceEvent URI/RID and payload contract, publish all detection transitions before DFM mapping, and add the Evidence Collector subscriptions.

### 2026-10-07 - Reconcile Case Mutator specification with Guardian model
- **Status**: Resolved
- **Description**: Implemented the Rust Case Mutator under `product/components/case_mutator` with bounded inverse search, quantization, real Guardian forward verification, ASC rendering, ground truth, oracle output, and structured unsatisfiable results.
- **Notes**: All eight canonical v1 injections are covered by end-to-end generation tests. The Guardian and canonical configuration now use the specified 5 pp/s source/generation-interval SoC rate (ADR-013). Product frame 0x100 is CAN FD with DLC code `0xA`, 16-byte payload, and a little-endian timestamp under ADR-011. Component and Guardian `make check` both passed at resolution time.

### 2026-10-07 - Specify battery campaign test harness
- **Status**: In Progress
- **Description**: Define five 20-second reference scenarios, multi-incident elementary-fault campaigns, pre-generated experiment artifacts, and the later Evidence Collector based execution flow.
- **Notes**: ADR-014 and `product/doc/testing/battery_campaign_test_harness.md` capture the agreed framework. Golden baselines, standard five-case profiles, timing slots, isolation, readiness, evidence windows, reporting, and the DevContainer Python runner are decided. Every Golden ASC now has same-prefix empty Ground Truth and an exact transition Oracle; Overtemp and Hotspot include their real fault timings. The compact configuration is split into harness selection, default campaigns, and explicit combined incidents; experiment files use a Collector-compatible `case` prefix. Campaign artifacts require no hashes. Transport delay/drop are deterministic ASC/CAN replay mutations; the product uses no Toxiproxy. Remaining specification work covers generated artifact/verdict schemas, readiness payloads, and runner/Make commands. The current Case Mutator still emits a model SHA-256 and supports one injection per request; the current Collector is mapped-only and still treats empty ground truth as a no-fault baseline; both must be reconciled.

### 2026-10-06 - Component & channel specification (product/doc/architecture)
- **Status**: In Progress
- **Description**: Draft `product/doc/architecture/components_and_channels.md`: component overview (C1–C15), edge overview (E1–E18), short per-component/channel specs, and option analyses for mitigation, DFM IPC transport, `run_id` entry, collector correlation and report generation.
- **Notes**: Decisions recorded as ADR-004 (source timestamp as identity and for generation-gap/drop detection, local receive time for timeout/rate evaluation; supersedes ADR-001), ADR-005 (DFM reinstated), ADR-006 (v1 scope), ADR-007 (mitigation M1, iceoryx2 DFM IPC, `run_id` A+C, `verdict.json`→MD) and ADR-008 (contract YAML + model doc authoritative). Doc deepened to payload level with sequence diagram, fault-class mapping and failure-mode matrix. Product transport faults are now ASC/CAN replay mutations; the earlier Toxiproxy assumption is removed. Still open: periodic Guardian state/snapshot RID (if any). `transport.duplicate`/`STREAM_DUPLICATE` exists in the contract but remains outside the implemented Mutator scope.

### 2026-10-07 - Evidence Collector on GuardianFaultEvent stream
- **Status**: In Progress
- **Description**: Collector rewritten for contract v3 (ADR-012): subscribes to `//guardian/1001/1/8001` + `//battery-vss/9001/1/9001`, mutator ground-truth record, window verdict, `expected_observations.yaml`.
- **Notes**: 18 unit tests and fake-publisher Zenoh runs pass. Open: VSS bridge still sends wall-clock `timestamp_ms` (ADR-013); `signal.combination` has no expectation; switch to raw `GuardianEvidenceEvent` once published.

### 2026-10-07 - Adopt relative timestamps as the pipeline-wide common time base
- **Status**: Open
- **Description**: ADR-013 supersedes ADR-008 and the receive-time base clause of ADR-004: the zero-based source `timestamp_ms` becomes the single common time base for the whole chain, and every component (including DFM/OpenSOVD) carries it unchanged.
- **Notes**: Requires a projected relative clock at each consumer (offset calibrated from arrivals) so staleness/transport delay stay observable. Follow-up scope: contract timestamp semantics; `components_and_channels.md`, `battery_guardian_model.md`, `case_mutator_model.md` (§7/§9.10/§14–§16/§21); reconcile `missing_packet_timeout_ms` (500 vs. 2000, mutator open point D); migrate Guardian `received_at: Instant`, `guardian_runtime.rs` and `guardian_uprotocol.rs`; stop `vss_bridge` `SystemTime` stamping and define how the relative timestamp crosses KUKSA Data Broker/VSS; re-scope the "Align source timestamps and Evidence Collector subscriptions" work item. Doc-level disambiguation applied: `battery_guardian_model.md`, `case_mutator_model.md`, `components_and_channels.md` now distinguish the source/generation interval Δτ from the receive interval Δt_recv (both on the relative axis).

### 2026-10-07 - ASC plotting script and Nix Python environment
- **Status**: Resolved
- **Description**: Added `product/scripts/plot-asc`, a small matplotlib script that decodes the 0x100 BatteryTemperature frame (little-endian payload: CellTempAvg/Max/Min at scale 0.5/offset -40, StateOfCharge at scale 0.5) and plots temperatures plus SoC. It handles both the plain 8-byte demo frame and the 16-byte CAN FD product frame.
- **Notes**: `flake.nix` devshell now provides `pkgs.python3.withPackages [ matplotlib ]` (Python 3.14.7, matplotlib 3.11.1); verified `nix develop --command python3 product/scripts/plot-asc` on both ASC files. No ADR change.

### 2026-10-07 - Battery CAN frames as CAN FD
- **Status**: Resolved
- **Description**: `product/config/battery_temp_with_ts.asc` converted to canonical CAN FD lines with DLC code `0xA` and 16-byte payload; Classic CAN replay was cut to 8 bytes by python-can, losing `CellTempMin` and `StateOfCharge`. The Case Mutator parses/renders both line formats while product replays use CAN FD.
- **Notes**: Replay via `product/components/start_can.sh` delivers all four signals; mutator and collector tests pass. The implemented format is now normative in `product/doc/can/battery_can_fd_replay.md`. Collector reads the mutator's `<stem>.ground_truth.yaml` and its `source_started_at_ms`/`source_finished_at_ms` window.

### 2026-10-07 - Specify live scenario observer for the Evidence Collector
- **Status**: Resolved
- **Description**: Added `product/doc/observer/live_observer.md` and ADR-016: a read-only Live Scenario Observer as a feature-flagged module (`observer`, `--observer`) inside the Evidence Collector binary, fed from in-process state and served to the browser via HTTP + SSE on `127.0.0.1:8090` with embedded static assets.
- **Notes**: Uses only present data — battery stream and mapped `GuardianFaultEvent` (`//guardian/1001/1/8001`) — plus the ADR-014 bundle (`ground_truth.yaml`, `oracle.yaml`); reuses the `battery-guardian` library for static bands; relative-time axis per ADR-013. Not DoD-critical. Known v1 limits: unmapped Guardian warnings invisible (bug logged) and step-after dynamic limits. Follow-ups: ADR-012 amendment for `oracle.yaml`, raw decision stream (ADR-007), `key_facts.md`/`components_and_channels.md`/E19 updated.

### 2026-10-07 - Implement the Live Scenario Observer (ADR-016)
- **Status**: Resolved
- **Description**: Implemented the observer as a Cargo feature `observer` in `product/components/evidence_collector`: module `src/observer/` with the 20 s ring buffer, SSE stream (snapshot-first, uncoalesced `sample`/`detection` deltas), embedded static frontend (`index.html`, `app.js`, `style.css`), and static bands derived from `guardian_model.yaml` via the `battery-guardian` library. CLI gains `--observer`, `--observer-addr`, `--guardian-model`; the subscription loop now feeds a live sink; `FaultEvent` carries optional `evidence` (`interval_ms`/`timestamp_ms`), `BatteryEvent` carries the signal values, and ground truth passes mutations through.
- **Notes**: 25 tests pass (`cargo test --features observer`), incl. an SSE integration test; default build (feature off) is unchanged with no warnings; both builds pass `--locked`. Smoke-tested against the `end2end-runner` baseline and `signal_out_of_range` cases (snapshot carries bands, incidents, and oracle). Pre-existing rustfmt/clippy deviations in `main.rs` (HEAD, unrelated) remain and were not reformatted. Runner integration: `E2E_OBSERVER=1` in `end2end-runner` builds the collector with the feature and passes `--observer` (`E2E_OBSERVER_ADDR` overrides the address).

### 2026-10-07 - Live observer default bind address set to 0.0.0.0:8090
- **Status**: Resolved
- **Description**: Changed the Live Scenario Observer's default listening address from `127.0.0.1:8090` to `0.0.0.0:8090` (`DEFAULT_ADDR` in `evidence_collector/src/observer/mod.rs`). Updated the runner default `E2E_OBSERVER_ADDR`, the `live_observer.md` contract, `info.md`, `components_and_channels.md` (E19), `key_facts.md`, and the end2end-runner README. In `run_golden.sh` the health check and logged URL now substitute `0.0.0.0` with `127.0.0.1` so the displayed URL stays browser-usable.
- **Notes**: `cargo build --features observer` passes; `bash -n run_golden.sh` clean. Recorded as a dated amendment to ADR-016 (see `decisions.md`). Binds all interfaces, so the observer is reachable from the network; override with `--observer-addr 127.0.0.1:8090` for host-local runs. Not DoD-critical (v1 extension).

### 2026-10-07 - run_campaign starts the live observer per case
- **Status**: Resolved
- **Description**: `tools/run_case.sh` gained the `E2E_OBSERVER`/`E2E_OBSERVER_ADDR` env handling already present in the end2end-runner: when `E2E_OBSERVER=1` it builds the collector with `--features observer`, passes `--observer --observer-addr`, and waits for `http://<loopback>/health` before replay. `tools/run_campaign.sh` now defaults `E2E_OBSERVER=1` (exported to `run_case.sh`), builds the collector with the feature, and logs the observer URL per case; set `E2E_OBSERVER=0` to disable.
- **Notes**: `bash -n` clean on both scripts. Per ADR-016 the collector serves one experiment, so the observer restarts with each case (same behavior as the end2end-runner). Observer startup is non-fatal to the verdict. Docs updated: `key_facts.md`, script headers.
