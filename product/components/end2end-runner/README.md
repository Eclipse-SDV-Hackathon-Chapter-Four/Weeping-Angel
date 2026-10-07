# end2end-runner — Golden Run Orchestration

Drives the **product evidence chain** end to end on a single host (dev
container), one replay per case, and produces a tri-state verdict per case:

```text
case_mutator ──▶ start_can.sh (ASC replay) ──▶ kuksa-databroker ──▶ vss_publisher
                                                                    │ uProtocol/Zenoh
                                                                    ▼
evidence_collector ◀──────────────────────────────────────────── guardian
        ▲                                                          │ iceoryx2 dfm/event
        │ verdict per case                                         ▼
     runs/<ts>/<case>/                        dfm_bin ──▶ dfm_sovd_bridge (:7690)
```

Status: transitional one-command runner for **golden runs** (baseline +
`signal.out_of_range`). The durable runner (campaign harness, harness-spec
§7/§8 open points: reset semantics, drain timing, runner technology) is
specified separately; this script deliberately keeps cases as data so it can
grow into that runner without a rewrite.

The runner provisions its own Python venv for the CAN replay (`$HOME/.venv`,
override via `E2E_VENV`) when missing — the devcontainer `post-create` step
having run or not does not matter.

## Usage

Inside the dev container:

```sh
cd product/components/end2end-runner
./run_golden.sh                    # all registered cases (baseline + fault case)
./run_golden.sh baseline           # a single case
E2E_REBUILD=1 ./run_golden.sh      # force binary rebuild first
E2E_REGEN_CASES=1 ./run_golden.sh  # regenerate mutator case artifacts
```

Results per case land in `runs/golden-<timestamp>/<case>/`:

| File | Content |
|---|---|
| `verdict.txt` | Collector's verdict line |
| `report.md` | Collector's Markdown report |
| `collector.out` / `collector.log` | Collector stdout/stderr |
| `guardian.log`, `dfm_bin.log`, `dfm_sovd_bridge.log` | Per-case component logs |
| `replay.log` | CAN provider replay log |
| `dfm_storage/` | Wiped and recreated for every case (clean reset) |

Exit code: `0` only if every case verdict is `PASS`; `1` otherwise
(`FAIL`, `INCONCLUSIVE`, `TIMEOUT`, `ERROR`).

## Runner configuration (environment)

| Variable | Default | Meaning |
|---|---|---|
| `ZENOH_CONNECT` | `tcp/127.0.0.1:7447` | uProtocol/Zenoh router endpoint for all subscribers/publishers |
| `DATABROKER_ADDR` | `http://127.0.0.1:55555` | gRPC address of kuksa-databroker for `vss_publisher` |
| `GUARDIAN_SOVD_PATH` | `battery_guardian` | SOVD component id / DFM entity path |
| `E2E_IDLE_TIMEOUT_S` | `3` | Collector drain window after the replay end |
| `E2E_CASE_TIMEOUT_S` | `240` | Hard deadline per case for the collector |
| `E2E_REBUILD` | `0` | `1` = rebuild all binaries before the run |
| `E2E_REGEN_CASES` | `0` | `1` = regenerate mutator-generated case artifacts |
| `E2E_VENV` | `$HOME/.venv` | Python venv used for the CAN replay; created + populated by the runner when missing (mirrors the devcontainer `post-create` convention) |

## Component arguments (as used by the runner)

### zenohd (uProtocol bus, C6)

Started via `tools/start_zenohd.sh` (skips if port is taken):

```sh
zenohd -l tcp/127.0.0.1:7447 --no-multicast-scouting
```

### kuksa-databroker (C4)

Started via `tools/start_databroker.sh` (skips if port is taken):

```sh
databroker --address 127.0.0.1 --port 55555 --vss "$KUKSA_VSS_FILE,product/config/vss_overlay.json" --insecure
```

`KUKSA_VSS_FILE` is set by the dev container image. The overlay adds custom
product paths missing from the standard catalogue
(`Vehicle.Powertrain.TractionBattery.SourceTimestamp`, mapped from the CAN
`TimeStamp`); the feeder exits if any mapped path is unknown to the broker.

### vss_publisher — VSS uProtocol bridge (C5)

```sh
DATABROKER_ADDR=http://127.0.0.1:55555 \
ZENOH_CONNECT=tcp/127.0.0.1:7447 \
vss_publisher            # no CLI args; publishes //battery-vss/9001/1/9001
```

Started once per run, kept across cases, restarted by the runner if it dies.
Note: the binary's built-in default address is `http://kuksa-databroker:55555`
(compose hostname); the runner always overrides it for host-local runs.

### dfm_bin — Diagnostic Fault Manager (C8)

```sh
dfm_bin --catalog-dir <product>/config/battery_guardian \
        --storage-dir <run>/<case>/dfm_storage
```

- `--catalog-dir` (required): directory scanned for `*.json` fault catalogs;
  here `guardian_diagnostics.json`.
- `--storage-dir` (required): persistent KVS fault-state storage; **wiped per
  case** by the runner (clean reset). Both flags are mandatory — `dfm_bin`
  exits immediately without them.

### dfm_sovd_bridge — OpenSOVD server over the DFM (C9)

```sh
DFM_SOVD_PATH=battery_guardian dfm_sovd_bridge     # serves :7690
```

| Env | Default | Meaning |
|---|---|---|
| `SOVD_URL` | `http://127.0.0.1:7690/sovd` | Base URL it serves |
| `DFM_SOVD_PATH` | `battery_guardian` | DFM entity path / SOVD component id |
| `DFM_SOVD_NAME` | `Battery Guardian` | Display name |
| `DFM_QUERY_TIMEOUT_MS` | `1000` | Per-query iceoryx2 timeout |
| `DFM_STARTUP_WAIT_S` | `10` | How long to wait for the DFM at startup |

Readiness check used by the runner:
`GET /sovd/v1/components/battery_guardian/data/faults` returns the catalog.

### guardian (C7)

```sh
GUARDIAN_CONFIG=<product>/config/battery_guardian/guardian_model.yaml \
GUARDIAN_FAULT_CATALOG=<product>/config/battery_guardian/guardian_diagnostics.json \
ZENOH_CONNECT=tcp/127.0.0.1:7447 \
HOST=127.0.0.1 PORT=8080 \
guardian                 # HTTP /health + /state on :8080
```

All four env vars have built-in defaults, but the runner pins them explicitly
(config/catalog live in `product/config/battery_guardian/`). Subscribes to
`//battery-vss/9001/1/9001`; publishes mapped `GuardianFaultEvent`s on
`//guardian/1001/1/8001` (ADR-012 evidence plane) and reports to the DFM over
iceoryx2.

### start_can.sh — CAN replay (C3)

```sh
../tools/start_can.sh <file.asc>  # from end2end-runner; foreground, returns after the replay
```

Replays the ASC once through the KUKSA CAN provider (`dbcfeeder.py`) into the
databroker on `127.0.0.1:55555` (must already run). DBC/mapping are pinned to
`product/config/battery_temp.dbc` + `vss_dbc.json`. Needs the container Python
venv (python-can, cantools, kuksa-client).

### case_mutator (C2)

Used at case-preparation time (not during the run):

```sh
cd ../case_mutator
cargo run --locked -- \
  --request examples/out_of_range.yaml \
  --output-dir ../end2end-runner/cases/signal_out_of_range
```

Emits `<injection-id>.asc`, `<injection-id>.ground_truth.yaml`,
`<injection-id>.oracle.yaml`. Deterministic; regenerate with
`E2E_REGEN_CASES=1`. Known limitation: one injection per case (see
`docs/project_notes/bugs.md`); multi-incident experiments are future harness
work (ADR-014).

### evidence_collector (C10)

```sh
ZENOH_CONNECT=tcp/127.0.0.1:7447 \
evidence_collector <prefix> \
  --idle-timeout 3 \
  --report <run>/<case>/report.md
```

| Arg/Env | Meaning |
|---|---|
| `<prefix>` | Path prefix; reads `<prefix>.asc` (replay) + `<prefix>.ground_truth.yaml` (fallback `.json`) |
| `--idle-timeout SECS` | Drain window after the replay end on the battery timeline |
| `--report FILE` | Write the Markdown report there |
| `--fault-topic URI` | Optional; default `//guardian/1001/1/8001` |
| `--battery-topic URI` | Optional; default `//battery-vss/9001/1/9001` |
| `--expectations FILE` | Optional; default `expected_observations.yaml` in the component |
| `ZENOH_CONNECT` | Zenoh router endpoint |

Exit codes: `0` PASS, `1` FAIL, `2` INCONCLUSIVE, `3` usage/input error. The
runner passes these through as case verdicts.

## Cases — how to add one

Cases are data. Register in `CASES` (top of `run_golden.sh`):

```bash
CASES=(
  "baseline|baseline/baseline"                 # name|prefix relative to cases/
  "signal_out_of_range|signal_out_of_range/signal_out_of_range"
  # add: "<name>|<dir>/<prefix>"
)
```

The prefix directory must contain `<prefix>.asc` and
`<prefix>.ground_truth.yaml` (or `.json`). Two ways to provide them:

1. **Generated** — add a `prepare_<name>()` function (like
   `prepare_signal_out_of_range`, which invokes the mutator).
2. **Committed** — check the artifacts in under `cases/<name>/`; no code
   change beyond the registry line.

**Baselines** are ordinary cases with an empty ground truth (`[]`); the
collector passes a baseline case only when the Guardian reports no fault.
Future reference scenarios from `doc/testing/battery_campaign_test_harness.md`
(`cold_nominal`, `warm_nominal`, `hot_nominal`, …) slot in exactly this way,
as do harness experiment bundles (`experiment.yaml` + `input.asc` +
`ground_truth.yaml` + `oracle.yaml`); only the collector-side oracle
evaluation is not wired yet (ADR-012 verdict plane).

## Known boundaries

- Verdicts evaluate the **mapped** fault stream (ADR-012). Raw
  `GuardianEvidenceEvent` correlation (ADR-007) and DFM/SOVD verdict
  dimensions (`DFM_MATCH`, `GUARDIAN_CLEARED`) are not implemented yet.
- Single injection per mutator case (bug log: multi-incident is open).
- The runner starts/stops processes by PID; ports 7447/55555 may be shared
  with already-running infra, 7690/8080 must be free at case start.
- Infrastructure (zenoh, databroker, vss bridge) is shared across cases;
  Guardian/DFM/SOVD are reset per case. DFM storage is wiped — evidence
  runs are single-shot, not restart/recovery scenarios (Phase-5 work).
