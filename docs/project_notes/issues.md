# Issues / Work Log

Short work log; details live in git history. Status: Open / In Progress / Resolved.

## Format

### YYYY-MM-DD - Short title
- **Status**: Open / In Progress / Resolved
- **Description**: 1–2 line summary
- **Notes**: Context worth remembering

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
- **Status**: In Progress
- **Description**: Updated the mutator specification to consume and validate the canonical Guardian model, record its content hash, use shared conformance vectors, and reflect source timestamps plus the three Evidence Collector views.
- **Notes**: Corrected stale timeout, rate-boundary, stuck-target and combination wording without changing model semantics. Open decisions remain for mutation-layer attribution, ground-truth time bases, timestamp-gap detection output, executable classes not present in the injection vocabulary, and the conflicting SoC step/rate definitions.

### 2026-10-06 - Component & channel specification (product/doc/architecture)
- **Status**: In Progress
- **Description**: Draft `product/doc/architecture/components_and_channels.md`: component overview (C1–C15), edge overview (E1–E18), short per-component/channel specs, and option analyses for mitigation, DFM IPC transport, `run_id` entry, collector correlation and report generation.
- **Notes**: Decisions recorded as ADR-004 (source timestamp as identity and for generation-gap/drop detection, local receive time for timeout/rate evaluation; supersedes ADR-001), ADR-005 (DFM reinstated), ADR-006 (v1 scope), ADR-007 (mitigation M1, iceoryx2 DFM IPC, `run_id` A+C, `verdict.json`→MD, Toxiproxy) and ADR-008 (contract YAML + model doc authoritative). Doc deepened to payload level with sequence diagram, fault-class mapping and failure-mode matrix. Still open: periodic Guardian state/snapshot RID (if any). Transport reorder deferred per ADR-009 (no native Toxiproxy toxic). `transport.duplicate`/`STREAM_DUPLICATE` added to the contract YAML.
