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

### ADR-004: Guardian uses receive-time periodic evaluation (2026-10-06)

**Context:**
- ADR-001 selected a future Guardian heartbeat as the time base, but the heartbeat event, URI, and publisher do not exist.
- The supplied Battery Guardian implementation specification requires the listener to store observations only, a periodic task to evaluate each sample generation at most once, and stream staleness to use elapsed monotonic receive time.
- The new product implementation lives independently of the non-authoritative threshold-based demo.

**Decision:**
- The product Guardian uses local monotonic receive timestamps (`Instant`) and a monotonically increasing sample-generation counter.
- A configurable periodic task evaluates each fresh generation once. It reports `STREAM_STALE` when receive age exceeds the configured timeout and clears it on the next fresh sample.
- No producer timestamp or sequence number is added to `BatteryTempEvent`; all physical-model parameters come from the Guardian YAML configuration.
- This receive-time decision supersedes ADR-001's not-yet-implemented heartbeat time base for the product Guardian. ADR-001's decision not to add a sequence number remains in force.

**Alternatives Considered:**
- Wait for and introduce the heartbeat contract from ADR-001 → Rejected: it blocks the specified Guardian and adds a new interface that the current architecture does not provide.
- Reuse the demo's threshold/watchdog logic → Rejected: the supplied specification requires a full physical-consistency rewrite and the demo is non-authoritative under ADR-003.
- Use producer timestamps for temporal checks → Rejected: receive timing must remain monotonic and independent of replay-controlled wall clocks.

**Consequences:**
- ✅ Listener, periodic scheduling, physical model, and reporting are cleanly separated.
- ✅ Missing input is detectable without changing CAN, VSS, or uProtocol contracts.
- ✅ The model is deterministic and unit-testable without Zenoh/uProtocol.
- ❌ Receive-side staleness identifies the observed symptom only; it cannot distinguish transport delay/drop from source dropout.

### ADR-005: Separate Guardian model, diagnostics, and injection ground truth (2026-10-07)

**Context:**
- The former `battery_guardian.yaml`, diagnostic catalog, and `battery_fault_contract.yaml` mixed or duplicated model parameters, diagnostic IDs, injected causes, and expected observations.
- The existing Guardian-to-DFM reporter is working and must remain the sole reporting path.
- A combination injection may produce zero, one, or multiple Guardian detections, so injected causes cannot be modeled as aliases for detection classes.

**Decision:**
- Canonical configuration lives under `product/config/battery_guardian/`: `guardian_model.yaml` parameterizes admissibility, `guardian_diagnostics.json` defines DFM representation, and `fault_injection_model.yaml` defines injection-side ground truth.
- Guardian model semantics remain the combination of source code and `guardian_model.yaml`; `DetectionClass × DetectionLevel` is the observation vocabulary.
- The existing DFM reporter consumes `guardian_diagnostics.json`. No second reporter or mapping mechanism is introduced.
- Signal injections use canonical Guardian signal names and a uniform `mutations[]` representation. Single-signal classes require one mutation; `signal.combination` requires at least two distinct signals and forbids nested combinations.
- Transport/source faults use actions rather than signal mutations. The Guardian never infers an injected class; the Fault Generator's execution record is authoritative for injection ground truth.

**Alternatives Considered:**
- Keep one combined contract containing classes, mappings, and model parameters → Rejected: duplicated semantics drifted from the actual reporter catalog and blurred cause versus observation.
- Map every injected class to one expected Guardian detection → Rejected: root causes are receiver-side ambiguous and combination faults can produce multiple or no detections.
- Replace the current DFM integration while renaming the catalog → Rejected: the reporter already implements the required detection-to-diagnostic path.

**Consequences:**
- ✅ Each artifact answers one question: admissibility, diagnostic representation, or deliberately injected cause.
- ✅ Injection-model validation rejects malformed single-signal, combination, transport, and source definitions before a campaign runs.
- ✅ The DFM reporting implementation and its catalog schema remain unchanged apart from the path rename.
- ❌ The current repository still needs a concrete Fault Generator/ASC mutator to execute the new injection model; this refactoring defines and validates its contract only.

### ADR-006: Detection class and level are orthogonal (2026-10-07)

**Context:**
- Model rules and the degree to which they are approached or exceeded are different concepts; encoding both in class names creates a growing warning/critical class matrix.
- A physically consistent battery sample can be hot enough to require a thermal warning or critical diagnostic, while continuous model bounds benefit from pre-violation observations.
- Guardian observations, DFM diagnostics, and injected causes must remain distinct.

**Decision:**
- Every Guardian observation is `DetectionClass × DetectionLevel`, where class identifies the reacting model rule and level is `Warning`, `Violation`, or `Critical`.
- Replace separate thermal classes with `THERMAL_LIMIT`: warning is active from the derived threshold `absolute_max_c - warning_margin_c` up to the maximum; critical is active at or above `absolute_max_c` and supersedes warning.
- At the configured 70 °C boundary, `THERMAL_LIMIT / CRITICAL` is active without an absolute-limit violation; above it, `PHYSICAL_TEMP_ABSOLUTE_LIMIT / VIOLATION` is independently valid.
- Spread, hotspot, and temperature-rate checks use `observed / limit`: no detection below the global utilization threshold, `Warning` from that threshold through 1.0, and `Violation` above 1.0. Other checks remain binary `Violation`s.
- The existing DFM reporter is a configured class/level projection. Thermal warning/critical and existing physical violations retain their fault IDs; spread/hotspot/rate warnings remain internal evidence without new DFM catalog entries.
- The injection vocabulary remains separate and unchanged. In particular, `signal.spike` is not a Guardian class and may be observed as a temperature-rate warning or violation.

**Alternatives Considered:**
- Create separate warning and critical classes per model rule → Rejected: severity is orthogonal to the rule and would duplicate the detection vocabulary.
- Derive warnings with separate formulas → Rejected: one utilization fraction gives consistent semantics for continuous upper bounds.
- Map every warning to a new DFM fault → Rejected: detections are internal evidence first; diagnostics are an explicit projection and the catalog has no suitable entries for the new continuous-bound warnings.
- Add thermal or spike injection classes based on Guardian output → Rejected: they would confuse observed behavior with injected root cause.

**Consequences:**
- ✅ One stable class vocabulary supports warning, violation, and critical observations without class proliferation.
- ✅ Continuous-model warnings carry residual and utilization for later Evidence Collector integration without changing the DFM catalog.
- ✅ Valid hot samples and absolute-limit violations remain independently observable, and fault-injection ground truth remains separate.
- ❌ The demonstrator has no hysteresis; values oscillating around thermal or utilization thresholds can produce repeated state transitions.
