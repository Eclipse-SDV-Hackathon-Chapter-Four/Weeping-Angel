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
