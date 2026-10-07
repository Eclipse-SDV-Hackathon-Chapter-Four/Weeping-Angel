# Battery Fault Case Mutator: ASC Injection Model

> **Status: DRAFT / scaffold — deliberately incomplete.** Section structure
> mirrors `product/doc/battery/battery_guardian_model.md`. Undecided items are
> marked `<!-- TODO(Dn): ... -->`; cross-artifact gaps are collected in
> "Open points and assumptions" below.

## Authority

- This document is **authoritative** for the case mutator's **mutation
  mechanics and parameters** (analogous to how `battery_guardian_model.md` is
  authoritative for the Guardian).
- Fault and detection **naming/registry authority follows ADR-005**: injected
  causes live in `product/config/battery_guardian/fault_injection_model.yaml`,
  the detection-to-DFM projection in
  `product/config/battery_guardian/guardian_diagnostics.json`, admissibility in
  `product/config/battery_guardian/guardian_model.yaml`; the interface file
  `product/interfaces/battery_fault_contract.yaml` (schema_version 3) defines
  event shapes only. This document references those identifiers and does not
  redefine them (see B).
- Guardian observations are the orthogonal pair `DetectionClass ×
  DetectionLevel` (ADR-006). This document treats the pair as **observed**
  output; the mutator only emits **injection ground truth** (see E).
- The `demo/` tree is a reference implementation, not authoritative (ADR-003).
  The prototype under `product/components/case_mutator/` is transient; this
  specification is the durable definition.

## Open points and assumptions (to be closed in a later iteration)

This specification is intentionally incomplete. The items below are recorded so
a later iteration can close them.

### A. Thermal warning/critical — resolved by ADR-006

The former assumption (absolute band + warning + critical) is now implemented:

- `THERMAL_LIMIT` uses `T_max`; the critical threshold is `T_crit =
  absolute_max_c`, the warning threshold is `T_warn = absolute_max_c -
  warning_margin_c`.
- At the configured values (`absolute_max_c = 70`, `warning_margin_c = 10`):
  - Normal: `T_max < 60`,
  - `THERMAL_LIMIT / WARNING`: `60 <= T_max < 70`,
  - `THERMAL_LIMIT / CRITICAL`: `T_max >= 70`.
- At exactly 70 °C the critical observation holds **without**
  `PHYSICAL_TEMP_ABSOLUTE_LIMIT`; above 70 °C both hold independently.
- Spread, hotspot, and rate use `utilization = observed / limit`
  (`warning.utilization_threshold = 0.8`): no detection below 0.8, `WARNING`
  from 0.8 through 1.0, `VIOLATION` above 1.0.

Overheating (`THERMAL_LIMIT`) and implausibility
(`PHYSICAL_TEMP_ABSOLUTE_LIMIT`) are therefore distinguishable.

### B. Naming and registry authority — resolved by ADR-005 (re-checked)

- `fault_injection_model.yaml` — injected causes (ground truth),
- `guardian_diagnostics.json` — detection-to-DFM projection,
- `guardian_model.yaml` — admissibility parameters,
- `battery_fault_contract.yaml` (v3) — event shapes only.

### C. Downstream scope

Guardian detection/classification faults and diagnostic-chain (DFM/OpenSOVD)
faults stay out of scope (§1.3).

### D. Stale timeout — inconsistent values

- `product/config/battery_guardian/guardian_model.yaml` sets
  `missing_packet_timeout_ms: 500`.
- `battery_guardian_model.md` §8 proposes `τ_stale = 2.0 s`.

The temporal recipe (§8) is parameterized by `τ_stale`; the two values must be
reconciled by a later iteration.

### E. Alignment with the canonical injection model — open

The authoritative `fault_injection_model.yaml` is narrower than the detection
vocabulary. Its signal operators are `stuck`, `spike`, `drift`, `out_of_range`,
plus `signal.combination` (>= 2 distinct signals); transport/source faults use
`action` operators `delay`, `drop`, `suspend_source`. Consequences:

- `PHYSICAL_TEMP_ORDERING`, `PHYSICAL_SOC_RANGE`, `PHYSICAL_SOC_RATE` have **no
  injection class**;
- `THERMAL_LIMIT` (warning/critical) has **no injection class**;
- an isolated `PHYSICAL_TEMP_HOTSPOT` only arises via `signal.drift`.

Per ADR-006 the injection vocabulary is deliberately separate from the
detection vocabulary; whether to extend it or to document these as gaps must be
decided in a later iteration. The mutator must not invent classes from
Guardian output.

## 1. Scope

### 1.1 Role in the evidence chain

The Case Mutator produces the **input-side ground truth** of a fault campaign.
It reads a reference ASC replay, applies exactly one mutation (deterministic
given a recorded seed) for a single injection class, and emits a mutated `.asc`
replay plus a ground-truth record (see §10). The mutated replay then runs
through the production path:

```text
ASC replay -> KUKSA CAN provider -> Databroker -> VSS bridge
          -> Guardian -> DFM -> OpenSOVD
```

The Evidence Collector correlates the Guardian's `DetectionClass ×
DetectionLevel` observations with the injection ground truth to decide
pass/fail per campaign. The mutator never bypasses CAN decoding, VSS mapping or
the VSS bridge, and never infers a class from Guardian output.

### 1.2 In scope (v1)

- **Value mutation** of the battery frame (CAN `0x100`, DLC 8): `temp_min`,
  `temp_avg`, `temp_max`, `SoC`; including implausibility and thermal
  warning/critical levels.
- **Temporal mutation** of the ASC timeline: frame drop / timestamp retiming
  to create stream gaps (`STREAM_STALE`, source side).
- Reproducibility (seeded; the same seed, template and injection class give
  the same output), replayability, and the ground-truth record.

### 1.3 Out of scope (v1)

- Guardian detection/classification faults (PLAN.md Step 2).
- Diagnostic-chain faults (DFM / OpenSOVD) (PLAN.md Step 3).
- Network/protocol transport faults (`transport.delay`, `transport.drop`,
  `transport.duplicate`; e.g. via Toxiproxy) — deferred, may change later.
- Duplicate/reorder injection (deferred); mitigation/actuator behavior.

### 1.4 Replay and timing assumptions

- Sample period `dt = 100 ms`; DBC quantization `q_T = 0.5 degC`,
  `q_SoC = 0.5 pp` (see `battery_guardian_model.md` §1).
- The replay honors ASC timestamps (inter-frame delays).
- The Guardian derives rate and freshness from **receive time**, so temporal
  mutations are observable only if the replay honors the timestamps.
- Only battery frames (CAN `0x100`, DLC 8) are mutated; all other lines stay
  byte-exact.

## 2. Configuration parameters

### 2.1 Input and output

| Parameter | Proposed value | Meaning |
|---|---|---|
| `template` | reference ASC path | nominal/reference replay (battery frames CAN `0x100`, DLC 8) |
| `output_dir` | `cases/<run_id>` | one mutated `.asc` + ground-truth record per case |
| `repetitions` | 1 | cases generated per run |

### 2.2 Timing and quantization (fixed by the CAN asset / DBC)

| Parameter | Symbol | Value | Unit |
|---|---:|---:|---|
| Sample period | $\Delta t$ | 100 | ms |
| Temperature quantum | $q_T$ | 0.5 | °C |
| SoC quantum | $q_{SoC}$ | 0.5 | pp |

### 2.3 Mutation parameters

| Parameter | Proposed value | Meaning |
|---|---|---|
| `lead_in_frames` | 20 | battery frames replayed byte-identical before the mutation |
| `seed` | 0 | RNG seed; the same seed, template and injection class give the same output (D5d) |
| `ramp_step` | $q_T$ | default rate-safe step |
| `rate_jump` | $\ge 2\,q_T$ | default rate-operator jump |

Individual recipes may override these; overrides are part of the case's
parameters and are recorded in the ground-truth record.

### 2.4 Target limits (mirror of the Guardian model)

Derived from the authoritative Guardian configuration
(`product/config/battery_guardian/guardian_model.yaml`); the mutator does not
redefine them.

| Parameter | Symbol | Value | Source |
|---|---:|---:|---|
| Absolute min / max | $T_{\mathrm{abs,min}}/T_{\mathrm{abs,max}}$ | −30 / 70 | model §2.2 |
| Warning margin | $M_{\mathrm{warning}}$ | 10 | model §2.2 |
| Warning / critical threshold | $T_{\mathrm{warn}}/T_{\mathrm{crit}}$ | 60 / 70 | model §2.2 |
| Utilization threshold | $u_{\mathrm{warning}}$ | 0.8 | model §2.2 |
| Reference / hot | $T_{\mathrm{ref}}/T_{\mathrm{hot}}$ | 20 / 70 | model §2.3 |
| Spread cold / hot | $S_{\mathrm{cold}}/S_{\mathrm{hot}}$ | 12 / 7 | model §2.4 |
| Hotspot cold / hot | $H_{\mathrm{cold}}/H_{\mathrm{hot}}$ | 5 / 2 | model §2.4 |
| Heating cold / hot | $R_{\uparrow,\mathrm{cold}}/R_{\uparrow,\mathrm{hot}}$ | 8 / 5 | model §2.5 |
| Cooling | $R_\downarrow$ | 6 | model §2.5 |
| SoC min / max / step | $SoC_{\min}/SoC_{\max}/\Delta SoC_{\max}$ | 0 / 100 / 0.5 | model §2.2/§6 |
| Stuck window / flatness | $N_{\mathrm{stuck}}/\epsilon_{\mathrm{stuck}}$ | 10 / 0.25 | model §7 |
| Excitation | $E_T/E_{SoC}$ | 1.0 / 1.0 | model §7 |
| Stale timeout | $\tau_\mathrm{stale}$ | 0.5 (config) / 2.0 (model) | Open point D |

## 3. Generic mutation formulation

The Guardian evaluates one residual per invariant,

$$
r_k = \mathrm{IST}_k - \mathrm{SOLL}_k,
\qquad r_k > 0 \;\Longrightarrow\; \text{fault condition}
$$

(see `battery_guardian_model.md` §3), and turns it into an observation
`DetectionClass × DetectionLevel` (ADR-006). A mutation is the inverse
operation: drive the sampled vector

$$
x_k = \left(T_{\min,k},\, T_{\mathrm{avg},k},\, T_{\max,k},\, SoC_k\right)
$$

so that the target residual crosses zero (violation) or the warning fraction
(warning). The recipes below are prose and reference the operators defined
here; their relation to observations is informative only.

### 3.1 Work region and lead-in

Mutations apply to the **work region** only: the battery frames after the
configured lead-in. Lead-in frames are replayed byte-identical, so each signal
starts from a reference base

$$
x_0 = \left(T_{\min}^{0},\, T_{\mathrm{avg}}^{0},\, T_{\max}^{0},\, SoC^{0}\right)
$$

read from the last lead-in frame. The 100 ms frame grid is preserved; if a
pattern is longer than the template it continues on the same grid.

### 3.2 Quantization

All mutated values are exact multiples of the DBC quantization
$q_T = 0.5\,^\circ\mathrm C$ and $q_{SoC} = 0.5\ \mathrm{pp}$ (Guardian model
§1). Rounding to the nearest quantum is the only allowed value transform.

### 3.3 Atomic step and rate safety

The smallest non-zero temperature change is one quantum, $q_T = 0.5\,^\circ\mathrm C$.
On the 100 ms grid this is $5\,^\circ\mathrm C/\mathrm s$, which stays below
the Guardian limits $R_\uparrow = 8 - 3\theta \in [5,8]\,^\circ\mathrm C/\mathrm s$
and $R_\downarrow = 6\,^\circ\mathrm C/\mathrm s$ (Guardian model §5):

- $|\Delta| = q_T$ per frame $\Rightarrow$ rate-safe,
- $|\Delta| \ge 2\,q_T$ per frame $\Rightarrow$ rate violation ($\ge 10\,^\circ\mathrm C/\mathrm s$).

A one-quantum drift drives the utilization ratio $u = \mathrm{observed}/\mathrm{limit}$
gradually, so the same recipe naturally passes through `WARNING` before
`VIOLATION`; a jump skips the warning band.

### 3.4 Stuck excitation

A flat signal is not a stuck fault by itself. The Guardian requires a
10-sample window that is flat within $\epsilon_{\mathrm{stuck}} = 0.25\,^\circ\mathrm C$
while another signal is excited by at least $1.0\,^\circ\mathrm C$ / $1.0\ \mathrm{pp}$
(Guardian model §7). Hence, whenever a recipe holds a signal (nearly)
constant it must either keep **all** signals constant (no excitation, no stuck)
or give the peers a wiggle of amplitude $> \epsilon_{\mathrm{stuck}}$.

### 3.5 Operator catalog

Two layers are used. The **canonical operators** are those of
`fault_injection_model.yaml`; they are what an injection instance specifies.
The **primitives** are the implementation vocabulary used to realize them in
the ASC file.

Canonical operators:

| Operator | Applies to | Parameters |
|---|---|---|
| `stuck` | one signal | `duration_samples` |
| `spike` | one signal | `delta`, `duration_samples` |
| `drift` | one signal | `rate_per_sample`, `duration_samples` |
| `out_of_range` | one signal | `value`, `duration_samples` |
| (`signal.combination`) | >= 2 distinct signals | one operator each |
| `delay` / `drop` / `suspend_source` | action | `delay_ms`/`duration_ms` |

Implementation primitives (work frame $k$, per signal column):

| Primitive | Definition | Realizes |
|---|---|---|
| `base` | repeat $x_0$ byte-identical | the frozen reference |
| `hold(v, n)` | $v$ for $n$ frames | `out_of_range` plateau |
| `ramp(v_0, v_t, s)` | $v_k = v_0 \pm k\,s$, $\lvert s\rvert = q_T$ | `drift` |
| `spike(v_p, n)` | $v_p$ for $n$ frames, else `base` | `spike` |
| `step(\Delta)` | $+\Delta$ for one frame | abrupt `out_of_range` |
| `wiggle(a)` | alternating $\{0, +a\}$, $a > \epsilon_{\mathrm{stuck}}$ | peer excitation for `stuck` |
| `shift_all(\Delta)` | $+\Delta$ on $T_{\min}, T_{\mathrm{avg}}, T_{\max}$ | coherent change (thermal limit) |
| `diverge(s)` | $T_{\max} {+}{=} s$, $T_{\min} {-}{=} s$ | spread growth |
| `drop(n)` / `retime(\Delta t)` | remove frames / shift timestamps | `drop`, `suspend_source`, `delay` |

An injection instance is executed faithfully: a mutation names **one**
signal (except `signal.combination`). Where a recipe below is marked
"isolated" it is an option that avoids co-trips, not a requirement of the
canonical operator.

## 4. Value / magnitude injection recipes

Observation references in this section are informative only; the registry is
Open point B and the token mapping is Open point E.

### 4.1 Implausible value (absolute limit)

Guardian invariant (model §4.1): $T_{\mathrm{abs,min}} \le T_{\min} \le T_{\max} \le T_{\mathrm{abs,max}}$;
only $T_{\min}$ and $T_{\max}$ are magnitude-checked. This is the
`signal.out_of_range` operator.

**Recipe:** drive the named signal to the absolute `value` for
`duration_samples` (e.g. `temp_min = -35`), e.g. via `step` to the value and
`hold`. Coherently shifting all three (`shift_all`) keeps spread/hotspot
constant if an isolated implausibility is wanted.

The prototype's `signal.out_of_range` sets `temp_min = -35.0`,
`duration_samples = 10`.

### 4.2 Thermal limit (warning / critical)

Guardian observation (model §2.2, ADR-006): `THERMAL_LIMIT` with `T_warn = 60`
and `T_crit = 70` on `T_max`. This is the coherent-change case; it stays inside
the absolute band up to and including 70 °C.

**Warning recipe:** `shift_all(+q_T)` from $x_0$ until `T_max >= T_warn` and
`T_max < T_crit`; hold. **Critical recipe:** continue until `T_max >= T_crit`;
at exactly 70 °C only `THERMAL_LIMIT / CRITICAL` holds, above 70 °C also
`PHYSICAL_TEMP_ABSOLUTE_LIMIT / VIOLATION`. A joint shift keeps spread/hotspot
constant and is rate-safe.

*Gap (Open point E):* the canonical injection model currently has **no**
injection class for `THERMAL_LIMIT`; a coherent increase is only mentioned as
an example in `product/config/battery_guardian/README.md`.

### 4.3 Temperature ordering

Guardian invariant (model §4.2): $T_{\min} \le T_{\mathrm{avg}} \le T_{\max}$;
binary `VIOLATION`. Gap: no injection class (Open point E).

**Recipe ($T_{\min} > T_{\mathrm{avg}}$):** `ramp(T_{\min}, T_{\mathrm{avg}} + q_T, +q_T)`
while `wiggle` on $T_{\mathrm{avg}}$ and $T_{\max}$. **Recipe ($T_{\mathrm{avg}} > T_{\max}$):**
`ramp(T_{\mathrm{avg}}, T_{\max} + q_T, +q_T)` with `wiggle` on $T_{\min}$ and
$T_{\max}$. A single quantum above the peer already violates the strict
ordering; the ramp is rate-safe and the peers are not stuck.

### 4.4 Temperature spread

Guardian invariant (model §4.3): $T_{\max} - T_{\min} \le S(\theta)$, with
utilization $u = (T_{\max}-T_{\min})/S(\theta)$: `WARNING` for $u \ge 0.8$,
`VIOLATION` for $u > 1$. This is the `signal.drift` mechanism.

**Recipe (`signal.drift`):** `ramp($T_{\max}$, $T_{\max}^{0} + k\,q_T$, $+q_T$)`
for `rate_per_sample * duration_samples` (the canonical instance uses
`rate_per_sample = 0.5`, `duration_samples = 30`). Because `T_avg` and `T_min`
stay put, the hotspot `(T_max - T_avg)` also grows — the same drift can
co-raise `PHYSICAL_TEMP_HOTSPOT`. The drift is below the rate limit, so it
passes through the spread/hotspot `WARNING` band before `VIOLATION`.

**Isolated option:** grow the spread by lowering only $T_{\min}$ (`ramp` down
by $q_T$ per frame) while `wiggle` on the others; the hotspot stays constant
and the 5 °C/s decrease is below $R_\downarrow = 6$ °C/s.

### 4.5 Hotspot

Guardian invariant (model §4.4): $T_{\max} - T_{\mathrm{avg}} \le H(\theta)$,
with the same utilization rules. The canonical injection model produces this
only via `signal.drift` (Open point E).

**Isolated option:** shift $T_{\max}$ and $T_{\min}$ jointly by $+q_T$ while
`wiggle` on $T_{\mathrm{avg}}$; the hotspot grows by $q_T$ per frame, the
spread stays constant.

## 5. Dynamic injection recipes

The rate check compares per-signal changes against $R_\uparrow(T_{\mathrm{avg},k-1}, Q_k)$
and $R_\downarrow$ (model §5), with `utilization = |rate|/limit`: `WARNING`
from 0.8, `VIOLATION` above 1.0. The previous average drives $\theta$, keeping
the check causal, and `signal.spike` is the canonical operator (there is no
`SPIKE` detection class).

**Spike (`signal.spike`):** `spike($\Delta$, n)` on the named signal with
$\Delta \ge 2\,q_T$ and a few quanta of margin against receive-time jitter
(the canonical instance is `temp_max`, `delta = 10.0`, `duration_samples = 1`).
The injected frame and the return frame each cross the rate limit. Because only
one signal moves, spatial co-trips depend on the chosen signal and delta.

**Sustained ramp:** `ramp(x_0, x_0 + $\Delta$, +$2q_T$)` for a run of frames —
every frame violates $R_\uparrow$; applies to `drift`-like injections.

**Cooling:** `spike(-$\Delta$, n)` with $\Delta \ge 2\,q_T$
($10\,^\circ\mathrm C/\mathrm s > 6\,^\circ\mathrm C/\mathrm s$).

## 6. State-of-charge injection recipes

The SoC checks are a binary range check and a per-sample step check with
$\Delta SoC_{\max} = q_{SoC} = 0.5\ \mathrm{pp}$ (model §6). The DBC field is
unsigned, so $SoC \ge 0$ cannot be violated; only the upper bound is reachable.

*Gap (Open point E):* `fault_injection_model.yaml` has **no** SoC operator, so
these recipes are documented for completeness only.

**Range:** `ramp(SoC^0, 100 + q_{SoC}, +q_{SoC})` — a one-quantum step equals
$\Delta SoC_{\max}$ and is strictly safe (the check is $> \Delta SoC_{\max}$);
hold above 100. Wiggle the temperatures, because a rising SoC excites the stuck
detector while flat temperatures would be classified as stuck. A single
`step(\ge 2\,q_{SoC})` additionally raises the step check.

**Rate:** `step(+2\,q_{SoC}, 1)` for one frame, then return, or a sustained
`ramp` with $2\,q_{SoC}$ per frame, keeping $0 \le SoC \le 100$.

## 7. Signal-stuck injection recipe

Stuck requires a signal whose $N_{\mathrm{stuck}} = 10$-sample window is flat within
$\epsilon_{\mathrm{stuck}} = 0.25\,^\circ\mathrm C$ **and** an independently excited
peer (model §7). This is the `stuck` operator.

**Recipe:** `base` on the named signal for at least $N_{\mathrm{stuck}}$ frames
(byte-identical raw values; the canonical instance is `temp_avg`,
`duration_samples = 20`), while a peer carries a sawtooth of amplitude at least
$E_T = 1.0\,^\circ\mathrm C$ with steps $\le q_T$ (rate-safe). The peer is
demonstrably changing, so the frozen channel is classified as stuck. A variant
freezes SoC and excites a temperature channel by at least $E_{SoC} = 1.0$ pp.

*Note:* if **all** signals are flat there is no excitation and no stuck
detection; a frozen run shorter than the window does not trigger either. The
peer sawtooth also perturbs spread/hotspot by up to its amplitude.

## 8. Temporal injection (stream gap)

`STREAM_STALE` is a symptom of the stream stopping, not a value mutation. It is
produced by the action operators `drop`, `suspend_source`, or `delay` so that
the receive age exceeds the configured missing-packet timeout $\tau_\mathrm{stale}$
(Open point D).

**Drop / suspend:** `drop(n)` removes $n$ consecutive battery frames, leaving a
gap of $(n+1)\,\Delta t$ between the surrounding frames. The minimum is
$n \ge \lceil \tau_\mathrm{stale}/\Delta t \rceil$; a margin of a few frames is
recommended. With $\Delta t = 0.1\,\mathrm s$:

- $\tau_\mathrm{stale} = 0.5\,\mathrm s$ (current `guardian_model.yaml`)
  $\rightarrow n \ge 5$, recommend $n \ge 10$;
- $\tau_\mathrm{stale} = 2.0\,\mathrm s$ (model-doc proposal, Open point D)
  $\rightarrow n \ge 20$.

**Delay:** `retime(\Delta t)` shifts the timestamps after a point by more than
$\tau_\mathrm{stale}$ without deleting frames (the canonical instance uses
`delay_ms = 750`). **Truncation:** dropping the tail leaves the stream stale
indefinitely; a mid-stream gap produces stale $\rightarrow$ cleared.

*Attribution note:* the Guardian cannot distinguish the injected root cause; a
gap may correspond to `source.dropout` or a deferred `transport.*` fault. The
ground-truth record (§10) carries the injected class.

*Dependency:* the replay must honor ASC timestamps; a fixed-rate replay
produces no receive-time gap.

## 9. Complete mutation pipeline

For each case:

1. Parse the reference ASC and collect the battery frames (CAN `0x100`, DLC 8).
2. Keep the first `lead_in_frames` frames byte-identical; read $x_0$ from the
   last lead-in frame.
3. Select the injection instance (`injected_class`, canonical operators and
   parameters) from `fault_injection_model.yaml`; draw any randomization from
   the recorded seed.
4. Realize each canonical operator with the primitives of §3.5 over the work
   region; extend the frame stream on the 100 ms grid if the pattern is longer
   than the template.
5. Render the mutated ASC (all non-battery lines byte-exact).
6. Emit the ground-truth record (§10) with the executed `mutations[]` and
   timing.

<!-- TODO(D8): formalize the operator-to-frame mapping as pseudocode. -->

## 10. Ground-truth sidecar schema

The record is the authoritative injection-side artifact
(`fault_injection_model.yaml` + `battery_fault_contract.yaml` v3). It is
**not** derived from Guardian output.

```yaml
run_id: <campaign run>
injection_id: <instance id>
injected_class: signal.spike
started_at: <epoch wall clock, ISO-8601>
duration_ms: 100            # or finished_at
mutations:                  # signal faults only
  - signal: temp_max
    operator: spike
    parameters: { delta: 10.0, duration_samples: 1 }
```

- Required: `injection_id`, `injected_class`, `started_at`; timing is either
  `finished_at` or `duration_ms` (contract `require_one_of`).
- Signal faults additionally require `mutations[]` with `signal`, `operator`,
  `parameters`; a `signal.combination` carries the complete list.
- `run_id` binds the record to the campaign run (contract
  `campaign_ground_truth_event`).
- Time base: `started_at` is **epoch wall clock**, so it correlates with the
  Guardian's `detected_at_ms`; the replay-relative `injected_at_ms` of the
  prototype is superseded.

<!-- TODO(D7): confirm the epoch/replay-relative decision and the collector's
     correlation window. -->

## 11. Mapping to the v1 fault campaign

Illustrative injection -> observation relations (not a diagnostic mapping;
the Guardian must not infer the injected class). This mirrors
`product/config/battery_guardian/README.md`.

| Injected class | Typical observation(s) |
|---|---|
| `signal.spike` | `PHYSICAL_TEMP_RATE / WARNING`, later `/ VIOLATION` |
| `signal.drift` | `PHYSICAL_TEMP_HOTSPOT / WARNING`, `PHYSICAL_TEMP_SPREAD / WARNING`, later the corresponding `VIOLATION`s |
| `signal.out_of_range` | `PHYSICAL_TEMP_ABSOLUTE_LIMIT / VIOLATION` |
| `signal.stuck` | `SIGNAL_STUCK / VIOLATION` |
| `signal.combination` | zero, one, or multiple observations |
| coherent temperature increase (no injection class yet) | `THERMAL_LIMIT / WARNING`, later `/ CRITICAL` |
| `transport.delay` / `transport.drop` / `source.dropout` | `STREAM_STALE / VIOLATION` |

## 12. Compact mutator parameter block

```yaml
case_mutator:
  template: <reference>.asc
  output_dir: cases/<run_id>
  repetitions: 1
  lead_in_frames: 20
  seed: 0                     # reproducible runs: same seed -> same output

  sample_period_ms: 100
  temperature_quantum_c: 0.5
  soc_quantum_pp: 0.5

  # Target limits mirror the Guardian (config/battery_guardian/guardian_model.yaml).
  temperature:
    absolute_min_c: -30.0
    absolute_max_c: 70.0
    warning_margin_c: 10.0    # T_warn = 60, T_crit = 70
    reference_c: 20.0
    hot_state_c: 70.0
    spread:  { cold_c: 12.0, hot_c: 7.0 }
    hotspot: { cold_c: 5.0,  hot_c: 2.0 }
    heating_rate_c_per_s: { cold: 8.0, hot: 5.0 }
    cooling_rate_c_per_s: 6.0

  warning:
    utilization_threshold: 0.8

  soc:
    min_percent: 0.0
    max_percent: 100.0
    max_step_pp: 0.5

  stream:
    stale_timeout_ms: 500     # Open point D: model doc proposes 2000

  stuck:
    window_samples: 10
    flatness_epsilon_c: 0.25
    temperature_excitation_c: 1.0
    soc_excitation_pp: 1.0
```
