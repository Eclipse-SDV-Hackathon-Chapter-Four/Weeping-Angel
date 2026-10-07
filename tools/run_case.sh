#!/usr/bin/env bash
# Run one test case end to end and print the Evidence Collector verdict.
#
#   tools/run_case.sh <prefix>      e.g. tools/run_case.sh product/config/signal_out_of_range
#
# Needs <prefix>.asc and <prefix>.ground_truth.yaml (or <prefix>.json).
# Starts zenohd and a fresh Data Broker (no values left from earlier runs),
# the VSS bridge and the Guardian, then the Evidence Collector, replays the
# .asc once and waits for the verdict. Guardian and bridge are stopped
# afterwards; zenohd and the Data Broker keep running.
# Logs: run/, report: reports/<case>.json. Exit code = collector's
# (0 PASS, 1 FAIL, 2 INCONCLUSIVE, 3 error).
set -euo pipefail

if [ $# -ne 1 ]; then
  echo "usage: $0 <prefix>   (e.g. product/config/signal_out_of_range)" >&2
  exit 3
fi

TOOLS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$TOOLS/.." && pwd)"
PREFIX="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
RUN_DIR="$ROOT/run"
COLLECTOR="$ROOT/product/components/evidence_collector/target/debug/evidence_collector"
export ZENOH_CONNECT="tcp/127.0.0.1:7447"

[ -f "$PREFIX.asc" ] || { echo "run_case: $PREFIX.asc not found" >&2; exit 3; }
[ -f "$PREFIX.ground_truth.yaml" ] || [ -f "$PREFIX.json" ] \
  || { echo "run_case: no $PREFIX.ground_truth.yaml or $PREFIX.json" >&2; exit 3; }

cd "$ROOT"
mkdir -p "$RUN_DIR"

echo "== building (if needed)"
[ -x product/components/guardien/target/debug/guardian ] || bash "$TOOLS/build_guardian.sh"
[ -x product/components/vss_bridge/target/debug/vss_publisher ] || bash "$TOOLS/build_vss_bridge.sh"
[ -x "$COLLECTOR" ] || (cd product/components/evidence_collector && cargo build)

cleanup() {
  echo "== stopping Guardian and VSS bridge"
  bash "$TOOLS/stop_guardian.sh" >/dev/null 2>&1 || true
  bash "$TOOLS/stop_vss_bridge.sh" >/dev/null 2>&1 || true
}
trap cleanup EXIT
cleanup   # leftovers from an earlier run

echo "== starting services"
bash "$TOOLS/start_zenohd.sh"
pkill -x databroker 2>/dev/null && sleep 1 || true
bash "$TOOLS/start_databroker.sh"
sleep 1
bash "$TOOLS/run_vss_bridge.sh" >"$RUN_DIR/vss_bridge.log" 2>&1
bash "$TOOLS/run_guardian.sh"

echo "== collector listening, replaying $(basename "$PREFIX").asc (takes as long as the recording)"
"$COLLECTOR" "$PREFIX" >"$RUN_DIR/collector.out" 2>"$RUN_DIR/collector.log" &
collector_pid=$!
sleep 1
bash "$TOOLS/start_can.sh" "$PREFIX.asc" >"$RUN_DIR/can.log" 2>&1 \
  || { echo "run_case: replay failed, see $RUN_DIR/can.log" >&2; kill "$collector_pid" 2>/dev/null; exit 3; }

set +e
wait "$collector_pid"
verdict=$?
set -e

echo "== result"
cat "$RUN_DIR/collector.out"
echo "   logs: $RUN_DIR/{collector,vss_bridge,guardian,can}.log"
exit "$verdict"
