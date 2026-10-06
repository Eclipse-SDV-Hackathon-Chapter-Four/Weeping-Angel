#!/usr/bin/env bash
#
# run_demo.sh — build and launch the full Battery Thermal Guardian safety
# evidence stack, run the Robot Framework suite, and tear everything down.
#
#   fault_injector --(uProtocol/Zenoh)--> Guardian --(iceoryx2 IPC)--> DFM
#       --> OpenSOVD gateway --(SOVD HTTP)--> Robot suite --> Markdown report
#
# Usage:
#   scripts/run_demo.sh            # build (if needed), run stack + tests
#   scripts/run_demo.sh --no-build # skip cargo builds, just run
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEMO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$DEMO_DIR"

RUN_DIR="${RUN_DIR:-/tmp/dr-whodunit}"
mkdir -p "$RUN_DIR"

CATALOG_DIR="$DEMO_DIR/diagnostics/catalog"
DFM_BIN="$DEMO_DIR/fault-lib/target/debug/dfm_bin"
GUARDIAN_BIN="$DEMO_DIR/target/debug/guardian"
GATEWAY_BIN="$DEMO_DIR/opensovd-core/target/debug/opensovd-gateway"
TOXI_BIN="$DEMO_DIR/.tools/toxiproxy-server"

ZENOH_LISTEN="tcp/127.0.0.1:7447"
GATEWAY_URL="http://127.0.0.1:7690"
TOXI_API="http://127.0.0.1:8474"

PIDS=()

log()  { printf '\033[1;34m[run]\033[0m %s\n' "$*"; }
fail() { printf '\033[1;31m[err]\033[0m %s\n' "$*" >&2; exit 1; }

cleanup() {
  log "shutting down stack ..."
  # Remove any lingering transport toxic.
  curl -s -X DELETE "$TOXI_API/proxies/zenoh/toxics/zenoh_blackhole" >/dev/null 2>&1 || true
  for pid in "${PIDS[@]:-}"; do
    if [[ -n "$pid" ]] && kill -0 "$pid" >/dev/null 2>&1; then
      kill "$pid" >/dev/null 2>&1 || true
    fi
  done
  sleep 1
  for pid in "${PIDS[@]:-}"; do
    if [[ -n "$pid" ]] && kill -0 "$pid" >/dev/null 2>&1; then
      kill -9 "$pid" >/dev/null 2>&1 || true
    fi
  done
}
trap cleanup EXIT

wait_http() {   # wait_http <url> <label> [attempts]
  local url="$1" label="$2" attempts="${3:-60}" code
  for ((i = 0; i < attempts; i++)); do
    code="$(curl -s -o /dev/null -w '%{http_code}' "$url" || true)"
    if [[ "$code" == "200" ]]; then
      log "$label is up"
      return 0
    fi
    sleep 0.5
  done
  fail "$label did not become ready ($url, last HTTP $code)"
}

# ---------------------------------------------------------------- build
if [[ "${1:-}" != "--no-build" ]]; then
  log "building DFM (fault-lib) ..."
  (cd "$DEMO_DIR/fault-lib" && cargo build --bin dfm_bin)
  log "building Guardian + fault_injector ..."
  cargo build -p dr-whodunit-services --bin guardian --bin fault_injector
  log "building OpenSOVD gateway (fault-lib feature) ..."
  (cd "$DEMO_DIR/opensovd-core" && cargo build -p opensovd-gateway --features fault-lib)
fi

[[ -x "$DFM_BIN" ]]      || fail "missing $DFM_BIN (run without --no-build)"
[[ -x "$GUARDIAN_BIN" ]] || fail "missing $GUARDIAN_BIN"
[[ -x "$GATEWAY_BIN" ]]  || fail "missing $GATEWAY_BIN"
[[ -x "$TOXI_BIN" ]]     || fail "missing $TOXI_BIN"

# ---------------------------------------------------------------- launch
log "starting DFM ..."
DFM_STORAGE="$RUN_DIR/dfm-storage"
rm -rf "$DFM_STORAGE"
mkdir -p "$DFM_STORAGE"
"$DFM_BIN" --catalog-dir "$CATALOG_DIR" --storage-dir "$DFM_STORAGE" >"$RUN_DIR/dfm.log" 2>&1 &
PIDS+=($!)
sleep 2

log "starting Guardian ..."
FAULT_CATALOG="$CATALOG_DIR/battery_guardian.json" \
SOVD_ENTITY="battery_guardian" \
PORT="8080" \
ZENOH_LISTEN="$ZENOH_LISTEN" \
  "$GUARDIAN_BIN" >"$RUN_DIR/guardian.log" 2>&1 &
PIDS+=($!)
wait_http "http://127.0.0.1:8080/health" "Guardian"

log "starting OpenSOVD gateway ..."
"$GATEWAY_BIN" --dfm-fault-app battery_guardian >"$RUN_DIR/gateway.log" 2>&1 &
PIDS+=($!)
wait_http "$GATEWAY_URL/sovd/v1/apps/battery_guardian/faults" "OpenSOVD gateway"

log "starting Toxiproxy ..."
"$TOXI_BIN" >"$RUN_DIR/toxiproxy.log" 2>&1 &
PIDS+=($!)
wait_http "$TOXI_API/version" "Toxiproxy"

log "creating Zenoh proxy (7448 -> 7447) ..."
curl -s -X POST "$TOXI_API/proxies" \
  -H 'Content-Type: application/json' \
  -d '{"name":"zenoh","listen":"127.0.0.1:7448","upstream":"127.0.0.1:7447","enabled":true}' \
  >/dev/null || true

# ---------------------------------------------------------------- test
log "running Robot Framework suite ..."
cd "$DEMO_DIR/tests"
set +e
python3 -m robot --outputdir "$DEMO_DIR/reports" battery_guardian.robot
RC=$?
set -e

log "evidence report: $DEMO_DIR/reports/evidence_report.md"
log "robot report:    $DEMO_DIR/reports/report.html"
exit $RC
