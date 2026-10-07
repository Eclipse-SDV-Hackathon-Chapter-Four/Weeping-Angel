# Architectural Decision Records

Chronological log of binding architectural decisions for this repo.

**Authority rule (ADR-002, ADR-003):** ADRs in this file are authoritative.
`PLAN.md` is an idea input and the `demo/` tree a reference implementation —
neither has authority over ADRs; where they conflict, the ADR governs.

## Format

Each decision records: date, context, decision, rejected alternatives, and
consequences (✅/❌). Number sequentially (ADR-001, ADR-002, ...).

### ADR-001: No sequence numbers — Guardian heartbeat counter as time base (2026-10-06) — Superseded by ADR-004

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
- No sequence number is added to `BatteryTempEvent`; all physical-model parameters come from the Guardian YAML configuration. Every battery message does carry the source-relative generation timestamp defined by ADR-008. The Guardian does **not** substitute it for local monotonic receive time in staleness or temperature-rate checks, but may use discontinuities in the source timeline to detect missing message generations (`transport.drop`).
- This receive-time decision supersedes ADR-001's not-yet-implemented heartbeat time base for the product Guardian. ADR-001's decision not to add a sequence number remains in force.

**Alternatives Considered:**
- Wait for and introduce the heartbeat contract from ADR-001 → Rejected: it blocks the specified Guardian and adds a new interface that the current architecture does not provide.
- Reuse the demo's threshold/watchdog logic → Rejected: the supplied specification requires a full physical-consistency rewrite and the demo is non-authoritative under ADR-003.
- Replace local receive time with producer timestamps for staleness or temperature-rate checks → Rejected: receive timing must remain monotonic and independent of replay-controlled progression. This does not prohibit source-timestamp gap analysis for drop detection.

**Consequences:**
- ✅ Listener, periodic scheduling, physical model, and reporting are cleanly separated.
- ✅ Missing input remains detectable from local receive age; once a later message arrives, an unexpected source-timestamp gap may additionally reveal one or more missing generations.
- ✅ The packed 8-byte CAN/DBC payload remains unchanged; its generation timestamp is transported as message metadata and in `BatteryTempEvent`.
- ✅ The model is deterministic and unit-testable without Zenoh/uProtocol.
- ❌ Receive-side staleness alone identifies the observed symptom only. A source-timestamp gap identifies missing generations but cannot by itself prove whether they were lost at the source or in transport.

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
- The existing DFM reporter is a configured class/level projection. Thermal warning/critical and existing physical violations retain their fault IDs; spread/hotspot/rate warnings have no DFM catalog entries but remain Guardian decisions and are published unchanged to the Evidence Collector under ADR-007.
- The injection vocabulary remains separate and unchanged. In particular, `signal.spike` is not a Guardian class and may be observed as a temperature-rate warning or violation.

**Alternatives Considered:**
- Create separate warning and critical classes per model rule → Rejected: severity is orthogonal to the rule and would duplicate the detection vocabulary.
- Derive warnings with separate formulas → Rejected: one utilization fraction gives consistent semantics for continuous upper bounds.
- Map every warning to a new DFM fault → Rejected: detections are internal evidence first; diagnostics are an explicit projection and the catalog has no suitable entries for the new continuous-bound warnings.
- Add thermal or spike injection classes based on Guardian output → Rejected: they would confuse observed behavior with injected root cause.

**Consequences:**
- ✅ One stable class vocabulary supports warning, violation, and critical observations without class proliferation.
- ✅ Continuous-model warnings carry residual and utilization to the Evidence Collector without changing the DFM catalog.
- ✅ Valid hot samples and absolute-limit violations remain independently observable, and fault-injection ground truth remains separate.
- ❌ The demonstrator has no hysteresis; values oscillating around thermal or utilization thresholds can produce repeated state transitions.

### ADR-007: Guardian publishes its original decisions independently of DFM mapping (2026-10-07)

**Context:**
- ADR-005 made the DFM reporter the diagnostic reporting path, but a DFM projection is not the Guardian's complete original view: it can aggregate signals and omit detections for which no diagnostic mapping exists.
- The Evidence Collector must be able to distinguish what entered the Guardian, what the Guardian itself decided, and what subsequently appeared in DFM/OpenSOVD.
- In particular, utilization warnings without DFM mappings must remain observable as Guardian decisions.

**Decision:**
- Amends ADR-005 (on top of ADR-006): every internal Guardian `Detection` transition is published in the Guardian's original representation as `GuardianEvidenceEvent` over uProtocol. The Evidence Collector receives this event before and independently of any class/level → DFM mapping.
- The Guardian stream contains mapped and unmapped detections alike, including utilization warnings. Its payload carries the Guardian decision (`DetectionClass × DetectionLevel`), active/cleared state, signal and the available observed/limit/residual/utilization evidence. It does not require or infer a DFM `fault_id`.
- Independently, the same internal detection may enter the configured DFM projection and fault-level aggregation. DFM mapping, omission or aggregation must not alter or suppress the GuardianEvidenceEvent.
- The Evidence Collector subscribes directly to `//battery-vss/9001/1/9001` (`BatteryTempEvent`) and to the Guardian decision stream. It obtains DFM/OpenSOVD messages through the diagnostic path as a third, independent view.
- The Guardian decision publisher and DFM reporter are independent: neither waits for the other, and failure of one sink does not suppress the other.
- The Guardian stream publishes actual detection transitions only. A synthetic DFM startup baseline is diagnostic state and is not presented as an original Guardian decision.

**Alternatives Considered:**
- Mirror only mapped DFM results over uProtocol → Rejected: the Evidence Collector would lose the Guardian's original signal-level decisions and every unmapped warning.
- Publish from inside the DFM worker → Rejected: this would apply DFM mapping/aggregation first and couple Guardian evidence to DFM availability.
- Periodically re-publish decisions for late subscribers → Rejected for now: the Guardian stream records transitions; diagnostic current state remains available through DFM/SOVD.

**Consequences:**
- ✅ The Evidence Collector has the Guardian's complete original view, including detections that never become DFM faults.
- ✅ It can correlate three independent evidence planes: incoming battery events, Guardian decisions, and DFM/OpenSOVD diagnostics.
- ✅ DFM/OpenSOVD remains the diagnostic truth while GuardianEvidenceEvent remains the truth of what the Guardian decided.
- ❌ Subscribers that join after a transition miss it (no retain on Zenoh publish).
- ❌ The existing mapped `GuardianFaultEvent` implementation must be replaced or separated from the raw GuardianEvidenceEvent contract.

### ADR-008: Source-relative generation timestamps accompany battery messages (amends ADR-004; payload placement amended by ADR-011) (2026-10-07)

**Context:**
- ADR-004 keeps the Guardian's model time base on receiver-side receive timestamps and dropped ADR-001's heartbeat.
- Deterministic replay and evidence correlation need a source time that is shared by the CAN-side message, the uProtocol battery event, the Guardian and the Evidence Collector.
- The packed CAN frame is already full, so the timestamp cannot be another DBC signal in the 8-byte payload.

**Decision:**
- No sequence numbers are introduced (retained from ADR-001/ADR-004).
- Every generated battery CAN message is associated with `timestamp_ms`, its generation time in integer milliseconds on the source/replay timeline. The first generated message starts at `0`; subsequent values express elapsed source time from that origin.
- `timestamp_ms` is CAN-message metadata, not an additional DBC payload signal. The CAN/VSS/uProtocol bridge preserves the value unchanged in `BatteryTempEvent`; it must not replace it with bridge wall-clock or receive time.
- The Guardian and Evidence Collector independently subscribe to the same `BatteryTempEvent` topic and therefore receive the same source timestamp and battery values.
- The Guardian continues to use local monotonic receive time for staleness and temperature-rate evaluation. The source timestamp is available for message identity, duplicate/reorder analysis, drop detection and cross-stream evidence correlation. In particular, the Guardian may compare consecutive source timestamps against the expected generation cadence and treat unexpected forward gaps as missing message generations; this does not make source time the receive-time model base.
- This amends ADR-004's former optional producer-timestamp clause and does not reintroduce ADR-001's heartbeat.

**Alternatives Considered:**
- Generate the timestamp in the VSS/uProtocol bridge → Rejected: that records bridge processing time, not CAN-message generation time, and prevents end-to-end correlation.
- Use Unix epoch milliseconds → Rejected for the campaign stream: a zero-based source timeline is deterministic and replayable without wall-clock synchronization.
- Add a sequence number → Rejected: already rejected in ADR-001 (frame full; contract churn).
- Use the source timestamp as the Guardian's receive-time model base → Rejected: it cannot measure receive-side silence or transport delay and remains controlled by replay progression. Selective use for generation-gap/drop detection remains permitted.

**Consequences:**
- ✅ CAN-side input, Guardian input and Evidence Collector input share one deterministic millisecond timeline beginning at zero.
- ✅ `transport.drop`, `transport.duplicate` and `transport.reorder` can use source-timestamp gaps or identity while the Guardian's receive-time model remains independent.
- ✅ Replays preserve correlation without depending on host wall clocks.
- ❌ CAN acquisition, VSS/uProtocol publishing and consumers must all preserve `timestamp_ms` exactly; replacing or rebasing it breaks evidence correlation.
- ❌ Timestamp-based drop/duplicate/reorder detection remains deferred until implemented, and the injection vocabulary (ADR-005) does not yet define `transport.duplicate`.

### ADR-009: DFM remains in the chain (2026-10-06)

**Context:**
- PLAN.md proposed “DFM leave out (direct reporting to OpenSOVD)”.
- README Definition of Done requires DFM records for faulted scenarios and OpenSOVD exposing matching diagnostics; the target architecture is `Guardian → DFM → OpenSOVD`.

**Decision:**
- DFM remains a component of the diagnostic chain: `Guardian → DFM → OpenSOVD → Evidence Collector`.
- In parallel, the Evidence Collector subscribes directly to `BatteryTempEvent` and to the Guardian's raw `GuardianEvidenceEvent` decision stream. These direct evidence inputs complement rather than replace DFM/OpenSOVD verification.
- The class/fault-ID registry is defined by the canonical artifacts (ADR-005); DFM consumes the Guardian's `DetectionClass × DetectionLevel` projection from `guardian_diagnostics.json`.
- The DFM IPC transport is not fixed by this ADR.

**Alternatives Considered:**
- Direct Guardian → OpenSOVD (PLAN) → Rejected: drops required DFM records and the Guardian-vs-diagnostic failure distinction.

**Consequences:**
- ✅ Satisfies README DoD 4/5 and lets the Evidence Collector compare source input, the Guardian's original decisions and DFM/OpenSOVD visibility.
- ❌ Reintroduces the DFM/IPC component and its startup/catalog-hash dependency.
- ❌ PLAN.md “DFM leave out” is superseded and must be reconciled.

### ADR-010: v1 scope — full single-host chain; openDuT/Ankaios documented only (2026-10-06)

**Context:**
- README lists openDuT (Phase 4) and Ankaios (Phase 5) as later maturity levels.
- Current focus is the deterministic single-host evidence path.

**Decision:**
- v1 delivers the single-host flow in parallel branches: the VSS Publisher sends `BatteryTempEvent` to both Guardian and Evidence Collector; the Guardian publishes every internal detection transition unchanged as `GuardianEvidenceEvent` to the Evidence Collector and separately projects configured detections into DFM; DFM continues through OpenSOVD to the Evidence Collector.
- openDuT (campaign supervisor) and Ankaios are specified but not implemented in v1.
- The transport-fault injection mechanism remains open.

**Alternatives Considered:**
- Attempt openDuT/Ankaios in v1 → Rejected: dilutes the evidence path.
- Freeze the transport-fault mechanism now → Rejected: options still under review.

**Consequences:**
- ✅ Focused v1; later phases have a documented place in the interface spec.
- ❌ DoD 7/8 not met in v1 (documented only).

### ADR-011: Product CAN frame carries the source timestamp explicitly (amends ADR-008) (2026-10-07)

**Context:**
- ADR-008 defined a zero-based source-generation timestamp but retained the former 8-byte demo payload and therefore described the timestamp as out-of-band metadata.
- The canonical product assets now use a timestamped BatteryTemperature frame: four timestamp bytes followed by the four existing 16-bit battery signals.
- The Case Mutator must preserve source identity while independently changing replay/arrival timing for transport-delay cases.

**Decision:**
- The product `BatteryTemperature` frame `0x100` is 16 bytes: unsigned 32-bit `TimeStamp` in milliseconds at the front, followed by `CellTempAvg`, `CellTempMax`, `CellTempMin`, `StateOfCharge`, and four reserved bytes.
- `TimeStamp` starts at `0`, uses little-endian byte order consistently with the other frame signals, and remains unchanged by value mutation, frame deletion, or transport-delay scheduling. The reserved bytes remain uninterpreted and unchanged.
- The unchanged value is propagated into `BatteryTempEvent.timestamp_ms`. Local monotonic receive time remains the Guardian time base for temperature rate and stale evaluation; ADR-008's permitted timestamp-gap drop detection remains unchanged.
- This amends only ADR-008's out-of-band-metadata clause. The decision not to add sequence numbers remains in force, and the 8-byte `demo/` frame remains a non-authoritative historical reference.

**Alternatives Considered:**
- Keep the product frame at 8 bytes and carry timestamp only outside CAN → Rejected: it does not match the canonical timestamped ASC asset and prevents the ASC mutator from preserving generation time independently of replay timing.
- Replace timestamp with a sequence number → Rejected: elapsed source time is required for evidence correlation and the no-sequence decision remains valid.

**Consequences:**
- ✅ Value mutations and drops preserve the original generation identity directly in the replay artifact.
- ✅ Transport delay can change ASC replay time without rewriting the embedded generation timestamp.
- ❌ CAN provider/VSS mapping and bridge still need implementation work to propagate `TimeStamp` unchanged instead of generating wall-clock time.
