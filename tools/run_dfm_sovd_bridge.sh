#!/usr/bin/bash

die() { 
    echo "$*" >&2
    exit 1
}

wait_http() {   # wait_http <url> <label> [attempts]
  local url="$1" label="$2" attempts="${3:-30}" code
  for ((i = 0; i < attempts; i++)); do
    code="$(curl -s -o /dev/null -w '%{http_code}' "$url" || true)"
    if [[ "$code" == "200" ]]; then
      echo "$label is up"
      return 0
    fi
    sleep 0.5
  done
  die "$label did not become ready ($url, last HTTP $code)"
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
RUN_DIR="$ROOT_DIR/run/"
BRIDGE_BIN="$ROOT_DIR/product/components/dfm_sovd_bridge/target/debug/dfm_sovd_bridge"

# Environment variables read by the bridge (override by exporting them before calling this script)
DEFAULT_SOVD_URL="http://127.0.0.1:7690/sovd"
export SOVD_URL="${SOVD_URL:-$DEFAULT_SOVD_URL}"                  # same default URL as opensovd-gateway
export DFM_SOVD_PATH="${DFM_SOVD_PATH:-battery_guardian}"          # must match GUARDIAN_SOVD_PATH
export DFM_SOVD_NAME="${DFM_SOVD_NAME:-Battery Guardian}"
export DFM_QUERY_TIMEOUT_MS="${DFM_QUERY_TIMEOUT_MS:-1000}"
export DFM_STARTUP_WAIT_S="${DFM_STARTUP_WAIT_S:-10}"             # wait for the DFM to answer at startup
export RUST_LOG="${RUST_LOG:-dfm_sovd_bridge=info,info}"

if [[ "$SOVD_URL" == "$DEFAULT_SOVD_URL" ]] && pidof opensovd-gateway > /dev/null ; then
    die "opensovd-gateway is running on port 7690, stop it (tools/stop_sovd.sh) or set SOVD_URL"
fi

pidof dfm_bin > /dev/null || echo "Warning: no DFM process found, start it with tools/run_dfm.sh" >&2

mkdir -p "$RUN_DIR"

if [ -x "$BRIDGE_BIN" ] ; then
    "$BRIDGE_BIN" >"$RUN_DIR/dfm_sovd_bridge.log" 2>&1 &
    wait_http "$SOVD_URL/version-info" "DFM SOVD bridge"
else
    die "DFM SOVD bridge binary $BRIDGE_BIN not found, did you build it?"
fi

echo "DFM SOVD bridge up and running on $SOVD_URL"
