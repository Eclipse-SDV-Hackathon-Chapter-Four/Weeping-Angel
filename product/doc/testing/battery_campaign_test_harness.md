# Battery Campaign Test Harness

> **Status:** normative discussion specification.
>
> Sections marked **Decided** constrain later implementation. Sections marked
> **Open** record questions for further discussion and must not be silently
> resolved by an implementation agent.

## 1. Purpose and authority

The test harness turns stable battery reference traces into pre-generated fault
experiments and later executes those experiments against the complete system.
It has two deliberately separate phases:

```text
reference trace + campaign specification
                |
                v
       deterministic generation
                |
                v
  ASC + ground truth + oracle
                |
                v
      one isolated system replay
                |
                v
 Evidence Collector data + verdict
```

This document is authoritative for reference-scenario roles, campaign
composition, experiment artifacts, and the intended execution lifecycle. The
following artifacts remain authoritative for their own semantics:

```text
product/doc/mutator/case_mutator_model.md
    mutation mechanics and inverse generation

product/doc/battery/battery_guardian_model.md
    Guardian formulas, boundaries, and detection semantics

product/config/battery_guardian/guardian_model.yaml
    Guardian model parameters

product/config/battery_guardian/fault_injection_model.yaml
    canonical injected fault classes and operators

product/config/battery_guardian/guardian_diagnostics.json
    Guardian-to-DFM projection
```

The harness must not redefine any of those semantics.

## 2. Terminology

```text
reference scenario
    A version-controlled, unmodified 20-second battery ASC trace with a known
    baseline expectation.

campaign
    A reusable definition for one canonical injected fault class, including
    its incident shapes and expected observations.

experiment
    One campaign instantiated for one applicable reference scenario.

incident
    One bounded occurrence of the experiment's injected fault class.

ground truth
    What was deliberately injected, where, and with which executed values.

oracle
    Which Guardian and DFM observations are required, allowed, or forbidden.

execution
    One isolated replay of one generated experiment through the system.
```

Injected fault class, Guardian observation, and DFM diagnostic remain separate
concepts. The harness must never infer injection ground truth from Guardian or
DFM output.

## 3. Reference scenarios

### 3.1 Decided common properties

There are five reference scenarios. Every scenario:

- lasts exactly 20 seconds;
- uses the nominal 100-ms generation cycle;
- contains 200 battery frames, beginning at source timestamp `0` ms and ending
  at `19_900` ms;
- uses CAN frame `0x100`, DLC 16, and the canonical product DBC encoding;
- has an explicit baseline oracle;
- is directly replayable without mutation.

### 3.2 Decided scenario roles

| ID | Role | Initial campaign use | Baseline expectation |
|---|---|---|---|
| `cold_nominal` | cold, physically plausible operation | mutation template | no Guardian detection |
| `warm_nominal` | moderate, physically plausible operation | mutation template | no Guardian detection |
| `hot_nominal` | hot but admissible operation | mutation template | explicitly defined after trajectory selection |
| `overtemp_fault` | genuine overtemperature behavior | unmodified positive regression | expected thermal/physical observations |
| `hotspot_fault` | genuine local hotspot behavior | unmodified positive regression | expected hotspot-related observations |

The initial elementary-fault matrix uses only the three nominal scenarios as
mutation templates. The two genuine-fault scenarios must not be used as generic
templates until combined-fault expectations are specified separately.

### 3.3 Open scenario details

The following are intentionally undecided:

- exact temperature and SoC trajectories;
- whether `hot_nominal` remains below all warning thresholds or deliberately
  carries a baseline warning;
- exact activation and recovery profiles of the two genuine-fault scenarios;
- whether nominal traces contain deterministic noise or smooth trajectories;
- file names and final asset directory.

An implementation agent must not invent these values without a follow-up
decision.

## 4. Campaign model

### 4.1 Decided campaign scope

One campaign represents exactly one canonical injected fault class:

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

For every applicable reference scenario, the campaign generator creates one
experiment containing five incidents of that class. A campaign may restrict
its applicable scenarios when a requested case is physically or representably
unsatisfiable.

The initial maximum matrix is therefore:

```text
8 elementary campaigns x 3 nominal reference scenarios = 24 experiments
```

This is a maximum, not a requirement to generate semantically invalid cases.

### 4.2 Decided incident rules

Every generated experiment:

- remains exactly 20 seconds long;
- contains five uniquely identified incidents;
- uses non-overlapping incident windows;
- contains a nominal lead-in before the first incident;
- provides a recovery interval after every incident;
- provides a final observation/drain interval within the replay where possible;
- records requested and actually executed mutation parameters per incident;
- has independent required, allowed, and forbidden observations per incident.

Recovery must be sufficient for the affected Guardian state to clear before the
next incident. A generic fixed recovery duration is not assumed: Stuck, Stale,
Rate, and spatial checks have different temporal behavior.

### 4.3 Detection levels and incident strength

Incident strength does not create new Guardian levels.

Warnings are valid only for Guardian checks with defined warning semantics:

```text
PHYSICAL_TEMP_SPREAD
PHYSICAL_TEMP_HOTSPOT
PHYSICAL_TEMP_RATE
THERMAL_LIMIT
```

The following checks remain binary or otherwise use only their defined levels:

```text
PHYSICAL_TEMP_ORDERING
PHYSICAL_TEMP_ABSOLUTE_LIMIT
PHYSICAL_SOC_RANGE
PHYSICAL_SOC_RATE
SIGNAL_STUCK
STREAM_STALE
```

For utilization-based checks, five incidents may cover points such as:

```text
below warning
warning above the utilization threshold
boundary at utilization == 1.0 (still WARNING)
minimal violation above 1.0
strong violation
```

For binary checks, five incidents instead vary meaningful properties such as:

```text
subthreshold or non-triggering control
legal boundary
minimal violation
strong violation
long or repeated violation
```

The exact five-case profile is campaign-specific and remains open until each
campaign is reviewed.

## 5. Proposed source specifications

The following schemas are proposals for discussion, not frozen interface
contracts.

### 5.1 Reference-scenario catalog

```yaml
schema_version: 1
scenarios:
  - id: cold_nominal
    asc: <path>
    duration_ms: 20000
    cycle_ms: 100
    role: nominal_template
    baseline_oracle:
      forbidden: all

  - id: overtemp_fault
    asc: <path>
    duration_ms: 20000
    cycle_ms: 100
    role: positive_regression
    baseline_oracle:
      primary:
        - class: THERMAL_LIMIT
          level: CRITICAL
```

### 5.2 Campaign definition

```yaml
schema_version: 1
campaign:
  id: temperature_spike_v1
  injected_class: signal.spike
  reference_scenarios:
    - cold_nominal
    - warm_nominal
    - hot_nominal

incidents:
  - id: below_warning
    start_ms: 2000
    duration_ms: 100
    mutation:
      signal: temp_max
      operator: spike
      parameters: {}
    generation_goal:
      primary: []
      allowed: []
      forbidden:
        - class: PHYSICAL_TEMP_RATE
          level: WARNING
        - class: PHYSICAL_TEMP_RATE
          level: VIOLATION

  # Four additional campaign-specific incidents follow.
```

The generator may choose executable values from the Guardian model, but it must
record the resulting values in ground truth and forward-verify the complete
20-second trajectory.

## 6. Generation phase

### 6.1 Decided behavior

Generation happens before system execution. For each campaign/scenario pair:

1. load and validate the reference scenario;
2. load the campaign and authoritative Guardian configuration;
3. generate all five incidents into one 20-second trajectory;
4. quantize and render the complete ASC;
5. forward-evaluate the rendered ASC with the real Guardian model;
6. verify every incident oracle and every required recovery;
7. emit either a complete experiment bundle or structured `UNSATISFIABLE`.

Generation must be deterministic for identical inputs. It must never relax an
oracle silently.

### 6.2 Experiment bundle

The intended minimal bundle is:

```text
<campaign-id>/<scenario-id>/
  experiment.yaml
  input.asc
  ground_truth.yaml
  oracle.yaml
```

No model hash, configuration hash, trace hash, artifact hash, or provenance
sidecar is generated.

`experiment.yaml` identifies the campaign, scenario, run duration, and the
other bundle files. `ground_truth.yaml` contains the five executed injections.
`oracle.yaml` contains baseline, per-incident, clearing, and end-state
expectations.

### 6.3 Required temporal identity

Ground truth and oracle windows use source-relative milliseconds. Dropped frames
do not rebase later source timestamps. Transport delay changes replay/arrival
time without rewriting source-generation timestamps.

An incident ID is sidecar data and is not added to the CAN payload.

## 7. Execution phase

### 7.1 Decided lifecycle

A future runner executes exactly one generated experiment at a time:

```text
prepare/reset system
start Evidence Collector for the experiment run_id
verify required subscribers/components are ready
start ASC replay
wait for replay completion
allow evidence drain
stop/finalize evidence capture
evaluate evidence against ground truth and oracle
write machine-readable verdict and human-readable report
```

The runner must not generate or alter the ASC during execution.

### 7.2 Evidence planes

The Evidence Collector correlates three independent views:

1. incoming `BatteryTempEvent` messages seen by the Guardian path;
2. raw `GuardianEvidenceEvent` decisions before DFM mapping;
3. DFM/OpenSOVD diagnostic state and transitions.

Ground truth identifies the intended cause. Guardian evidence identifies the
model decision. DFM evidence identifies the diagnostic projection. None may be
substituted for another.

### 7.3 Verdict dimensions

Each incident is evaluated independently:

```text
INJECTION_OBSERVED
    Did the expected mutated input reach the evidence plane?

GUARDIAN_MATCH
    Did all required and no forbidden Guardian observations occur in the
    accepted window?

GUARDIAN_CLEARED
    Did the affected Guardian decision clear during recovery?

DFM_MATCH
    Did the configured DFM projection appear and clear as expected?

BASELINE_PRESERVED
    Were baseline expectations satisfied outside incident windows?
```

The experiment verdict is tri-state:

```text
PASS
    All required evidence was available and all expectations matched.

FAIL
    Required evidence was available and demonstrated a behavioral mismatch.

INCONCLUSIVE
    Missing or invalid infrastructure/evidence prevents a behavioral verdict.
```

Infrastructure absence must not be reported as a Guardian-model failure.

## 8. Open execution questions

The following require further discussion before runner implementation:

- concrete process/container orchestration mechanism;
- system reset semantics, especially DFM/OpenSOVD state cleanup;
- readiness checks and startup ordering;
- replay completion and evidence-drain timing;
- accepted timing tolerance around incident windows;
- exact mapping from Guardian transitions to DFM current state;
- handling of baseline detections in the two genuine-fault scenarios;
- report format and retention location;
- whether a campaign stops after infrastructure failure or continues;
- technology choice for the runner.

## 9. Implementation-agent constraints

Until the open points are resolved, implementation agents must follow these
rules:

- do not invent concrete Golden Scenario trajectories;
- do not use the two genuine-fault scenarios as mutation templates;
- do not invent warning levels for binary Guardian checks;
- do not add hashes or provenance artifacts;
- do not combine generation and live execution;
- do not classify missing evidence as a Guardian failure;
- do not weaken or auto-expand required/allowed/forbidden oracle sets;
- preserve the 20-second duration and five-incident structure;
- surface an unsatisfied timing or model constraint as `UNSATISFIABLE` or an
  open design issue rather than silently changing the campaign.

## 10. Next discussion steps

Before implementation continues, agree in this order:

1. the five concrete reference trajectories and their baseline oracles;
2. the five-incident profile for each canonical campaign;
3. incident/recovery timing feasibility within 20 seconds;
4. the final YAML schemas and directory layout;
5. reset, readiness, drain, and evidence-window semantics;
6. runner technology and report rendering.
