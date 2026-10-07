# Battery Fault Case Mutator: Inverse Guardian Model

> **Status:** normative design specification for the case mutator.
>
> The mutator does not merely apply arbitrary signal perturbations. It constructs
> quantized CAN mutations that are expected to produce a requested Guardian
> observation under the authoritative Battery Guardian model.

---

## 1. Authority and semantic separation

This document is authoritative for the **case mutator's generation algorithm,
mutation mechanics, inverse-model constraints, and ground-truth output**.

The following artifacts remain authoritative for their respective semantics:

```text
product/config/battery_guardian/guardian_model.yaml
    Guardian model parameters

product/doc/battery/battery_guardian_model.md
    Guardian model formulas and boundary semantics

product/config/battery_guardian/fault_injection_model.yaml
    injected fault classes and canonical mutation operators

product/config/battery_guardian/guardian_diagnostics.json
    Guardian DetectionClass × DetectionLevel -> DFM projection

product/interfaces/battery_fault_contract.yaml
    event schemas only
```

The mutator must keep three concepts strictly separate:

```text
Injected fault class
    what the campaign deliberately changes

Generation goal
    which Guardian observation(s) the generated case shall produce

Guardian observation
    DetectionClass × DetectionLevel produced by forward model evaluation
```

The mutator emits injection ground truth. It must never infer the injected class
from Guardian output.

---

## 2. Role in the campaign

The mutator reads a nominal ASC replay and produces:

1. one mutated `.asc` replay;
2. one injection-side ground-truth record;
3. one test oracle describing the requested Guardian observation and allowed /
   forbidden co-detections.

The replay follows the production path unchanged:

```text
ASC replay -> KUKSA CAN provider -> Databroker -> VSS bridge
           -> BatteryTempEvent ---------------------------> Evidence Collector
                     \-> Guardian
                           |-> GuardianEvidenceEvent ------> Evidence Collector
                           \-> DFM -> OpenSOVD ------------> Evidence Collector
```

The mutator operates on timestamped CAN frame `0x100`, DLC 16, containing a
little-endian 32-bit source-generation timestamp followed by:

```text
temp_min
temp_avg
temp_max
soc
```

The remaining four payload bytes are reserved. The mutator preserves them
unchanged and does not interpret them.

All non-target ASC content must remain byte-identical unless a temporal mutation
explicitly changes the timeline.

---

## 3. Required mutator inputs

Each generation request contains:

```yaml
template: <reference.asc>
battery_model: product/config/battery_guardian/guardian_model.yaml
started_at: <campaign start, ISO-8601>
injection_id: <stable id>
injected_class: <canonical injected fault class>
mutations: [...]
generation_goal: ...
seed: 0
```

The exact Guardian model file must be loaded at generation time and treated as
read-only input. The mutator must not duplicate or override Guardian thresholds.

Record the SHA-256 hash of the model file in every generated case.

---

## 4. Canonical injected fault classes

The v1 mutator supports:

```text
signal.stuck
signal.spike
signal.drift
signal.out_of_range
signal.combination

transport.delay
transport.drop
source.dropout
```

Deferred classes are not implemented:

```text
transport.duplicate
transport.reorder
source.replay_interruption
diagnostics.dfm_write_delay
diagnostics.opensovd_partial_visibility
```

### 4.1 Signal targets

Canonical signal names:

```text
temp_min
temp_avg
temp_max
soc
```

`signal.stuck` is valid only for:

```text
temp_min
temp_avg
temp_max
```

SoC may be used as independent excitation for temperature-stuck generation but
is not itself a stuck target.

### 4.2 Combination faults

`signal.combination` contains at least two distinct signal mutations:

```yaml
injected_class: signal.combination
mutations:
  - signal: temp_min
    operator: drift
    parameters: {...}
  - signal: temp_max
    operator: drift
    parameters: {...}
```

Nested combinations are forbidden.

Combination faults are first-class because coordinated changes may alter one
Guardian invariant while preserving others.

---

## 5. Generation goal

Every generated case must have an explicit **generation goal**.

Example:

```yaml
generation_goal:
  primary:
    - class: PHYSICAL_TEMP_SPREAD
      level: WARNING

  allowed:
    - class: PHYSICAL_TEMP_RATE
      level: WARNING

  forbidden:
    - class: PHYSICAL_TEMP_SPREAD
      level: VIOLATION
    - class: PHYSICAL_TEMP_ABSOLUTE_LIMIT
      level: VIOLATION
```

Semantics:

```text
primary
    observations that MUST be produced

allowed
    additional observations that MAY occur

forbidden
    observations that MUST NOT occur
```

Anything not listed as `primary` or `allowed` is forbidden by default unless the
case explicitly sets:

```yaml
allow_unspecified_codetections: true
```

Default:

```text
allow_unspecified_codetections = false
```

The generation goal is **test-oracle data**, not injection ground truth.

---

## 6. Guardian model inputs consumed by the mutator

The mutator must consume the same model keys as the Guardian.

Current model:

```text
guardian.evaluation_period_ms
guardian.missing_packet_timeout_ms

guardian.warning.utilization_threshold

guardian.temperature.absolute_min_c
guardian.temperature.absolute_max_c
guardian.temperature.warning_margin_c
guardian.temperature.reference_c
guardian.temperature.hot_state_c

guardian.temperature.spread.cold_c
guardian.temperature.spread.hot_c

guardian.temperature.hotspot.cold_c
guardian.temperature.hotspot.hot_c

guardian.temperature.dynamics.heating_rate_c_per_s.cold
guardian.temperature.dynamics.heating_rate_c_per_s.hot
guardian.temperature.dynamics.cooling_rate_c_per_s
guardian.temperature.dynamics.soc_coupling.enabled
guardian.temperature.dynamics.soc_coupling.gain_c_per_pp
guardian.temperature.dynamics.soc_coupling.rate_cap_pp_per_s

guardian.soc.min_percent
guardian.soc.max_percent
guardian.soc.max_rate_pp_per_s

guardian.stuck.enabled
guardian.stuck.window_samples
guardian.stuck.flatness_epsilon_c
guardian.stuck.temperature_excitation_c
guardian.stuck.soc_excitation_pp
```

The model configuration must pass the same validity constraints as the Guardian,
including:

$$
T_{\mathrm{abs,min}}
\le T_{\mathrm{ref}}
< T_{\mathrm{hot}}
\le T_{\mathrm{abs,max}}.
$$

A model rejected by the Guardian must also be rejected by the mutator.

Do not silently translate obsolete model keys such as `soc.max_step_pp`.

---

## 7. Quantization and timing

DBC properties:

```text
nominal sample period = 100 ms
temperature quantum   = 0.5 °C
SoC quantum           = 0.5 pp
```

For dynamic Guardian rules, the relevant interval is the **source/generation
interval**, i.e. the difference of the relative generation timestamps:

$$
\Delta\tau_k=ts_k-ts_{k-1}.
$$

This is the same interval the Guardian uses for its rate and dynamics checks
(ADR-013). The receive interval $\Delta t^{\mathrm{recv}}_k$ and the resulting
freshness/age are properties of arrival (transport/source timing), not of the
signal values; the ASC mutator does not fabricate them and must not substitute
them for $\Delta\tau_k$.

Generation may initially assume the ASC schedule for candidate construction, but
every candidate must be forward-verified using the Guardian's
source/generation-interval semantics.

All emitted signal values must be representable by the DBC.

### 7.1 Constraint-preserving quantization

Do not perform unconstrained "round to nearest".

Quantization must preserve the requested observation.

For a continuous upper-bound check with limit $L$ and warning threshold $u_w$:

```text
WARNING:
    choose a representable value y such that
    u_w * L <= y <= L

VIOLATION:
    choose a representable value y such that
    y > L
```

Prefer robust interior targets instead of exact thresholds:

```text
default warning target utilization   = 0.90
default violation target utilization = 1.10
```

These are mutator generation defaults, not Guardian model parameters.

If the requested band contains no representable value, the candidate is
unsatisfiable.

---

## 8. Core generation algorithm

The mutator must generate cases by **inverse Guardian evaluation**.

Normative algorithm:

```text
generate_case(template, injection, generation_goal, model):

    1. Load and validate the exact Guardian model.

    2. Parse the reference ASC and decode the nominal battery frames.

    3. Preserve the configured lead-in unchanged.

    4. Select the mutation start state x0 from the nominal replay.

    5. Forward-evaluate x0 with the Guardian model.

    6. Translate generation_goal into mathematical constraints.

    7. Add constraints imposed by:
         - injected_class;
         - mutation operator;
         - selected target signal(s);
         - DBC representable range;
         - DBC quantization;
         - requested primary observations;
         - allowed co-detections;
         - forbidden observations;
         - temporal ordering and duration.

    8. Solve for one or more candidate mutated values / trajectories.

    9. Quantize candidates with constraint-preserving quantization.

   10. Forward-evaluate the complete quantized trajectory using the exact
       Guardian model semantics.

   11. Accept a candidate iff:
         - every primary observation occurs;
         - no forbidden observation occurs;
         - all mutation/operator constraints are satisfied.

   12. Otherwise search the next candidate.

   13. If no representable candidate exists:
         return UNSATISFIABLE with a structured reason.

   14. Emit the mutated ASC, injection ground truth, model provenance,
       and test oracle.
```

Step 10 is mandatory even for analytically invertible rules.

---

## 9. Inverse Guardian constraints

### 9.1 Thermal limit

Guardian thresholds:

$$
T_{\mathrm{crit}}=T_{\mathrm{abs,max}},
$$

$$
T_{\mathrm{warn}}
=
T_{\mathrm{abs,max}}-M_{\mathrm{warning}}.
$$

Inverse constraints:

```text
THERMAL_LIMIT / WARNING:
    T_warn <= temp_max < T_crit

THERMAL_LIMIT / CRITICAL:
    temp_max >= T_crit
```

If the goal forbids `PHYSICAL_TEMP_ABSOLUTE_LIMIT`, additionally require:

$$
T_{\max}\le T_{\mathrm{abs,max}}.
$$

Therefore the isolated critical point is exactly:

$$
T_{\max}=T_{\mathrm{abs,max}}.
$$

A coherent thermal mutation should normally use `signal.combination` so that
`temp_min`, `temp_avg`, and `temp_max` may be shifted together while preserving
spread/hotspot.

There is no separate injected class called `thermal_warning` or
`thermal_critical`.

### 9.2 Absolute temperature limit

Inverse conditions:

```text
low violation:
    temp_min < T_abs,min

high violation:
    temp_max > T_abs,max
```

A requested strict violation must be quantized to a representable value strictly
outside the bound.

Typical injection class:

```text
signal.out_of_range
```

### 9.3 Temperature ordering

Guardian invariant:

$$
T_{\min}\le T_{\mathrm{avg}}\le T_{\max}.
$$

Inverse alternatives:

$$
T_{\min}>T_{\mathrm{avg}}
$$

or

$$
T_{\mathrm{avg}}>T_{\max}.
$$

Use at least one representable quantum beyond the peer value.

There is no dedicated ordering injection class. A case uses one of the existing
signal mutation classes and records ordering only as the requested observation.

### 9.4 Spread

Guardian limit:

$$
L_S=S(T_{\mathrm{avg}})
$$

with

$$
S(T)=
S_{\mathrm{cold}}
-
(S_{\mathrm{cold}}-S_{\mathrm{hot}})
\theta(T).
$$

Observed:

$$
S_{\mathrm{obs}}=T_{\max}-T_{\min}.
$$

Inverse target:

```text
SPREAD / WARNING:
    u_warning * L_S <= S_obs <= L_S

SPREAD / VIOLATION:
    S_obs > L_S
```

If `temp_avg` is held constant and `temp_min` is the target:

$$
T_{\min}^{*}=T_{\max}-S_{\mathrm{target}}.
$$

If `temp_max` is the target:

$$
T_{\max}^{*}=T_{\min}+S_{\mathrm{target}}.
$$

After quantization, recompute $\theta$, $L_S$, utilization, and all other
Guardian rules.

A single-signal drift may also change hotspot and rate. Such co-detections must
be included in `allowed` or the solver must choose another realization.

### 9.5 Hotspot

Guardian limit:

$$
L_H=H(T_{\mathrm{avg}})
$$

with

$$
H(T)=
H_{\mathrm{cold}}
-
(H_{\mathrm{cold}}-H_{\mathrm{hot}})
\theta(T).
$$

Observed:

$$
H_{\mathrm{obs}}=T_{\max}-T_{\mathrm{avg}}.
$$

Inverse target:

```text
HOTSPOT / WARNING:
    u_warning * L_H <= H_obs <= L_H

HOTSPOT / VIOLATION:
    H_obs > L_H
```

For fixed `temp_avg`:

$$
T_{\max}^{*}=T_{\mathrm{avg}}+H_{\mathrm{target}}.
$$

To increase hotspot while preserving spread, a combination mutation may shift
`temp_max` and `temp_min` together by the same amount.

Again, the complete candidate must be forward-verified because changing
`temp_max` or `temp_avg` may also affect thermal-limit, spread, and rate rules.

### 9.6 Temperature rate / spike

For each temperature signal:

$$
\dot T_i
=
\frac{T_{i,k}-T_{i,k-1}}{\Delta\tau_k}.
$$

Heating limit:

$$
R_{\uparrow,\mathrm{base}}
=
R_{\uparrow,\mathrm{cold}}
-
(R_{\uparrow,\mathrm{cold}}-R_{\uparrow,\mathrm{hot}})
\theta(T_{\mathrm{avg},k-1}).
$$

If SoC coupling is enabled:

$$
Q_k=
\min
\left(
\left|
\frac{SoC_k-SoC_{k-1}}{\Delta\tau_k}
\right|,
Q_{\mathrm{cap}}
\right),
$$

$$
R_\uparrow=
R_{\uparrow,\mathrm{base}}+K_{SoC}Q_k.
$$

Cooling limit:

$$
R_\downarrow.
$$

Inverse heating target:

```text
RATE / WARNING:
    u_warning * R_up <= dT/dt <= R_up

RATE / VIOLATION:
    dT/dt > R_up
```

Therefore:

$$
\Delta T_{\mathrm{target}}
=
R_{\mathrm{target}}\Delta\tau_k.
$$

For cooling use magnitudes:

```text
RATE / WARNING:
    u_warning * R_down <= -dT/dt <= R_down

RATE / VIOLATION:
    -dT/dt > R_down
```

A `signal.spike` is an injected cause, not a Guardian detection class.

The return edge of a one-frame spike must also be forward-evaluated because it
may generate a second rate observation.

### 9.7 SoC range

Inverse conditions:

```text
SOC_RANGE / VIOLATION:
    soc < soc.min_percent
    OR
    soc > soc.max_percent
```

Only DBC-representable directions are feasible.

If the physical/DBC encoding cannot represent a value below zero, a low-range
violation is `UNSATISFIABLE` for the ASC mutator and must not be faked.

### 9.8 SoC rate

Guardian model:

$$
\dot{SoC}_k=
\frac{SoC_k-SoC_{k-1}}{\Delta\tau_k}
$$

with

$$
|\dot{SoC}_k|
\le
R_{SoC,\max}.
$$

Current configuration:

$$
R_{SoC,\max}=5\ \mathrm{pp/s}.
$$

Inverse violation:

$$
|SoC_k-SoC_{k-1}|
>
R_{SoC,\max}\Delta\tau_k.
$$

This check is binary in v1; there is no SoC-rate warning level.

Do not use the obsolete fixed `0.5 pp/sample` rule.

### 9.9 Stuck

For temperature signal $T_i$ over $N$ samples:

$$
A_i=\max(T_i)-\min(T_i).
$$

The target must satisfy:

$$
A_i\le\epsilon_{\mathrm{stuck}}.
$$

Independent excitation must simultaneously satisfy:

$$
\max_{j\ne i}A_j\ge E_T
\quad\lor\quad
A_{SoC}\ge E_{SoC}.
$$

Therefore the mutator must construct **both**:

1. a flat target trajectory;
2. at least one independently excited peer trajectory.

The peer trajectory must itself be checked against spread, hotspot, thermal and
rate constraints.

A case with all signals flat must not be accepted as a stuck case.

### 9.10 Stream stale

Guardian condition:

$$
age(t)>\tau_{\mathrm{stale}}.
$$

The stale test is periodic with evaluation interval
$\tau_{\mathrm{eval}}$.

For a robust source-gap case, construct a receive gap with margin:

$$
gap
>
\tau_{\mathrm{stale}}
+
\tau_{\mathrm{eval}}.
$$

For nominal 100-ms source frames and the current model:

```text
stale timeout       = 500 ms
evaluation period   = 100 ms
```

Use at least a 700-ms receive gap; campaign default should remain more
conservative (for example 1 s).

`STREAM_STALE` is a symptom only. `transport.delay`, `transport.drop`, and
`source.dropout` remain distinct injected causes.

The stale check uses the receive axis ($\Delta t^{\mathrm{recv}}$, projected
relative now), never the source/generation interval $\Delta\tau$.

A pure transport delay must preserve source-generation timestamps while
delaying receipt. ASC timestamp retiming alone changes generation time and must
not be mislabeled as pure transport delay.

---

## 10. Satisfiability and co-detections

The mutator must explicitly model that not every requested observation pattern
is realizable for every baseline state.

Return:

```text
UNSATISFIABLE
```

when no DBC-representable trajectory satisfies all requested and forbidden
constraints.

Required structured reason examples:

```yaml
status: UNSATISFIABLE
reason:
  code: QUANTIZATION_CONFLICT
  detail: no representable spread value lies inside requested WARNING band
```

```yaml
status: UNSATISFIABLE
reason:
  code: FORBIDDEN_CODETECTION
  detail: every representable spread drift also raises PHYSICAL_TEMP_RATE/WARNING
```

```yaml
status: UNSATISFIABLE
reason:
  code: ENCODING_LIMIT
  detail: requested SoC below-range value is not representable in the DBC
```

Do not silently relax the generation goal.

### 10.1 Important current-model consequence

At nominal 100 ms, one temperature quantum is:

$$
0.5^\circ\mathrm C / 0.1\,s
=
5^\circ\mathrm C/s.
$$

Cooling utilization for one negative quantum is:

$$
5/6\approx0.833.
$$

Therefore a one-quantum-per-frame downward drift already produces
`PHYSICAL_TEMP_RATE / WARNING`.

Likewise, at the hot-state heating limit:

$$
5/5=1.0,
$$

so one positive quantum per nominal frame also produces
`PHYSICAL_TEMP_RATE / WARNING`.

Consequently, recipes previously described as "isolated" purely because they
stay below the violation threshold are **not isolated under the current warning
semantics**.

The solver must use the generation goal to decide whether such co-detections
are allowed or make the request unsatisfiable.

---

## 11. Mutation operators and trajectory construction

Canonical operator semantics come from `fault_injection_model.yaml`.

### `stuck`

```text
one temperature signal
duration_samples >= configured stuck window
flat within epsilon_stuck
requires independent peer excitation
```

### `spike`

```text
one signal
temporary delta for duration_samples
return edge is part of the generated trajectory and must be verified
```

### `drift`

```text
one signal
quantized monotonic or piecewise-monotonic trajectory
actual trajectory is solved from the generation goal
```

`rate_per_sample` is a requested operator parameter, not proof that the desired
Guardian observation will occur.

### `out_of_range`

```text
one signal
target value outside configured admissible range
value must remain DBC-representable
```

### `signal.combination`

```text
>= 2 distinct signal mutations
all component mutations belong to one injection instance
```

Combination generation may be used to preserve non-target invariants while
driving the requested one.

---

## 12. Search strategy

A full nonlinear optimizer is not required for v1.

Use deterministic bounded search around analytically derived targets:

```text
1. derive ideal continuous target;
2. enumerate nearby quantized candidates;
3. enumerate allowed signal combinations if needed;
4. forward-evaluate each complete trajectory;
5. choose the first candidate satisfying the generation goal;
6. break ties deterministically using minimum mutation magnitude, then signal
   order, then lexical operator order.
```

Recommended optimization objective:

```text
minimize:
    total absolute signal modification
then:
    number of mutated signals
then:
    mutation duration
```

Subject to satisfying the generation goal.

The same seed, template, model, injection instance, and generation goal must
produce the same output.

---

## 13. Forward model oracle

The mutator and Guardian must use identical model semantics.

Preferred implementation:

```text
shared pure battery-model library
```

containing:

```text
thermal normalization
thermal thresholds
spread limit
hotspot limit
temperature-rate limit
SoC rate/range rules
stuck rule
classification boundaries
```

If code sharing is not practical, both implementations must execute the same
versioned conformance vectors.

Normative boundary vectors must include:

```text
utilization immediately below 0.8
utilization exactly 0.8
utilization exactly 1.0
utilization immediately above 1.0

temp_max immediately below warning threshold
temp_max exactly warning threshold
temp_max immediately below critical threshold
temp_max exactly absolute_max_c
temp_max strictly above absolute_max_c

source/generation-interval ($\Delta\tau$) rate cases
receive-axis freshness cases (projected relative now)
SoC rate cases
stuck window / excitation cases
```

`utilization == 1.0` is `WARNING`, not `VIOLATION`.

---

## 14. Complete case-generation pipeline

For each case:

1. parse the reference ASC;
2. collect timestamped battery frames `0x100`, DLC 16;
3. preserve the configured lead-in byte-identically;
4. load and hash the exact Guardian model;
5. load the injection instance;
6. load the generation goal;
7. derive inverse constraints;
8. construct candidate quantized trajectories;
9. forward-evaluate every candidate;
10. reject candidates violating forbidden observations;
11. emit `UNSATISFIABLE` if no candidate exists;
12. render the selected mutated ASC;
13. emit injection ground truth;
14. emit test-oracle metadata;
15. integration-test the generated replay against the real Guardian.

---

## 15. Ground-truth sidecar

Injection ground truth remains independent of Guardian output.

Example:

```yaml
run_id: <campaign run>
injection_id: spread-warning-001
injected_class: signal.drift

started_at: <ISO-8601>
duration_ms: 700

battery_model:
  path: product/config/battery_guardian/guardian_model.yaml
  sha256: <exact file hash>

mutations:
  - signal: temp_min
    operator: drift
    parameters:
      direction: decrease
      executed_values: [...]
```

For `signal.combination`, record all component mutations.

Do not write Guardian detections into injection ground truth.

---

## 16. Test-oracle sidecar

Keep expected Guardian behavior in a separate oracle section/artifact:

```yaml
generation_goal:
  primary:
    - class: PHYSICAL_TEMP_SPREAD
      level: WARNING

  allowed:
    - class: PHYSICAL_TEMP_RATE
      level: WARNING

  forbidden:
    - class: PHYSICAL_TEMP_SPREAD
      level: VIOLATION
    - class: PHYSICAL_TEMP_ABSOLUTE_LIMIT
      level: VIOLATION
```

Also record the model-predicted trigger samples / intervals when useful.

The integration test passes only when the real Guardian output conforms to this
oracle.

---

## 17. Example: inverse spread warning

Assume baseline:

```text
temp_min = 42.0
temp_avg = 47.0
temp_max = 49.0
```

First compute:

$$
\theta=\theta(47)
$$

and:

$$
L_S=S(47).
$$

For warning target utilization:

$$
u_t=0.9.
$$

Choose:

$$
S_{\mathrm{target}}=0.9L_S.
$$

If mutating only `temp_min`:

$$
T_{\min}^{*}=49.0-S_{\mathrm{target}}.
$$

Then:

1. enumerate nearby 0.5 °C representable values;
2. recompute actual spread utilization;
3. compute temporal rate against the preceding received sample;
4. evaluate ordering, hotspot, thermal limit, absolute limit, SoC and stuck;
5. accept only if the complete generation goal is satisfied.

If every representable `temp_min` value in the warning band causes a forbidden
rate warning, return `UNSATISFIABLE` or retry with an allowed combination
mutation if the injection class permits it.

---

## 18. Example: inverse spread violation

Compute $L_S$ from the current `temp_avg`.

Select the smallest representable spread satisfying:

$$
S_{\mathrm{obs}}>L_S
$$

plus the configured mutator guard margin.

For `temp_min` mutation:

$$
T_{\min}^{*}=T_{\max}-S_{\mathrm{obs}}.
$$

Forward-verify the result.

Do not assume that "one extra quantum" is always sufficient after the model is
re-evaluated; changing `temp_avg` in a combination mutation changes $L_S$
itself.

---

## 19. Example: coherent thermal warning

Requested observation:

```text
THERMAL_LIMIT / WARNING
```

with no spread/hotspot violation.

Target:

$$
60\le T_{\max}<70.
$$

Use a `signal.combination` and construct a common-mode shift:

$$
T_{\min}^{*}=T_{\min}+\Delta,
$$

$$
T_{\mathrm{avg}}^{*}=T_{\mathrm{avg}}+\Delta,
$$

$$
T_{\max}^{*}=T_{\max}+\Delta.
$$

This preserves instantaneous spread and hotspot differences.

The generated trajectory must still be checked against temperature-rate
warnings/violations. If a single-frame common-mode step would violate the rate
goal, spread the shift over multiple frames or declare the requested oracle
unsatisfiable under the specified operator constraints.

---

## 20. Compact mutator configuration

Mutator-specific configuration should contain only generation mechanics:

```yaml
case_mutator:
  template: <reference.asc>
  output_dir: cases/<run_id>
  repetitions: 1
  lead_in_frames: 20
  seed: 0

  battery_model: product/config/battery_guardian/guardian_model.yaml

  search:
    warning_target_utilization: 0.90
    violation_target_utilization: 1.10
    max_candidate_quanta: 64
```

Do not duplicate:

```text
Guardian thresholds
spread/hotspot coefficients
temperature-rate limits
SoC limits
stuck parameters
stale timeout
```

Those are always read from `guardian_model.yaml`.

---

## 21. Acceptance criteria

The mutator implementation is complete when:

- it reads the authoritative Guardian model;
- it accepts an explicit `generation_goal`;
- it converts the requested `DetectionClass × DetectionLevel` into inverse
  constraints;
- it generates DBC-representable signal values / trajectories;
- it distinguishes `WARNING`, `VIOLATION`, and `CRITICAL` correctly;
- it uses the source/generation interval $\Delta\tau$ for dynamic constraints and the receive axis only for freshness/transport;
- it supports coordinated `signal.combination` mutations;
- it forward-verifies every generated candidate against the Guardian model;
- it rejects forbidden co-detections;
- it returns structured `UNSATISFIABLE` instead of silently relaxing the goal;
- it emits injection ground truth independently of Guardian observations;
- it records the exact Guardian-model path and hash;
- it is deterministic for identical input, model, seed, injection and goal;
- generated integration cases reproduce the requested Guardian observations in
  the real Guardian.
