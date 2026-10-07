# Live Scenario Observer

> **Status:** decided specification. Authority for the decisions recorded here
> is ADR-016; where this document conflicts with an ADR, the ADR governs.
> This document does not redefine model, mutator, or contract semantics —
> those remain authoritative in their own artifacts (ADR-005).

## 1. Purpose and scope

The Live Scenario Observer makes one running battery campaign experiment
visible in a browser while it executes. It shows the live battery signals, the
Guardian's detected fault classes, the injected incidents, and the expected
observations on a shared relative-time axis.

- **Read-only.** The observer never injects, mutates, or steers the system, and
  never becomes a source of truth. It visualizes what the Evidence Collector
  currently sees.
- **Product component**, implemented as a **feature-flagged module plus a CLI
  option inside the Evidence Collector binary** — not a separate process and not
  a separate crate (ADR-016). The Evidence Collector remains the main binary.
- **v1 single-host, one experiment per collector process** (consistent with
  ADR-014 §7.1).
- **Not DoD-critical.** The challenge `README.md` Definition of Done does not
  require a UI; the observer is a v1 extension and must not dilute the evidence
  pipeline.

Out of scope for v1: multi-run comparison/history, run control from the UI,
transport-fault visualization, the raw Guardian decision stream, and any
computation that would duplicate Guardian or collector semantics.

## 2. Governing constraints

- **ADR-005** — separate model, diagnostics, injection ground truth; the
  observer must not infer injected causes.
- **ADR-006** — Guardian observations are orthogonal
  `DetectionClass × DetectionLevel`.
- **ADR-007** — three independent evidence planes (battery input, Guardian
  decision, DFM/OpenSOVD); the raw `GuardianEvidenceEvent` stream is still
  `[PLANNED]`.
- **ADR-012** — collector verdict on the Guardian fault-event stream.
- **ADR-013** — one relative, zero-based millisecond time base across the
  pipeline; no wall clock in the graph.
- **ADR-014** and `product/doc/testing/battery_campaign_test_harness.md` — the
  experiment bundle, five incidents, 20 s duration, and no provenance hashes.
- **ADR-015** — Guardian detects lost source generations as
  `STREAM_GENERATION_GAP / VIOLATION` (DFM `BatteryTempGenerationGap`); the
  observer shows it like any mapped detection and consumes its `interval_ms`.
- **ADR-003** — the `demo/` tree is a reference, not a requirement.

## 3. Architecture

```text
Evidence Collector process (main binary)
  ├─ collector core ....... subscriptions, correlation, verdict   [existing]
  └─ observer module ...... live state, ring buffer, SSE, assets  [feature "observer"]
        │  in-process read of collector state
        ▼
   HTTP server (axum) on 127.0.0.1:8090
        │  GET /            embedded static frontend
        │  GET /events      Server-Sent Events (snapshot + deltas)
        ▼
   Browser (static assets, EventSource)
```

- There is **no collector→UI transport link**: the Web UI is fed directly from
  in-process collector state. The only external interface is browser-facing.
- The observer is compiled behind the Cargo feature `observer` and enabled by
  the CLI option `--observer`. With the feature off, the binary behavior is
  unchanged.

## 4. Data sources (present data only)

| Source | Content used | Status |
|---|---|---|
| `BatteryTempEvent` `//battery-vss/9001/1/9001` | `timestamp_ms`, `temp_min`, `temp_avg`, `temp_max`, `soc` | present |
| `GuardianFaultEvent` `//guardian/1001/1/8001` | `detection_class`, `level`, `stage`, `baseline`, `fault_id`, `evidence{signal,observed,limit,residual,utilization,interval_ms,timestamp_ms}` (incl. `STREAM_GENERATION_GAP`) | present |
| Ground truth `<prefix>.ground_truth.yaml` | `injection_id`, `injected_class`, `source_started_at_ms`, `source_finished_at_ms`/`duration_ms`, `mutations{signal,operator,requested_parameters,executed_values}` | present (ADR-014) |
| Oracle `<prefix>.oracle.yaml` (runner-supplied) | `generation_goal{primary,allowed,forbidden}`, `evaluation_window_ms`, `predicted_observations[]` | per ADR-014; binding change to ADR-012 |
| `battery-guardian` library (`GuardianConfig`, model) | static envelope/threshold derivation | present |

Explicitly **not** used in v1:

- the raw `GuardianEvidenceEvent` stream (`//guardian-vss/9000/1/9002`,
  `[PLANNED]`), see §11;
- collector health/status output — the UI does not display collector status.

## 5. Time base

- Axis and all windows are **relative milliseconds** on the ADR-013 base.
- `t0` = `timestamp_ms` of the first `BatteryTempEvent` (by convention `0`).
- Incident windows from ground truth are relative ms and are **not** rebased.
- Reset happens at process start; a follow-run is a new process (§10).
- No wall clock is shown except, at most, an informational local start time.
- **Dependency:** until `vss_bridge` forwards the CAN `TimeStamp` unchanged
  (ADR-015 consequence / ADR-013 open point), the bridge's wall-clock jitter can
  raise **false** `STREAM_GENERATION_GAP` detections and make the axis jump.

## 6. Live-view data model

The observer keeps a **20-second ring buffer** of samples plus the current
run's static inputs and detected classes. No coalescing: every received event is
forwarded as-is.

### 6.1 Event types (SSE `event:` names)

- `snapshot` — full current state, sent first on every (re)connect.
- `sample` — one battery sample.
- `detection` — one Guardian fault event placed on the timeline.

### 6.2 Schemas

`sample`:
```json
{ "type": "sample", "timestamp_ms": 1200,
  "temp_min": 25.0, "temp_avg": 30.0, "temp_max": 32.0, "soc": 80.0 }
```

`detection` (fault events carry no top-level time; `at_ms` = `evidence.timestamp_ms`
when present, otherwise the latest battery timestamp):
```json
{ "type": "detection", "at_ms": 1200,
  "detection_class": "PHYSICAL_TEMP_RATE", "level": "VIOLATION",
  "stage": "Failed", "baseline": false, "fault_id": "BatteryTempRateViolation",
  "evidence": { "signal": "temp_max", "observed": 12.0, "limit": 8.0,
                "residual": 4.0, "utilization": 1.5,
                "interval_ms": 100, "timestamp_ms": 1200 } }
```

`snapshot` (static inputs included once):
```json
{ "type": "snapshot", "run_id": "...", "t0_ms": 0, "window_ms": 20000,
  "bands": { "temp_abs_min_c": -30.0, "temp_warning_c": 60.0,
             "temp_abs_max_c": 70.0, "soc_min": 0.0, "soc_max": 100.0 },
  "ground_truth": { "incidents": [
    { "injection_id": "...", "injected_class": "signal.spike",
      "start_ms": 2000, "end_ms": 2100,
      "mutations": [ { "signal": "temp_max", "operator": "spike",
                       "requested_parameters": {}, "executed_values": [] } ] } ] },
  "oracle": { "incidents": [
    { "injection_id": "...",
      "evaluation_window_ms": { "start": 2000, "end": 3000 },
      "primary": [], "allowed": [],
      "forbidden": [ { "class": "PHYSICAL_TEMP_RATE", "level": "WARNING" } ],
      "predicted_observations": [ { "class": "...", "level": "...",
        "state": "active", "predicted_at_ms": 2000,
        "source_timestamp_ms": 2000 } ] } ] },
  "samples": [ /* sample[] */ ], "detections": [ /* detection[] */ ] }
```

### 6.3 Static bands

Bands are derived from `guardian_model.yaml` through the `battery-guardian`
library — never re-derived by hand or in the frontend (ADR-005):

- temperature envelope `[absolute_min_c, absolute_max_c]`;
- warning band `[absolute_max_c − warning_margin_c, absolute_max_c)`;
- critical at `>= absolute_max_c`;
- SoC range `[min_percent, max_percent]`.

## 7. HTTP / SSE interface

- Bind address `127.0.0.1:8090` (port 8080 is the Guardian).
- `GET /` — embedded static frontend (single-file or `rust-embed` assets).
- `GET /events` — `text/event-stream`; the first event is always `snapshot`,
  followed by `sample`/`detection` deltas. No coalescing.
- `GET /health` — optional liveness only; not rendered in the UI.
- Reconnect: the browser `EventSource` reconnects automatically; because SSE
  has no retain, every reconnect receives a fresh `snapshot`.

## 8. Frontend (scenario graph)

- 20-second sliding window on the relative-time axis.
- Temperature lane: `temp_min`, `temp_avg`, `temp_max` as lines.
- SoC lane: `soc` as a line.
- Static bands drawn as background regions.
- Dynamic limits from `detection.evidence` rendered **step-after** (they only
  exist at transitions, see §11).
- Detection lane: `detection_class` transitions, colored by `level`
  (`WARNING` / `VIOLATION` / `CRITICAL`), labeled with `stage`.
- Ground-truth lane: one marker per incident window, labeled `injected_class`.
- Oracle lane: required/allowed/forbidden expectations per incident; a
  provisional in-window match indicator may be shown, clearly marked as
  live/provisional.

## 9. CLI and feature flag

- Cargo feature: `observer` (off by default).
- CLI: `--observer` enables the module; `--observer-addr 127.0.0.1:8090`
  overrides the bind address.
- Example: `cargo run --features observer -- <prefix> --observer`.
- With the feature disabled the option is absent and the existing CLI and
  exit-code contract are unchanged.

## 10. Lifecycle

1. The runner (`product/components/end2end-runner/run_golden.sh`) starts the
   collector for exactly one experiment (ADR-014 §7.1), passing the bundle
   prefix and `--idle-timeout` (drain). The observer reads
   `<prefix>.ground_truth.yaml` and, once bound, the same-prefix
   `<prefix>.oracle.yaml` (runner-supplied).
2. The observer serves the live view during the replay.
3. After replay end and drain, the last view is **frozen** and kept served until
   the process terminates; the collector's drain/finalize/verdict phase is that
   grace period. The observer is not required to survive the run.
4. A follow-run is a new collector process that **overwrites** the previous
   state. The runner terminates the predecessor first, so no port conflict or
   bind-retry is required.

## 11. Known limitations (v1)

- **Unmapped warnings are invisible.** Only DFM-mapped `class/level` pairs are
  published on `//guardian/1001/1/8001`, so `PHYSICAL_TEMP_SPREAD` /
  `PHYSICAL_TEMP_HOTSPOT` / `PHYSICAL_TEMP_RATE` `WARNING` observations cannot
  be shown. Logged in `docs/project_notes/bugs.md`; resolved by the raw
  `GuardianEvidenceEvent` stream (ADR-007).
- **Dynamic limits are discrete.** `GuardianFaultEvent` is a fault-level
  projection; `evidence` is present only at transitions, so limits appear as
  step-after segments, not as a continuous curve. Accepted for v1.
- **Bridge time not yet preserved.** Until `vss_bridge` forwards the CAN
  `TimeStamp` (ADR-015/ADR-013), wall-clock jitter can show false
  `STREAM_GENERATION_GAP` detections and the live axis may jump.
- No history, multi-run comparison, or control from the UI.

## 12. Interfaces to change / follow-ups

- **ADR-012 amendment:** the collector reads the same-prefix `<prefix>.oracle.yaml`
  (runner-supplied) instead of expecting the class-to-class table in the
  collector (`expected_observations.yaml`). Until then the observer displays
  whatever expectations the collector currently holds.
- **ADR-007:** implement the raw decision stream; then the observer subscribes
  to it (via the collector) and §11 limits disappear.
- **Bookkeeping done:** observer noted in `key_facts.md` (port 8090) and
  `components_and_channels.md` (C10 + E19).

## 13. Testing

- Unit: ring-buffer eviction (20 s), detection placement at latest timestamp,
  band derivation via the library, snapshot schema serialization.
- Integration: start the collector with `--observer`, connect to `/events`,
  assert a `snapshot` is first and `sample`/`detection` match injected input.
- Regression: build and run with the feature **off** — no server, unchanged
  behavior and exit codes.

## 14. Open / deferred

- Binding of `oracle.yaml` (ADR-012 amendment) and freezing of the harness
  schemas.
- Raw decision stream (ADR-007) to surface unmapped warnings.
- Continuous (non-step) dynamic limits.
- Multi-run comparison and history.
- Authentication/binding beyond localhost, if the observer ever leaves the
  single-host v1 scope.
