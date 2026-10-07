# Key Facts

Non-sensitive project reference data. Verified against the checkout
2026-10-06 (see also `demo/README.md`, `demo/docs/README.md`, `demo/tutorial.md`).
The `demo/` tree is a reference implementation, not authoritative (ADR-003):
the facts below describe the current checkout, not requirements.
Never store credentials here — this file is committed to git.

## Repository

- Challenge spec: root `README.md` (its Definition of Done = acceptance list)
- Demo app: `demo/` — self-documenting via `demo/README.md`, `demo/docs/README.md`, `demo/tutorial.md`
- Idea input (no authority over ADRs, see `decisions.md` ADR-002): `PLAN.md`
- Build environment: root Nix flake (`nix develop`); build through the devshell, not the user nix profile
- One-command evidence run: `demo/scripts/run_demo.sh` (needs Toxiproxy binaries in `demo/.tools/`, not committed)

## uProtocol Topics (over Zenoh; `up-rust` + `up-transport-zenoh`)

- `battery-vss/9001/1/9001` — BatteryTempEvent JSON (source → Guardian)
- `battery-vss/9001/1/9003` — HighTempAlert JSON
- `guardian-vss/9000/1/9002` — GuardianSnapshot (defined in `demo/services/src/lib.rs`, not yet published)

## Guardian Detection Constants (`demo/services/src/bin/guardian.rs`)

- WARNING ≥ 45 °C, CRITICAL ≥ 55 °C (separate HighTempAlert event ≥ 50 °C)
- Plausible range −40…125 °C; max step between consecutive samples 20 °C
- Stuck: ≥ 5 consecutive identical samples; stale: no sample for 2000 ms (watchdog polls every 500 ms)
- States: Clear/Monitoring/Warning/Critical — `Mitigating` defined in the enum but never entered; Guardian state not published over uProtocol

## Product Guardian

- Component: `product/components/guardien/`
- Rust crate: `Cargo.toml`; source files live in `source/`; binary name is `guardian`
- Model configuration: `product/config/battery_guardian/guardian_model.yaml`; startup fails on missing or inconsistent mandatory parameters
- DFM catalog: `product/config/battery_guardian/guardian_diagnostics.json`; consumed by the existing Guardian DFM reporter
- Injection model: `product/config/battery_guardian/fault_injection_model.yaml`; canonical signals are `temp_min`, `temp_avg`, `temp_max`, and `soc`
- Supported injected classes: `transport.delay`, `transport.drop`, `source.dropout`, `signal.stuck`, `signal.spike`, `signal.drift`, `signal.out_of_range`, and `signal.combination`
- Input: existing BatteryTempEvent uProtocol URI `battery-vss/9001/1/9001`; listener stores `temp_min`, `temp_avg`, `temp_max`, `soc`, and local receive time only
- Evaluation: 100 ms periodic cycle, 500 ms missing-packet timeout, each sample generation evaluated at most once (ADR-004)
- Detections: stream stale; absolute temperature, ordering, spread, hotspot and temperature-rate violations; SoC range/step violations; excitation-gated stuck signals
- Reporting: one dummy `report_detection` function; DFM/Evidence Collector integration intentionally deferred
- HTTP: port 8080 by default, `/health` and `/state`
- Dev-container workflow: `make test`, `make check`, and `make run` from the component directory invoke Cargo directly; the repository-mounted `target/` and Cargo home provide the caches
- The shared `.devcontainer` initializes the Guardian's `fault-lib` submodule, installs rustfmt and Clippy, preinstalls the Codex VS Code extension, and forwards Guardian HTTP port 8080
- `make check` also validates the canonical injection model and runs its malformed-configuration tests
- Verification on 2026-10-07: dev-container `make check` passed formatting, Clippy with warnings denied, all 27 Rust tests, injection-model validation, and all 10 validator tests

## CAN Assets (`demo/can/`)

- Frame 0x100 (256) `BatteryTemperature`, 8 bytes, 100 ms cycle: CellTempAvg (bits 0–15), CellTempMax (16–31), CellTempMin (32–47), StateOfCharge (48–63); scale 0.5, offset −40 (SoC offset 0)
- `battery_temp.asc` = Vector ASC replay input for kuksa-can-provider `--dumpfile`
- `vss_dbc.json` maps signals to `Vehicle.Powertrain.TractionBattery.*` (interval 100 ms)

## Ports (demo)

- 7447 Zenoh (uProtocol bus) · 7448 Toxiproxy(zenoh) · 7690 OpenSOVD gateway · 8080 Guardian HTTP (`/health`, `/state`) · 8474 Toxiproxy API · 55555 kuksa-databroker

## Fault Catalog (`demo/diagnostics/catalog/battery_guardian.json`)

- `BatteryOverTempWarning`, `BatteryOverTempCritical`, `BatteryTempSignalStale`, `BatteryTempSignalStuck`, `BatteryTempImplausible`

## Known Upstream Blocker

- Vendored `demo/opensovd-core` references nonexistent crates `opensovd-cli/lib` + `opensovd-cli/build` → `cargo build -p opensovd-gateway` aborts until fixed upstream; other binaries build cleanly through the devshell
