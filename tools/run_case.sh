#!/usr/bin/env bash
# Run one test case end to end and print the Evidence Collector verdict.
#
#   tools/run_case.sh <prefix> [report.json]
#                                   e.g. tools/run_case.sh product/config/signal_out_of_range
#
# Needs <prefix>.asc and <prefix>.ground_truth.yaml (or <prefix>.json).
# Starts zenohd and a fresh Data Broker (no values left from earlier runs),
# the DFM (fresh storage) with its OpenSOVD bridge, the VSS bridge and the
# Guardian, then the Evidence Collector, replays the .asc once and waits for
# the verdict (Guardian events and OpenSOVD visibility). Everything except
# zenohd and the Data Broker is stopped afterwards.
# Logs: run/, report: [report.json] or reports/<case>.json. Exit code = collector's
# (0 PASS, 1 FAIL, 2 INCONCLUSIVE, 3 error).
set -euo pipefail
trap 'exit 3' ERR   # a failing build/start step is an error, not a FAIL verdict

if [ $# -lt 1 ] || [ $# -gt 2 ]; then
  echo "usage: $0 <prefix> [report.json]   (e.g. product/config/signal_out_of_range)" >&2
  exit 3
fi

TOOLS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$TOOLS/.." && pwd)"
PREFIX="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
RUN_DIR="$ROOT/run"
REPORT_ARGS=()
[ $# -eq 2 ] && REPORT_ARGS=(--report "$2")
COLLECTOR="$ROOT/product/components/evidence_collector/target/debug/evidence_collector"
export ZENOH_CONNECT="${ZENOH_CONNECT:-tcp/127.0.0.1:7447}"
E2E_CASE_TIMEOUT_S="${E2E_CASE_TIMEOUT_S:-240}"   # per-case collector deadline

[ -f "$PREFIX.asc" ] || { echo "run_case: $PREFIX.asc not found" >&2; exit 3; }
[ -f "$PREFIX.ground_truth.yaml" ] || [ -f "$PREFIX.json" ] \
  || { echo "run_case: no $PREFIX.ground_truth.yaml or $PREFIX.json" >&2; exit 3; }

cd "$ROOT"
mkdir -p "$RUN_DIR"

echo "== building (if needed)"
[ -x product/components/guardien/target/debug/guardian ] || bash "$TOOLS/build_guardian.sh"
[ -x product/components/vss_bridge/target/debug/vss_publisher ] || bash "$TOOLS/build_vss_bridge.sh"
[ -x product/components/fault-lib/target/debug/dfm_bin ] || bash "$TOOLS/build_dfm.sh"
[ -x product/components/dfm_sovd_bridge/target/debug/dfm_sovd_bridge ] || bash "$TOOLS/build_dfm_sovd_bridge.sh"
(cd product/components/evidence_collector && cargo build -q)   # always: cheap when up to date

source "$TOOLS/ensure_python_env.sh"
ensure_python_env || { echo "run_case: replay python environment incomplete (E2E_VENV=${E2E_VENV:-$HOME/.venv}, needs cantools/python-can/pyyaml)" >&2; exit 3; }

cleanup() {
  echo "== stopping Guardian, VSS bridge, OpenSOVD bridge and DFM"
  bash "$TOOLS/stop_guardian.sh" >/dev/null 2>&1 || true
  bash "$TOOLS/stop_vss_bridge.sh" >/dev/null 2>&1 || true
  bash "$TOOLS/stop_dfm_sovd_bridge.sh" >/dev/null 2>&1 || true
  bash "$TOOLS/stop_dfm.sh" >/dev/null 2>&1 || true
}
trap cleanup EXIT
cleanup   # leftovers from an earlier run

echo "== starting services"
bash "$TOOLS/start_zenohd.sh"
for _i in $(seq 1 30); do (exec 3<>/dev/tcp/127.0.0.1/7447) 2>/dev/null && break; sleep 0.5; done
(exec 3<>/dev/tcp/127.0.0.1/7447) 2>/dev/null \
  || { echo "run_case: zenohd never opened port 7447 (see ${ZENOH_LOG:-/tmp/zenohd.log})" >&2; exit 3; }
pkill -x databroker 2>/dev/null && sleep 1 || true
bash "$TOOLS/start_databroker.sh"
for _i in $(seq 1 30); do (exec 3<>/dev/tcp/127.0.0.1/55555) 2>/dev/null && break; sleep 0.5; done
(exec 3<>/dev/tcp/127.0.0.1/55555) 2>/dev/null \
  || { echo "run_case: databroker never opened port 55555 (see ${DATABROKER_LOG:-/tmp/databroker.log})" >&2; exit 3; }
sleep 1
bash "$TOOLS/run_dfm.sh"                        # fresh DFM storage per run
sleep 2
bash "$TOOLS/run_dfm_sovd_bridge.sh"
bash "$TOOLS/run_vss_bridge.sh" >"$RUN_DIR/vss_bridge.log" 2>&1
# Liveness: the bridge exits when the broker connection fails.
for _i in $(seq 1 10); do
  pgrep -x vss_publisher >/dev/null 2>&1 && break
  sleep 0.5
done
pgrep -x vss_publisher >/dev/null 2>&1 \
  || { echo "run_case: vss_publisher did not start (see $RUN_DIR/vss_bridge.log)" >&2; exit 3; }
bash "$TOOLS/run_guardian.sh"

echo "== collector listening, replaying $(basename "$PREFIX").asc (takes as long as the recording)"
"$COLLECTOR" "$PREFIX" "${REPORT_ARGS[@]}" >"$RUN_DIR/collector.out" 2>"$RUN_DIR/collector.log" &
collector_pid=$!
# Readiness: the collector prints "replay lasts ... ms" once it is subscribed;
# replaying before that races the subscription and loses events.
_subscribed=0
for _i in $(seq 1 40); do
  grep -q "replay lasts" "$RUN_DIR/collector.log" 2>/dev/null && { _subscribed=1; break; }
  kill -0 "$collector_pid" 2>/dev/null || break
  sleep 0.5
done
if [ "$_subscribed" != 1 ]; then
  echo "run_case: collector never became ready (see $RUN_DIR/collector.log)" >&2
  kill "$collector_pid" 2>/dev/null || true
  exit 3
fi
bash "$TOOLS/start_can.sh" "$PREFIX.asc" >"$RUN_DIR/can.log" 2>&1 \
  || { echo "run_case: replay failed, see $RUN_DIR/can.log" >&2; kill "$collector_pid" 2>/dev/null; exit 3; }

trap - ERR   # from here on the exit code is the collector's verdict
set +e
_waited=0 _timed_out=0
while kill -0 "$collector_pid" 2>/dev/null; do
  if [ "$_waited" -ge "$E2E_CASE_TIMEOUT_S" ]; then
    echo "run_case: collector did not finish within ${E2E_CASE_TIMEOUT_S}s; killing (fail-closed)" >&2
    _timed_out=1
    kill "$collector_pid" 2>/dev/null || true
    break
  fi
  sleep 1; _waited=$((_waited + 1))
done
wait "$collector_pid" 2>/dev/null
verdict=$?
set -e
[ "$_timed_out" = 0 ] || { echo "run_case: TIMEOUT treated as INCONCLUSIVE" >&2; exit 2; }

echo "== result"
cat "$RUN_DIR/collector.out"
echo "   logs: $RUN_DIR/{collector,vss_bridge,guardian,can,dfm,dfm_sovd_bridge}.log"
exit "$verdict"
