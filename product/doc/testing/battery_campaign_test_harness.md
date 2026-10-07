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

product/doc/can/battery_can_fd_replay.md
    product CAN FD payload and Vector ASC representation

product/config/battery_campaign/harness.yaml
    selected scenarios, campaign files, and experiment output location
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
- uses CAN FD frame `0x100`, DLC code `0xA`, a 16-byte payload, and the
  canonical product DBC/ASC encoding;
- has same-prefix `.ground_truth.yaml` and `.oracle.yaml` sidecars;
- is directly replayable without mutation.

### 3.2 Decided scenario roles

| ID | File | Role | Initial campaign use | Baseline expectation |
|---|---|---|---|---|
| `cold_nominal` | `0_cold_nominal.asc` | cold, physically plausible operation | mutation template | no Guardian detection |
| `warm_nominal` | `1_warm_nominal.asc` | moderate, physically plausible operation | mutation template | no Guardian detection |
| `hot_nominal` | `2_hot_nominal.asc` | hot but admissible operation | mutation template | `THERMAL_LIMIT/WARNING/temp_max` throughout |
| `overtemp_fault` | `3_overtemp_fault.asc` | genuine overtemperature behavior | unmodified positive regression | expected thermal/physical observations |
| `hotspot_fault` | `4_hotspot_fault.asc` | genuine local hotspot behavior | unmodified positive regression | expected hotspot-related observations |

The initial elementary-fault matrix uses only the three nominal scenarios as
mutation templates. The two genuine-fault scenarios must not be used as generic
templates until combined-fault expectations are specified separately.

### 3.3 Decided scenario details

The committed ASC files are the authoritative trajectories; the specification
does not duplicate all 200 samples. They contain deterministic, quantized,
piecewise-smooth values and no random noise.

Golden Scenarios contain genuine reference behavior but no injected incidents,
so each `.ground_truth.yaml` is the empty list `[]`. This does not mean that no
Guardian observation is expected. The same-prefix `.oracle.yaml` is the
authoritative baseline expectation:

- `cold_nominal` and `warm_nominal` produce no detections;
- `hot_nominal` deliberately produces
  `THERMAL_LIMIT/WARNING/temp_max` throughout the replay;
- `overtemp_fault` progresses through thermal Warning and Critical behavior
  and also exercises absolute-limit and temperature-rate observations;
- `hotspot_fault` progresses through Hotspot Warning/Violation and then Spread
  Warning/Violation.

The two genuine-fault traces are positive regressions, not recovery tests.
Their fault state may remain active at the end; their baseline oracle must not
require clearing. Generated five-incident experiments retain the independent
recovery requirements from Section 4.2.

Golden Oracle entries describe transitions as `class`, `level`, `state`, an
optional `signal`, and `at_ms`. A scalar `at_ms` is one exact source timestamp.
Recurring transitions use the inclusive compact form:

```yaml
at_ms: { from: 600, through: 19800, every: 600 }
```

`signals` expands one otherwise identical transition for every listed signal.
`allow_unspecified: false` makes any additional Guardian transition a mismatch.
The DFM expectation is derived from `guardian_diagnostics.json`; unmapped
Guardian transitions remain `NOT_APPLICABLE` on that plane.

`golden_scenarios/validation.json` remains a derived per-frame summary for
human review. It is not the Collector oracle and must not replace the transition
YAMLs.

When the harness skeleton is implemented, every ASC and both same-prefix YAML
sidecars move together into the target `scenarios/` directory from Section 5.1.
Until then, `harness.yaml` is the sole owner of their physical prefixes.

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

All entries except `signal.combination` are elementary campaigns. Combination
is a separate explicitly configured campaign whose incidents each contain at
least two distinct signal mutations. For every applicable reference scenario, the
campaign generator creates one experiment containing five incidents of that
class. A campaign may restrict its applicable scenarios when a requested case
is physically or representably unsatisfiable.

The initial configured matrix is therefore:

```text
7 elementary campaigns x 3 nominal reference scenarios = 21 experiments
1 explicit combined campaign x warm_nominal           =  1 experiment
                                                        22 experiments total
```

This is a maximum, not a requirement to generate semantically invalid cases.

### 4.2 Decided incident rules

Every generated experiment:

- retains the exact 20-second source timeline from `0` through `19_900` ms;
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

CAN delay may extend wall-clock replay duration beyond 20 seconds because ASC
delivery time and embedded source time are distinct. Incident slots and oracle
windows remain on the 20-second source timeline.

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

### 4.4 Decided standard variation profile

`variation: standard` has five ordered positions. The generator derives exact,
DBC-representable values from the Guardian model and then forward-verifies
them. A quantum means the smallest representable change of the target signal.

| Campaign | Five incident goals |
|---|---|
| `signal.stuck` | `N-1` samples (control), `N` samples (boundary), `N+1` samples, `ceil(1.5N)` samples, `2N` samples; target flatness and independent excitation remain mandatory |
| `signal.spike` | temperature-rate utilization just below warning, 0.9, 1.0, one quantum above 1.0, and a strong violation; return edge included |
| `signal.drift` | Hotspot utilization just below warning, 0.9, 1.0, one quantum above 1.0, and a strong violation; drift pace stays below Rate Warning where satisfiable |
| `signal.out_of_range` | legal lower boundary, one quantum below it, legal upper boundary, one quantum above it, and a strong high violation |
| `transport.delay` | CAN replay-delivery gaps of 400, 500, 700, 1000, and 2000 ms while preserving payload source timestamps |
| `transport.drop` | omitted CAN replay frames producing gaps of 400, 500, 700, 1000, and 2000 ms without rebasing later source timestamps |
| `source.dropout` | source-suspension gaps of 400, 500, 700, 1000, and 2000 ms |

Here `N` is `guardian.stuck.window_samples`. With the current 500-ms stale
timeout, 400 ms is a non-triggering control, 500 ms exercises the strict
boundary, and 700 ms is the first robust stale case including one evaluation
period of margin. The last three transport/source cases are binary violations
of increasing duration, not invented warning severities.

The drift campaign has one stable primary purpose: slowly create
`PHYSICAL_TEMP_HOTSPOT` through `temp_max`. Spread and Rate observations are
allowed only when the generated oracle states them. Other drift targets belong
in explicit override campaigns rather than changing the meaning of the
default.

### 4.5 Decided CAN transport semantics

In v1, `transport` denotes delivery on the ASC/CAN replay path before the CAN
Provider. It does not denote the uProtocol/Zenoh link and requires no network
proxy.

- `transport.delay` changes ASC replay timestamps and therefore CAN delivery
  time while preserving the embedded source-generation timestamp;
- `transport.drop` omits selected CAN frames while preserving all later replay
  and embedded source timestamps;
- `source.dropout` states that the source generated no frame in the interval.

An offline replay can render `transport.drop` and `source.dropout` identically.
Their distinction is deliberate injection ground truth, not something the
Guardian can infer from `STREAM_STALE`. Delay and drop are generated before
execution; the runner does not apply live transport mutation.

### 4.6 Decided timing envelope

`schedule: even` divides the source timeline into a lead-in, five equal
incident slots, and a final drain interval:

```text
lead-in       [    0,  1000) ms
incident 1    [ 1000,  4500) ms
incident 2    [ 4500,  8000) ms
incident 3    [ 8000, 11500) ms
incident 4    [11500, 15000) ms
incident 5    [15000, 18500) ms
final drain   [18500, 20000) ms
```

Each slot contains both its mutation and the recovery needed before the next
slot. Exact start and end times are solver outputs and need not sit at the slot
boundaries. The fifth incident must recover inside its slot; the final drain is
reserved for evidence delivery and final-state checks. If a requested profile
cannot fit and clear in its slot, that campaign/scenario pair is
`UNSATISFIABLE`; slots must not be merged or allowed to overlap silently.

## 5. Configuration and directory contract

The source format is intentionally split into a small harness configuration,
compact default campaigns, and explicit combined campaigns. All paths in these
files are repository-relative except campaign file names, which are relative to
`harness.yaml`.

### 5.1 Decided directory layout

Configuration remains below `product/config`; test assets and generated
artifacts remain below `product/tests`. The target layout is:

```text
product/config/battery_campaign/
  harness.yaml
  default_campaigns.yaml
  combined_example.yaml

product/tests/battery_campaign/
  scenarios/
    0_cold_nominal.{asc,ground_truth.yaml,oracle.yaml}
    1_warm_nominal.{asc,ground_truth.yaml,oracle.yaml}
    2_hot_nominal.{asc,ground_truth.yaml,oracle.yaml}
    3_overtemp_fault.{asc,ground_truth.yaml,oracle.yaml}
    4_hotspot_fault.{asc,ground_truth.yaml,oracle.yaml}
  experiments/
    <campaign-id>/<scenario-id>/
      experiment.yaml
      case.asc
      case.ground_truth.yaml
      case.oracle.yaml
      evidence/
        <run-id>/
          collector.json
          verdict.json
          report.md
          logs/
          dfm/
```

The Golden Scenario files currently remain in the Case Mutator directory until
the harness implementation moves them. `harness.yaml` maps each scenario ID to
one prefix; `.asc`, `.ground_truth.yaml`, and `.oracle.yaml` are appended to
that prefix. This is also the prefix passed to the Evidence Collector.

The common `case` prefix deliberately matches the current Evidence Collector
interface: given `<experiment-dir>/case`, it finds `case.asc` and
`case.ground_truth.yaml` directly. The runner supplies `case.oracle.yaml` and
the run directory below `evidence/` explicitly. No component reconstructs
paths from a campaign name. Separate run directories permit reruns without
overwriting earlier evidence.

### 5.2 Harness configuration

```yaml
schema_version: 1
scenarios:
  cold_nominal: <path-prefix>
  warm_nominal: <path-prefix>
scenario_groups:
  nominal: [cold_nominal, warm_nominal]
campaign_files: [default_campaigns.yaml, combined_example.yaml]
output_dir: product/tests/battery_campaign/experiments
execution:
  readiness_timeout_ms: 10000
  guardian_slack_ms: 100
  dfm_slack_ms: 500
  drain_ms: 3000
  continue_after_infrastructure_error: true
```

The complete initial configuration is
`product/config/battery_campaign/harness.yaml`. Duration, cycle, CAN encoding,
and scenario roles are fixed by Sections 3.1 and 3.2 and are therefore not
repeated in YAML.

### 5.3 Compact default campaigns

```yaml
schema_version: 1
defaults:
  scenarios: nominal
  incidents: 5
  schedule: even
  variation: standard
campaigns: [signal.stuck, signal.spike, signal.drift]
```

`incidents` controls frequency. `variation: standard` requests the canonical
five-point, class-specific strength profile described in Section 4.4. It does
not mean five synthetic detection levels: continuous and binary checks retain
their different semantics. `schedule: even` asks the generator to distribute
incidents and required recovery intervals across the 20-second trace. If that
is impossible, generation returns `UNSATISFIABLE`.

The complete defaults live in
`product/config/battery_campaign/default_campaigns.yaml`. A campaign may be
expanded to mapping form later only when it overrides a default; the scalar
form remains the preferred form.

### 5.4 Explicit combined campaigns

A combined campaign uses one explicitly timed incident list. Every `combine`
entry is one `signal.combination` incident and contains exactly two elementary
signal faults:

```yaml
campaign: { id: combined_example, scenario: warm_nominal }
incidents:
  - at_ms: 2000
    combine:
      - { fault: signal.stuck, signal: temp_avg, strength: medium }
      - { fault: signal.spike, signal: temp_max, strength: high }
```

`strength` is an abstract generation goal (`low`, `medium`, or `high`), not a
Guardian `DetectionLevel`. The generator resolves it using the authoritative
Guardian and fault-injection configurations, records the executed numeric
values in ground truth, and forward-verifies the complete trajectory. Exact
operator parameters may be added to an individual fault as an override, but
they are intentionally absent by default.

The three labels select these points from the standard profile:

| Operator | `low` | `medium` | `high` |
|---|---|---|---|
| `stuck` | `N` samples | `ceil(1.5N)` samples | `2N` samples |
| temperature `spike` or `drift` | utilization 0.9 | smallest representable violation above 1.0 | utilization 1.25 or next representable value above it |
| `out_of_range` | 1 quantum outside the selected bound | 5 quanta outside | 10 quanta outside |

Every selected value remains subject to representability, recovery, and full
forward verification. An impossible pair is `UNSATISFIABLE` rather than
silently weakened.

The single complete five-incident example is
`product/config/battery_campaign/combined_example.yaml`.

## 6. Generation phase

### 6.1 Decided behavior

Generation happens before system execution. For each campaign/scenario pair:

1. load and validate the reference scenario;
2. load the campaign and authoritative Guardian configuration;
3. generate all five incidents into one 20-second source trajectory;
4. quantize and render the complete ASC;
5. forward-evaluate the rendered ASC with the real Guardian model;
6. verify every incident oracle and every required recovery;
7. emit either a complete experiment bundle or structured `UNSATISFIABLE`.

Generation must be deterministic for identical inputs. It must never relax an
oracle silently.

### 6.2 Experiment bundle

The minimal bundle is:

```text
<campaign-id>/<scenario-id>/
  experiment.yaml
  case.asc
  case.ground_truth.yaml
  case.oracle.yaml
  evidence/<run-id>/
```

No model hash, configuration hash, trace hash, artifact hash, or provenance
sidecar is generated.

`experiment.yaml` identifies the campaign, scenario, run duration, and the
other bundle files. `case.ground_truth.yaml` contains the five executed
injections. `case.oracle.yaml` contains baseline, per-incident, clearing, and
end-state expectations. Execution writes only below `evidence/<run-id>/` and
never modifies the pre-generated files. A default run ID is
`<campaign-id>--<scenario-id>--<UTC-basic-timestamp>`; callers may supply a
different filesystem-safe ID. It is propagated unchanged through all evidence
planes.

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
load the same-prefix ground truth and oracle
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

For a Golden Scenario, empty ground truth means "no injected cause" only. The
Collector must still evaluate all transitions in the Oracle YAML. In
particular, it must not apply the current legacy shortcut that treats every
empty-ground-truth run as a no-fault baseline.

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

## 8. Decided execution contract

### 8.1 Runner and isolation

The v1 runner is a small Python program executed directly inside the existing
DevContainer. It uses the DevContainer's Python, PyYAML, and subprocess support
and invokes the Rust binaries and existing infrastructure commands. The
harness has no dedicated Docker image or nested build environment.

Every experiment is process-isolated:

1. stop processes owned by the preceding experiment;
2. create `evidence/<run-id>/`, including a fresh `dfm/` storage directory;
3. start a fresh stateful system path, including Databroker, VSS bridge,
   Guardian, DFM, and OpenSOVD bridge;
4. run readiness checks;
5. start the Evidence Collector and wait for subscription readiness;
6. start the one-shot replay.

Reusing DFM storage or Guardian/Databroker process state across experiments is
forbidden. The runner must terminate only processes it started. Process logs
are retained under the run's `logs/` directory.

### 8.2 Readiness and startup order

Readiness is condition-based and bounded by `readiness_timeout_ms`; fixed sleep
delays are not readiness checks. At minimum the runner verifies:

- Zenoh and the KUKSA Databroker accept connections;
- DFM has loaded the catalog and answers its query interface;
- OpenSOVD exposes the configured Guardian component;
- the Guardian HTTP health state includes active uProtocol subscriptions,
  raw-evidence publication readiness, and DFM reporting readiness;
- the VSS bridge is subscribed and ready to publish complete battery samples;
- the Evidence Collector confirms that all required subscriptions are active.

The current Guardian `/health` response and Collector console output are not
yet sufficient machine-readable readiness contracts. Their implementation must
be strengthened before the runner relies on them. A readiness failure produces
`INCONCLUSIVE`, not a model `FAIL`.

### 8.3 Completion, drain, and campaign continuation

Replay is complete only when both the replay process exits successfully and the
Collector observes the expected terminal source timestamp (`19_900` ms for the
standard traces). The Collector then remains active for `drain_ms` to accept
late Guardian and DFM evidence. Failure of the replay process, absence of the
terminal battery evidence, or loss of a required component is
`INCONCLUSIVE`.

The default drain is 3000 ms, matching the Collector's current idle timeout.
After an infrastructure failure the campaign continues with the next isolated
experiment by default. A future CLI may offer `--fail-fast`, but it does not
change the default or any recorded verdict.

### 8.4 Evidence windows

Ground truth and oracle windows remain source-relative and are never widened by
wall-clock delay. Plane-specific right-hand slack accounts only for periodic
evaluation and asynchronous projection:

```text
Battery input:       exact source timestamps, no slack
Guardian decisions: oracle window + guardian_slack_ms on the right
DFM/OpenSOVD:        matching Guardian transition + dfm_slack_ms on the right
```

Defaults are one Guardian evaluation period (100 ms) and 500 ms for DFM
projection. Slack never changes an incident's start, makes overlapping
incidents legal, or permits an event from the next slot to satisfy the previous
one.

### 8.5 Guardian-to-DFM semantics

For a class/level pair present in `guardian_diagnostics.json`, Guardian Active
maps to DFM `Failed` and Guardian Cleared maps to DFM `Passed`. The current DFM
state must therefore reflect whether the mapped decision is active. Retained
records and latched ISO-14229 history bits are not required to disappear when
the current fault passes.

Guardian decisions without a DFM mapping, including the utilization warnings
called out in the model, have DFM status `NOT_APPLICABLE`; their absence from
DFM is neither `PASS` nor `FAIL`. The raw Guardian plane remains authoritative
for those decisions.

The two genuine-fault Golden Scenarios use their committed baseline snapshot.
They do not require clearing at replay end. Fresh process and DFM state still
apply before each such regression run.

### 8.6 Reports and retention

The Collector writes `collector.json`. The runner writes a stable
machine-readable `verdict.json` and derives `report.md` from it. The JSON
contains at least the run, campaign, scenario and incident IDs; per-plane
results; observed transitions; infrastructure errors; and the final tri-state
verdict. Markdown is presentation only and must not contain evaluation facts
absent from the JSON.

Run evidence remains under `evidence/<run-id>/`. The runner never deletes or
overwrites an existing run directory; reuse of a run ID is an error.

## 9. Implementation-agent constraints

Until the open points are resolved, implementation agents must follow these
rules:

- do not alter the committed Golden Scenario trajectories or their baseline
  expectation snapshot implicitly;
- do not use the two genuine-fault scenarios as mutation templates;
- do not invent warning levels for binary Guardian checks;
- do not add hashes or provenance artifacts;
- do not combine generation and live execution;
- do not classify missing evidence as a Guardian failure;
- do not weaken or auto-expand required/allowed/forbidden oracle sets;
- preserve the 20-second source duration and five-incident structure;
- emit product frame `0x100` only in the canonical CAN FD ASC form;
- surface an unsatisfied timing or model constraint as `UNSATISFIABLE` or an
  open design issue rather than silently changing the campaign.

## 10. Remaining specification work

Before implementation continues:

1. define the exact generated `experiment.yaml`, multi-incident ground-truth,
   and `verdict.json` schemas and align generated Oracle output with the Golden
   transition vocabulary;
2. define the machine-readable readiness payloads for Guardian, VSS bridge, and
   Evidence Collector;
3. define the runner command interface and Make targets;
4. reconcile the current single-injection Mutator and mapped-only Collector
   implementations with this multi-incident, three-plane contract;
