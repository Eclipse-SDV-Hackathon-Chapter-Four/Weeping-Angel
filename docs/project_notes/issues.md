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
