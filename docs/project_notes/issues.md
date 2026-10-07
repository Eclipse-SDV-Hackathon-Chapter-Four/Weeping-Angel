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
- **Notes**: Added a cached Docker/Make workflow; `make check` passes formatting, Clippy, and all 23 tests. Final DFM/Evidence Collector reporting remains intentionally out of scope.
