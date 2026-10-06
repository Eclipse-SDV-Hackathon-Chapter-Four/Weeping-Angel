# Architectural Decision Records

Chronological log of binding architectural decisions for this repo.

**Authority rule (ADR-002, ADR-003):** ADRs in this file are authoritative.
`PLAN.md` is an idea input and the `demo/` tree a reference implementation —
neither has authority over ADRs; where they conflict, the ADR governs.

## Format

Each decision records: date, context, decision, rejected alternatives, and
consequences (✅/❌). Number sequentially (ADR-001, ADR-002, ...).

### ADR-001: No sequence numbers — Guardian heartbeat counter as time base (2026-10-06)

**Context:**
- Fault-campaign design (PLAN.md, Step 1) considered value faults on a per-message sequence number ("seq-nr") to make message loss, repetition, and ordering detectable.
- The CAN asset has no room for one: frame `BO_ 256 BatteryTemperature` (0x100, 8 bytes, `demo/can/battery_temp.dbc`) is fully packed with CellTempAvg / CellTempMax / CellTempMin / StateOfCharge (4 × 16 bit); `battery_temp.asc` carries no counter either.
- A seq-nr without DBC/frame + VSS-mapping + event-contract changes would be a campaign against a signal that never reaches the Guardian (vacuous fault injection).

**Decision:**
- No sequence number will be introduced — not in the DBC/frame, not in the VSS mapping, not in `BatteryTempEvent`.
- The Guardian uses its own heartbeat as the time base and counts heartbeats. Message loss, repetition, ordering, and staleness/jitter are measured relative to this heartbeat count.

**Alternatives Considered:**
- Add a seq-nr signal to the DBC/frame and map it through to `BatteryTempEvent` → Rejected: frame is full (8/8 bytes used); contract churn across CAN assets, mapping, and event schema for a single fault class.
- Use payload timestamps (`BatteryTempEvent.timestamp_ms`) as the time base → Rejected: producer-controlled wall clock is not monotonic under replay/pause and is exactly what fault campaigns manipulate.
- Use ASC replay timestamps → Rejected: only available at the source layer, not at the Guardian; conflicts with the architecture law (Guardian sees uProtocol only).

**Consequences:**
- ✅ No changes to CAN assets, DBC mapping, or the event contract.
- ✅ Time base lives inside the Guardian — staleness/jitter/loss checks become heartbeat-relative instead of wall-clock-dependent.
- ❌ Delay/reorder/duplicate faults are observable only as heartbeat-gap anomalies (expected vs. counted beats), not per-message sequence jumps.
- ❌ Open design point: the heartbeat event (URI, rate, format) does not exist yet (see `demo/services/src/lib.rs` topic list) — must be defined and published before heartbeat-based campaigns can run.

### ADR-002: ADRs take precedence over PLAN.md (2026-10-06)

**Context:**
- `PLAN.md` collects ideas and proposed steps; it is edited freely and can drift from decisions already made.
- Agents and contributors need a single source of truth for what is decided.

**Decision:**
- `docs/project_notes/decisions.md` (this file) is authoritative for architecture and scope decisions.
- `PLAN.md` has no authority over ADRs: where PLAN.md conflicts with an ADR, the ADR governs; PLAN.md is an idea backlog to be reconciled, not a competing spec.

**Alternatives Considered:**
- Keep PLAN.md as the single spec → Rejected: mixes decided state with open ideas.
- Mirror every ADR back into PLAN.md immediately → Rejected: duplication drifts; reconciliation is a separate, explicit step.

**Consequences:**
- ✅ Decisions survive PLAN.md rewrites and idea churn.
- ✅ Agents can resolve conflicts mechanically (ADR wins).
- ❌ PLAN.md can be temporarily out of sync (e.g., its Step 1 still lists seq-nr, superseded by ADR-001).

### ADR-003: The existing demo is not authoritative (2026-10-06)

**Context:**
- `demo/` contains a working reference implementation (two parallel stacks, Robot suite, sample report) that predates the current planning round and will be rebuilt and extended.
- Treating demo behavior as the spec — its fault coverage, its sample `evidence_report.md`, its catalog contents — would freeze accidental early choices into requirements.
- Observations derived from demo artifacts had been logged as a project issue (evidence-report defects); that framing is wrong — the demo documents what is, not what must be.

**Decision:**
- The demo is a reference implementation and starting point — informative, not authoritative.
- Requirements come from the challenge `README.md` (Definition of Done = acceptance criteria) and our ADRs; deviations from the demo are permitted and need no demo-side justification.
- Observations against demo artifacts (e.g., stale checked-in report contents, demo suite quirks, its fault-class coverage) are informational only and must not become project issues or constraints.

**Alternatives Considered:**
- Keep demo behavior as the de-facto spec → Rejected: freezes accidental early choices; blocks the ASC-mutation rearchitecture.
- Promote demo docs to normative → Rejected: they describe current state, not targets.

**Consequences:**
- ✅ Free hand to rebuild the demo (unify stacks, ASC-level injection) without demo-compatibility obligations.
- ✅ Project memory stays free of demo-sample quirks logged as issues.
- ❌ Demo code/docs can drift from decisions until reconciled — current decisions live in `decisions.md`, not in `demo/`.
