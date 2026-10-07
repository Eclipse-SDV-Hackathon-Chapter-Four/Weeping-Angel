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
- **Notes**: Thermal DFM IDs and existing physical-violation IDs remain unchanged. Spread/hotspot/rate warnings remain internal observations without new DFM entries; boundary, transition, catalog, and projection tests pass in the 37-test dev-container suite.

### 2026-10-07 - Publish Guardian fault events over uProtocol
- **Status**: Resolved
- **Description**: Every DFM fault change is also published as JSON `GuardianFaultEvent` on `//guardian/1001/1/8001`; aggregation moved to `guardian_faults.rs`, DFM and uProtocol channels independent (ADR-007).
- **Notes**: Transitions plus startup baseline only. 41 Rust tests pass; Clippy (`-D warnings`) and rustfmt clean in the dev-container image.

### 2026-10-06 - Component & channel specification (product/doc/architecture)
- **Status**: In Progress
- **Description**: Draft `product/doc/architecture/components_and_channels.md`: component overview (C1–C15), edge overview (E1–E18), short per-component/channel specs, and option analyses for mitigation, DFM IPC transport, `run_id` entry, collector correlation and report generation.
- **Notes**: Decisions recorded as ADR-004 (source timestamp as identity, receive-timeout detection; supersedes ADR-001), ADR-005 (DFM reinstated), ADR-006 (v1 scope), ADR-007 (mitigation M1, iceoryx2 DFM IPC, `run_id` A+C, `verdict.json`→MD, Toxiproxy) and ADR-008 (contract YAML + model doc authoritative). Doc deepened to payload level with sequence diagram, fault-class mapping and failure-mode matrix. Still open: periodic Guardian state/snapshot RID (if any). Transport reorder deferred per ADR-009 (no native Toxiproxy toxic). `transport.duplicate`/`STREAM_DUPLICATE` added to the contract YAML.
