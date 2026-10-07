# Issues / Work Log

Short work log; details live in git history. Status: Open / In Progress / Resolved.

## Format

### YYYY-MM-DD - Short title
- **Status**: Open / In Progress / Resolved
- **Description**: 1–2 line summary
- **Notes**: Context worth remembering

### 2026-10-07 - Golden-run orchestration runner (end2end-runner)
- **Status**: Resolved
- **Description**: Added `product/components/end2end-runner/` with `run_golden.sh` (bash, dev-container target) + README: builds components, starts zenoh/databroker/vss_publisher once, resets Guardian+DFM+SOVD per case, runs the collector per case, aggregates tri-state verdicts. Registered cases: `baseline` (nominal template, empty ground truth) and `signal_out_of_range` (mutator-generated).
- **Notes**: First full runs verified in the dev container (docker image `codium-devcontainer-weeping-angel`, repo bind-mounted, run as repo uid with PATH incl. the venv). Runner bug found+fixed: `log()` wrote to stdout and corrupted the command-substitution verdict capture (produced a false overall PASS). Verdict plane is ADR-012 (mapped stream 8001); ADR-007 raw-stream correlation and DFM/SOVD verdict dimensions are not wired yet.

### 2026-10-07 - Golden run findings: collector timing flake + dirty nominal baseline
- **Status**: Open
- **Description**: Two real product issues surfaced from the first golden runs. (1) The collector has no timing tolerance around the injection window: one run missed `PHYSICAL_TEMP_ABSOLUTE_LIMIT` by 3 ms (detection event placed at the latest battery event, arrival jitter put it at t=1997 against window start 2000) → FAIL; an immediate rerun PASSed → verdicts are flaky without tolerance (harness spec §8 open point "accepted timing tolerance", now evidence-backed). (2) The nominal template `battery_temp_with_ts.asc` is not a clean baseline: the Guardian reports `PHYSICAL_SOC_RATE Failed` repeatedly, one `PHYSICAL_TEMP_RATE`, one `THERMAL_LIMIT` (trajectory crosses 60 °C) and one `STREAM_STALE` per replay, so a `[]`-ground-truth baseline can never PASS.
- **Notes**: (1) candidate fix: small tolerance (e.g. 100–200 ms) around window edges in the collector, or event placement anchored to the next battery event instead of the latest; needs a spec decision. (2) candidate causes: the bridge still stamps wall-clock `now_ms()` (ADR-013 migration open; jittered Δτ makes the SoC-rate limit trip), and the trajectory itself crosses THERMAL_LIMIT/WARNING — a clean baseline needs either a different nominal trace or an explicit baseline-oracle definition.

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
