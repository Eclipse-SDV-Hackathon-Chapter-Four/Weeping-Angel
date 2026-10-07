# Bug Log

Bugs with solutions, brief and dated. Keep entries 2–3 lines; drop entries
that stopped being relevant (6+ months).

## Format

### YYYY-MM-DD - Brief Bug Description
- **Issue**: What went wrong
- **Root Cause**: Why it happened
- **Solution**: How it was fixed
- **Prevention**: How to avoid it in the future

### 2026-10-07 - Case Mutator supports only a single injection per case
- **Issue**: `case_mutator` cannot generate multi-injection experiments: `GenerationRequest` resolves exactly one `injection_id`/`injected_class`, and `generate()` emits one `GroundTruth` with one window plus one `OracleSidecar`. ADR-014 requires five ordered incidents per experiment ASC; the Evidence Collector already accepts a ground-truth record list, so the gap is mutator-side only.
- **Root Cause**: Mutator was built for the single-injection v0.2.0 case format, before ADR-014 fixed the multi-incident campaign model.
- **Solution**: Open. Extend the request schema to an ordered incident list (per-incident window, mutations, generation goal) and emit ground truth as a record list; keep single-injection requests working.
- **Prevention**: Align component capabilities with ADR consequences (ADR-014 ❌ items) before planning new campaign work.

(no other entries yet)

### 2026-10-07 - Unmapped Guardian warning detections are invisible to the collector
- **Issue**: Only DFM-mapped `class/level` pairs are published on `//guardian/1001/1/8001`, so the continuous-model warnings (`PHYSICAL_TEMP_SPREAD`/`PHYSICAL_TEMP_HOTSPOT`/`PHYSICAL_TEMP_RATE` `WARNING`) never reach the Evidence Collector or the observer UI. ADR-006/007 require them as original Guardian decisions.
- **Root Cause**: The raw `GuardianEvidenceEvent` stream (`//guardian-vss/9000/1/9002`, ADR-007) is specified but still unimplemented; today only the mapped `GuardianFaultEvent` path exists.
- **Solution**: Open. Implement the raw evidence stream and have the collector subscribe to it alongside the mapped topic; v1 deliberately accepts the visibility gap (tracked under ADR-007).
- **Prevention**: Treat the raw decision view as a first-class contract whenever detection vocabulary grows, so unmapped warnings stay observable by design.
