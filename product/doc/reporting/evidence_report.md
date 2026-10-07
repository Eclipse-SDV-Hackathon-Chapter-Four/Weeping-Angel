# Evidence Report — specification (draft)

> **Status:** decided (2026-10-07). Authority: **ADR-018**. Where this document
> conflicts with an ADR, the ADR governs (ADR-002). This document does not
> redefine verdict, model, mutator, or observer semantics — those stay
> authoritative in their own artifacts. The resolved decisions are recorded in
> §13.

## 1. Purpose and scope

The **Evidence Reporter** turns the machine-readable results of a battery
campaign into one human-readable, evidence-linked report per run and one
aggregate report per campaign.

- It **renders** the canonical JSON produced by the Evidence Collector and the
  runner. It never re-evaluates Guardian decisions or recomputes verdicts.
- It answers, per experiment, the README question: *what fault was injected,
  what the system observed, which mitigation was triggered, and why the verdict
  is PASS, FAIL, or INCONCLUSIVE* — for the parts the current chain records.
- It is **presentation only**: no evaluation fact may exist only in Markdown
  (test-harness spec §8.6). Every statement must be traceable to an input.
- It is **reproducible**: given the same inputs it produces the same report
  (modulo the generated-at stamp).

## 2. Non-goals

- No live execution, replay, system reset, or component orchestration.
- No independent verdict logic (ADR-012/ADR-007); no second Oracle.
- No screenshots of a live UI as a *source* of evaluation facts.
- No history/multi-run comparison beyond a campaign-to-campaign consistency
  table (§7.5); no run control.
- Not part of the Definition of Done by itself; it is the reporting leg of the
  evidence chain.

## 3. Governing decisions

- **ADR-007** — three independent evidence planes; raw Guardian decisions vs.
  DFM projection; report is `verdict.json` → Markdown.
- **ADR-012** — verdict is computed on the Guardian fault-event stream; window
  is `source_started_at_ms`..`source_finished_at_ms`; tri-state
  `PASS`/`FAIL`/`INCONCLUSIVE`.
- **ADR-014** — per-experiment bundles, five incidents, `evidence/<run-id>/`
  retention (Collector JSON, normative verdict JSON, derived Markdown, logs,
  per-run DFM storage); no hashes/provenance artifacts.
- **ADR-016** — the Live Scenario Observer is read-only and not DoD-critical;
  its data must not become report truth. ADR-018 amends it with a read-only
  export surface (`/snapshot.json`, `--dump-html`).
- **ADR-017** — fault events carry the source timestamp of their causing
  sample; unplaced events are reported as such in the notes.
- **ADR-018** — standalone Evidence Reporter component, Python with minimal
  dependencies, renders the current `report.json` schema, outputs not committed,
  GitHub-safe presentation only.
- **Test-harness spec §8.6** — `verdict.json` is canonical, `report.md` is
  derived; Markdown must not contain facts absent from the JSON.

## 4. Position in the architecture

```text
tools/run_case.sh  ──►  <run>/report.json + logs
                          │
tools/run_campaign.sh ──► └─► Evidence Reporter ──► <run>/report.md
                          │                         campaign/evidence_report.md
                          └──► observer snapshot ──► observer.html / observer.png
```

The **Evidence Reporter** is a standalone component in
`product/components/evidence_reporter/` with its own README and tests, invoked
by the campaign runner `tools/run_campaign.sh` (ADR-018). It is a plain Python
CLI with minimal dependencies (standard library plus `PyYAML`; no templating
engine). It has no execution role (ADR-014 §7 leaves execution to the runner).

## 5. Inputs

### 5.1 Per run (canonical)

The reporter renders against the **current Evidence Collector `report.json`
schema** (ADR-018). A future frozen `verdict.json` may supersede it; until then
this schema is the contract. Run/campaign/scenario identities are **derived
from the directory paths** (`reports/campaign-<timestamp>/<campaign>--<scenario>/`)
and the sibling `experiment.yaml`; they are not expected as top-level fields in
`report.json`.

- **Collector result JSON** (`report.json`). Fields actually consumed:
  - identity/result: `case`, `verdict`, `replay_end_ms`, `battery_end_ms`,
    `notes`.
  - `oracle`: `expected`, `not_applicable`, `matched`, `missing`,
    `not_reached`, `unexpected`.
  - `injections[]`: `run_id`, `injection_id`, `injected_class`,
    `source_started_at_ms`, `source_finished_at_ms`, `duration_ms`,
    `window_ms`, `slot_ms`, `mutations{signal, operator,
    requested_parameters, executed_values}`, `verdict`,
    `expected`/`matched`/`missing`/`unexpected`/`sovd_missing`.
  - `transitions[]`: `status`, `fault_id`, `detection_class`, `level`, `stage`,
    `expected_at_ms`, `observed_at_ms`, `signal`, `sovd_visible_at_ms`,
    `sovd_missing`, `injection_id`.
  - `fault_events[]`: `t_ms`, `fault_id`, `detection_class`, `level`, `stage`,
    `baseline`, `evidence{observed, limit, residual, utilization, interval_ms,
    timestamp_ms, signal}`.
  - `sovd`: `url`, `polls`, `errors`, `activations[{t_ms, code}]`,
    `not_visible[]`, `unexplained[]`.
- **Experiment bundle**: `experiment.yaml`, `case.asc`,
  `case.ground_truth.yaml`, `case.oracle.yaml`; for non-executed profiles
  `unsatisfiable.yaml`.
- **Logs**: `can.log`, `collector.log`, `collector.out`, `dfm.log`,
  `dfm_sovd_bridge.log`, `guardian.log`, `vss_bridge.log`.

### 5.2 Per campaign

- `summary.txt` (runner verdict list and totals).
- the run directories and their `experiment.yaml` / `unsatisfiable.yaml`.
- optional second campaign directory for the consistency table (§7.5).

### 5.3 Static configuration (join, not evaluation)

- `product/config/battery_guardian/guardian_diagnostics.json` — fault catalog
  `id | name | category | severity | summary`.
- (No narrative mapping: the detective-story section is deliberately out of
  scope for v1, see §13 Q8.)

## 6. Outputs and artifacts

The reporter follows the **current** folder structure under `reports/`
(ADR-018); generated reports are **not committed**.

- `<run-dir>/report.md` — per-run report, next to `report.json`.
- `<campaign-dir>/evidence_report.md` — campaign aggregate.
- `<run-dir>/observer.html` — self-contained final-state observer document
  (from the observer `snapshot`), and `<run-dir>/observer.png` — a rendered
  figure (see §10).
- **Link policy:** the Markdown links to reports/artifacts; it embeds HTML only
  as GitHub-safe fragments and images. No `<script>`/`<iframe>`/`<style>`.
  Interactive HTML stays a separate, linked artifact.

## 7. Report structure

### 7.1 Per-run report (`report.md`)

1. **Header** — run id, campaign id, scenario id, generated-at, overall verdict.
2. **Provenance & reproduction** — bundle paths, oracle/ground-truth links,
   log links, the exact generation and run command, run-id.
3. **Diagnostic chain** — static pipeline diagram (`source → … → DFM →
   OpenSOVD → collector`), links only.
4. **Fault catalog** — catalog rows for the fault ids that occurred.
5. **Injection summary** — per incident: injected class, signal/operator,
   requested vs. executed values, window/slot, per-injection verdict.
6. **Expected vs. observed** — oracle transitions vs. collector transitions
   (`matched`/`missing`/`not_reached`/`unexpected`), with `signal`, `level`,
   `stage`, `expected_at_ms`, `observed_at_ms`.
7. **Detection evidence** — fault events with `observed`, `limit`, `residual`,
   `utilization`, `interval_ms` — the "why" behind each detection.
8. **DFM / OpenSOVD correlation** — activations with `t_ms`, `not_visible`,
   `unexplained`, poll/error counts; visibility latency
   (`sovd_visible_at_ms − observed_at_ms`).
9. **Timing** — detection latency (`observed_at_ms − source_started_at_ms`),
   SOVD visibility latency, replay end vs. battery end.
10. **Verdict rationale** — explicit derivation from the oracle counts and
    injection verdicts; never re-inferred.
11. **Observer artifacts** — inline `observer.png`, link to `observer.html`.
12. **Gaps & limitations** — see §8 (mitigation, SOVD status triple, unmapped
    warnings).
13. **Notes** — the collector's `notes` verbatim (e.g. unplaced events,
    trailing stale).

### 7.2 Campaign report (`evidence_report.md`)

1. **Header** — campaign id, generated-at, totals
   (`PASS` / `FAIL` / `INCONCLUSIVE` / `SKIPPED`).
2. **Run table** — run id, injected class(es), verdict, oracle counts, link to
   `report.md`.
3. **Fault-class coverage matrix** — rows = README classes
   (transport/signal/source/diagnostics), columns = executed/skipped/not
   attempted, with reasons from `unsatisfiable.yaml`.
4. **Non-PASS section** — every FAIL and SKIPPED with cause; FAILs are never
   hidden (README "What Not to Do").
5. **Consistency** (optional, when ≥2 campaign dirs are given) — verdicts per
   run side by side.
6. **Aggregate gaps** — §8 across all runs.
7. **Reproduction** — campaign command and environment notes.

## 8. Data gaps — render, do not invent

These chain links are not present in current inputs and **must appear as
explicit placeholders**, not be fabricated or silently omitted:

- **Mitigation** — v1 is event-only (ADR-007); no mitigation event is recorded.
  Render `Mitigation: not instrumented (event-only, ADR-007)`.
- **SOVD fault status triple** — only activation times are captured; there is no
  `testFailed`/`confirmedDtc`/`warningIndicator`. Render activation-based
  visibility and state the limitation.
- **Unmapped Guardian warnings** — not on the DFM-mapped stream (ADR-016 §11);
  note that the report and the observer see only mapped `class/level` pairs.
- **Unplaced events** — fault events without a causing sample timestamp
  (ADR-017) appear in the notes; do not guess a time.

## 9. Verdict semantics

- The report echoes the tri-state verdict; it does not compute it.
- `expected = matched + missing + not_reached`; `unexpected` and
  `sovd_missing` are shown separately.
- `INCONCLUSIVE` (empty expectation set, missing evidence, infrastructure
  failure) and infrastructure errors are reported distinctly from a model
  `FAIL` (test-harness spec, failure-mode matrix).

## 10. Observer artifacts

Option C is adopted (ADR-018): the observer gains a small export surface and the
runner triggers the capture. The reporter only consumes the artifacts.

- **Observer export (to implement, ADR-016 amendment):**
  - `GET /snapshot.json` — returns the current `snapshot` from the in-process
    `handle.snapshot()` (route addition in `observer/mod.rs`).
  - `--dump-html FILE` (and/or `GET /export.html`) — writes the self-contained
    document below.
- `observer.html`: the embedded frontend (`index.html`/`app.js`/`style.css`)
  with the captured snapshot injected instead of the `EventSource`; self-
  contained, opens offline.
- `observer.png`: a headless-browser screenshot of the frozen page (Chromium
  is available in the environment); **illustrative only**, never a fact source.
- Capture point: while the collector's frozen view is still served (ADR-016
  §10); the runner triggers the capture. Capturing the `snapshot` event makes
  the result independent of the process lifetime.

## 11. Determinism and retention

- Inputs are immutable per run; the reporter never overwrites an existing
  run/report directory (ADR-014).
- `generated_at` is the only intentionally non-deterministic field; it lives in
  the header and is excluded from any consistency comparison (ADR-018).
- Retention follows ADR-014: reports are derived artifacts next to the
  canonical JSON, logs, and per-run DFM storage.

## 12. Interfaces and component layout

CLI (ADR-018):

```sh
# per run (usually called by run_case.sh)
evidence_reporter run  <run-dir>  [--diagnostics FILE] [--observer-snapshot FILE]

# per campaign (called by run_campaign.sh at the end)
evidence_reporter campaign <campaign-dir> [--compare <other-campaign-dir>]
```

Layout:

```text
product/components/evidence_reporter/
  README.md          # usage
  tests/             # golden-input → golden-report tests
  source/            # renderer (Python, stdlib + PyYAML)
```

## 13. Resolved decisions (2026-10-07, ADR-018)

- **Q1 — Form and ownership.** Standalone component
  `product/components/evidence_reporter/`, invoked by `tools/run_campaign.sh`
  (and per-run by `tools/run_case.sh`).
- **Q2 — Input schema.** Render against the **current** collector `report.json`
  schema. Run/campaign/scenario identities come from the directory paths and
  `experiment.yaml`, not from top-level JSON fields.
- **Q3 — Granularity.** Both: per-run `report.md` and campaign
  `evidence_report.md`.
- **Q4 — Location/retention.** Adapt to the current `reports/campaign-<ts>/…`
  structure; generated reports are not committed.
- **Q5 — Technology.** Python with minimal dependencies (stdlib + `PyYAML`).
- **Q6 — Determinism.** `generated_at` appears in the header only and is
  excluded from comparisons.
- **Q7 — Observer capture.** Option C: observer `GET /snapshot.json` +
  `--dump-html`, capture triggered by the runner; PNG via headless Chromium.
- **Q8 — Detective narrative.** Out of scope for v1.
- **Q9 — Markdown target.** GitHub-safe subset (no scripts/styles/iframes).

## 14. Testing

- Golden tests: fixed input JSON + bundle → expected `report.md` (no
  `generated_at`) and campaign report.
- PASS/FAIL/INCONCLUSIVE/SKIPPED each rendered; FAILs visible.
- Missing optional data (no `sovd`, no `oracle`, unplaced events) does not
  crash and does not invent facts.
- Placeholder rendering for §8 gaps.
- Link/asset existence checks and GitHub-safe HTML subset.
