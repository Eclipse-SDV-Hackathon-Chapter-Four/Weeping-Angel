# dfm_sovd_bridge (draft)

OpenSOVD server that exposes the faults stored in the DFM for one entity path
(default `battery_guardian`, the Guardian's `GUARDIAN_SOVD_PATH`). Closes the
`DFM → OpenSOVD` link of ADR-009: the DFM only offers a pull API (iceoryx2
request-response `dfm/query`), and `opensovd-gateway` has no DFM provider and
starts with an empty topology.

## How it works

- Uses `dfm_lib::Iceoryx2DfmQuery` on a dedicated worker thread; every SOVD read
  queries the DFM live (no caching).
- Builds an `opensovd-server` with one component `<DFM_SOVD_PATH>`.
- opensovd-core has no SOVD `faults` resource yet, so faults are exposed as
  data resources in the custom category `x-dfm-faults`:
  - `faults` — all faults of the path (`{"items": [...]}`)
  - `faults.active` — only faults with `testFailed`
  - `fault.<code>` — one fault plus its DFM environment data; registered for
    every code the DFM reports at startup

## Run

DFM must be running (`tools/run_dfm.sh`). The bridge serves the same default
URL as `opensovd-gateway` and is meant to replace it; stop the gateway first or
set `SOVD_URL` to another port.

```bash
tools/build_dfm_sovd_bridge.sh
tools/run_dfm_sovd_bridge.sh      # log: run/dfm_sovd_bridge.log
tools/stop_dfm_sovd_bridge.sh
curl -s http://127.0.0.1:7690/sovd/v1/components/battery_guardian/data/faults.active
```

Environment: `SOVD_URL` (default `http://127.0.0.1:7690/sovd`), `DFM_SOVD_PATH`,
`DFM_SOVD_NAME`, `DFM_QUERY_TIMEOUT_MS` (1000), `DFM_STARTUP_WAIT_S` (10).

## Open points

- Replace data resources with a real SOVD `faults` endpoint once opensovd-core
  provides one (or add it as a custom route).
- Faults added to the DFM catalog after startup only appear in `faults`, not as
  `fault.<code>` (restart the bridge).
- `GetAllFaults` returns at most 64 faults per IPC response.
- No delete/clear yet (`DeleteFault` exists in the DFM query API).
