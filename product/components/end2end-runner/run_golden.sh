#!/usr/bin/env bash
# Golden-run orchestration for the product evidence chain:
#
#   case mutator -> CAN replay (start_can.sh) -> kuksa-databroker
#     -> vss_publisher -> zenoh (uProtocol) -> guardian
#     -> dfm_bin (iceoryx2) -> dfm_sovd_bridge (SOVD :7690)
#     -> evidence_collector (verdict per case)
#
# Runs ONE replay per case, each case against a freshly reset
# Guardian/DFM/SOVD stack. Cases are data, not code: register a case in
# `CASES` and provide its artifacts under cases/<name>/ (see README.md).
# Adding further baseline scenarios or harness experiment bundles later
# must not require changing the flow below.
#
# Built for the dev container (zenohd, databroker, python venv present).
# Usage:
#   run_golden.sh                 # run all registered cases
#   run_golden.sh <case> [...]    # run only the named cases
#   E2E_REBUILD=1 run_golden.sh   # force binary rebuild
#   E2E_REGEN_CASES=1 run_golden.sh  # force case regeneration
#   E2E_OBSERVER=1 run_golden.sh     # serve the live observer during each case
set -euo pipefail

COMPONENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PRODUCT_DIR="$(cd "$COMPONENT_DIR/../.." && pwd)"
REPO_DIR="$(cd "$PRODUCT_DIR/.." && pwd)"

# ---------------------------------------------------------------------------
# Configuration (env-overridable)
# ---------------------------------------------------------------------------
ZENOH_CONNECT="${ZENOH_CONNECT:-tcp/127.0.0.1:7447}"
DATABROKER_ADDR="${DATABROKER_ADDR:-http://127.0.0.1:55555}"
GUARDIAN_SOVD_PATH="${GUARDIAN_SOVD_PATH:-battery_guardian}"
E2E_IDLE_TIMEOUT_S="${E2E_IDLE_TIMEOUT_S:-3}"     # collector drain window
E2E_CASE_TIMEOUT_S="${E2E_CASE_TIMEOUT_S:-240}"   # per-case collector deadline
E2E_REBUILD="${E2E_REBUILD:-0}"                   # 1 = force binary rebuild
E2E_REGEN_CASES="${E2E_REGEN_CASES:-0}"           # 1 = regenerate case artifacts
E2E_OBSERVER="${E2E_OBSERVER:-0}"                 # 1 = serve the live observer (ADR-016)
E2E_OBSERVER_ADDR="${E2E_OBSERVER_ADDR:-127.0.0.1:8090}"

TARGET="$PRODUCT_DIR/components"
BIN_GUARDIAN="$TARGET/guardien/target/debug/guardian"
BIN_DFM="$TARGET/fault-lib/target/debug/dfm_bin"
BIN_SOVD_BRIDGE="$TARGET/dfm_sovd_bridge/target/debug/dfm_sovd_bridge"
BIN_VSS_BRIDGE="$TARGET/vss_bridge/target/debug/vss_publisher"
BIN_COLLECTOR="$TARGET/evidence_collector/target/debug/evidence_collector"
BIN_MUTATOR="$TARGET/case_mutator/target/debug/case-mutator"

TOOLS_DIR="$REPO_DIR/tools"   # service start scripts live here (moved out of components/)

# Python environment for the CAN replay (dbcfeeder: python-can, cantools,
# kuksa-client). Self-provisioned when missing, mirroring the devcontainer
# post-create convention ($HOME/.venv).
E2E_VENV="${E2E_VENV:-$HOME/.venv}"

CONFIG_DIR="$PRODUCT_DIR/config/battery_guardian"
NOMINAL_ASC="$PRODUCT_DIR/config/battery_temp_with_ts.asc"
CASES_DIR="$COMPONENT_DIR/cases"

# ---------------------------------------------------------------------------
# Case registry — the single place to add runs.
# Each line: <name>|<collector prefix relative to CASES_DIR>
# The prefix must have <prefix>.asc and <prefix>.ground_truth.yaml|json
# after prepare_case_<name> ran. Baselines are just cases with an empty
# ground truth; future harness experiment bundles slot in the same way.
# ---------------------------------------------------------------------------
CASES=(
  "baseline|baseline/baseline"
  "signal_out_of_range|signal_out_of_range/signal_out_of_range"
)

# ---------------------------------------------------------------------------
# Small helpers
# ---------------------------------------------------------------------------
log()  { printf '\e[1;34m[runner]\e[0m %s\n' "$*" >&2; }
warn() { printf '\e[1;33m[runner]\e[0m %s\n' "$*" >&2; }
die()  { printf '\e[1;31m[runner]\e[0m %s\n' "$*" >&2; exit 1; }

port_open() { (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null; }

wait_port() { # <port> <what> [timeout_s]
  local port="$1" what="$2" timeout="${3:-30}" i
  for ((i = 0; i < timeout * 2; i++)); do
    port_open "$port" && return 0
    sleep 0.5
  done
  die "$what did not open port $port within ${timeout}s (see logs)"
}

wait_http() { # <url> <what> [timeout_s]
  local url="$1" what="$2" timeout="${3:-30}" i
  for ((i = 0; i < timeout * 2; i++)); do
    curl -fsS "$url" >/dev/null 2>&1 && return 0
    sleep 0.5
  done
  die "$what not ready at $url within ${timeout}s"
}

stop_pid() { # <pid> <name>
  local pid="$1" name="$2" i
  [ -n "$pid" ] || return 0
  kill -0 "$pid" 2>/dev/null || return 0
  kill "$pid" 2>/dev/null || true
  for ((i = 0; i < 50; i++)); do
    kill -0 "$pid" 2>/dev/null || return 0
    sleep 0.1
  done
  warn "$name (pid $pid) ignored SIGTERM, sending SIGKILL"
  kill -KILL "$pid" 2>/dev/null || true
}

# ---------------------------------------------------------------------------
# Build (fresh binaries; the evidence run must be reproducible)
# ---------------------------------------------------------------------------
build() {
  local cargo="cargo"
  command -v "$cargo" >/dev/null || die "cargo not found (run inside the dev container)"
  export PROTOC="${PROTOC:-$(command -v protoc || true)}"
  export PROTOC_INCLUDE="${PROTOC_INCLUDE:-/usr/include}"
  if [ ! -x "$BIN_GUARDIAN" ] || [ "$E2E_REBUILD" = 1 ]; then
    log "building guardian"
    (cd "$TARGET/guardien" && make build) >&2
  fi
  if [ ! -x "$BIN_DFM" ] || [ "$E2E_REBUILD" = 1 ]; then
    log "building dfm_bin"
    (cd "$TARGET/fault-lib" && $cargo build --locked -p dfm_bin) >&2
  fi
  if [ ! -x "$BIN_SOVD_BRIDGE" ] || [ "$E2E_REBUILD" = 1 ]; then
    log "building dfm_sovd_bridge"
    (cd "$TARGET/dfm_sovd_bridge" && make build) >&2
  fi
  if [ ! -x "$BIN_VSS_BRIDGE" ] || [ "$E2E_REBUILD" = 1 ]; then
    log "building vss_publisher"
    (cd "$TARGET/vss_bridge" && $cargo build --locked) >&2
  fi
  local collector_features=()
  [ "$E2E_OBSERVER" = 1 ] && collector_features=(--features observer)
  if [ ! -x "$BIN_COLLECTOR" ] || [ "$E2E_REBUILD" = 1 ] || [ "$E2E_OBSERVER" = 1 ]; then
    log "building evidence_collector ${collector_features[*]:-}"
    (cd "$TARGET/evidence_collector" && $cargo build --locked "${collector_features[@]}") >&2
  fi
  if [ ! -x "$BIN_MUTATOR" ] || [ "$E2E_REBUILD" = 1 ]; then
    log "building case-mutator"
    (cd "$TARGET/case_mutator" && make build) >&2
  fi
}

# ---------------------------------------------------------------------------
# Case preparation — one function per case, idempotent. New cases need a
# prepare function only if their artifacts are generated (mutator) rather
# than committed.
# ---------------------------------------------------------------------------
prepare_baseline() {
  local dir="$CASES_DIR/baseline"
  mkdir -p "$dir"
  [ -f "$dir/baseline.asc" ] || cp "$NOMINAL_ASC" "$dir/baseline.asc"
  [ -f "$dir/baseline.ground_truth.yaml" ] || printf '[]\n' > "$dir/baseline.ground_truth.yaml"
}

prepare_signal_out_of_range() {
  local dir="$CASES_DIR/signal_out_of_range"
  if [ "$E2E_REGEN_CASES" = 1 ] || [ ! -f "$dir/signal_out_of_range.asc" ] ||
     { [ ! -f "$dir/signal_out_of_range.ground_truth.yaml" ] &&
       [ ! -f "$dir/signal_out_of_range.ground_truth.json" ]; }; then
    log "generating case signal_out_of_range with the mutator"
    mkdir -p "$dir"
    (cd "$TARGET/case_mutator" && \
      cargo run --locked --quiet -- \
        --request "$TARGET/case_mutator/examples/out_of_range.yaml" \
        --output-dir "$dir") >&2
  fi
}

prepare_case() { # <name>
  case "$1" in
    baseline) prepare_baseline ;;
    signal_out_of_range) prepare_signal_out_of_range ;;
    *) die "no prepare step for case '$1' (add prepare_<name> or commit its artifacts)" ;;
  esac
  # shellcheck disable=SC2178 # assignment, not comparison
  local prefix="$2" missing=0
  [ -f "${prefix}.asc" ] || { warn "case $1: missing ${prefix}.asc"; missing=1; }
  if [ ! -f "${prefix}.ground_truth.yaml" ] && [ ! -f "${prefix}.ground_truth.json" ]; then
    warn "case $1: missing ${prefix}.ground_truth.yaml|json"; missing=1
  fi
  [ "$missing" = 0 ] || die "case '$1' artifacts incomplete"
}

ensure_python_env() {
  local py="$E2E_VENV/bin/python"
  if [ ! -x "$py" ] || ! "$py" -c "import cantools, can" >/dev/null 2>&1; then
    log "setting up the replay python environment in $E2E_VENV"
    if [ ! -x "$py" ]; then
      python3 -m venv "$E2E_VENV" >&2
    fi
    "$E2E_VENV/bin/pip" install --quiet \
      -r "$REPO_DIR/.devcontainer/requirements.txt" \
      -r "$REPO_DIR/product/components/kuksa-can-provider/requirements.in" >&2
  fi
  export PATH="$E2E_VENV/bin:$PATH"
  "$E2E_VENV/bin/python" -c "import cantools, can" 2>/dev/null \
    || die "replay python environment incomplete (see $E2E_VENV, install cantools/python-can/kuksa-client)"
}

# ---------------------------------------------------------------------------
# Infrastructure (started once, shared across cases)
# ---------------------------------------------------------------------------
INFRA_PIDS=()

start_infra() {
  ensure_python_env
  # Infrastructure logs land in the run dir so the dump is self-contained.
  log "starting zenoh router"
  ZENOH_LOG="$RUN_DIR/logs/zenohd.log" "$TOOLS_DIR/start_zenohd.sh" >&2
  wait_port 7447 "zenohd" 15
  log "starting kuksa-databroker"
  DATABROKER_LOG="$RUN_DIR/logs/databroker.log" "$TOOLS_DIR/start_databroker.sh" >&2
  wait_port 55555 "kuksa-databroker" 15

  log "starting vss_publisher (databroker -> uProtocol)"
  DATABROKER_ADDR="$DATABROKER_ADDR" ZENOH_CONNECT="$ZENOH_CONNECT" \
    "$BIN_VSS_BRIDGE" >"$RUN_DIR/logs/vss_publisher.log" 2>&1 &
  INFRA_PIDS+=($!)
  # The bridge exits when the broker connection fails; verify it is alive.
  sleep 1
  kill -0 "${INFRA_PIDS[-1]}" 2>/dev/null || die "vss_publisher exited at startup (see logs/vss_publisher.log)"
}

ensure_vss_bridge_alive() {
  local pid="${INFRA_PIDS[-1]:-}"
  if [ -z "$pid" ] || ! kill -0 "$pid" 2>/dev/null; then
    warn "vss_publisher died between cases; restarting"
    DATABROKER_ADDR="$DATABROKER_ADDR" ZENOH_CONNECT="$ZENOH_CONNECT" \
      "$BIN_VSS_BRIDGE" >>"$RUN_DIR/logs/vss_publisher.log" 2>&1 &
    INFRA_PIDS+=($!)
    sleep 1
    kill -0 "${INFRA_PIDS[-1]}" 2>/dev/null || die "vss_publisher restart failed"
  fi
}

# ---------------------------------------------------------------------------
# Per-case stack: guardian + DFM + SOVD bridge, reset for every case so
# each verdict starts from a clean diagnostic state.
# ---------------------------------------------------------------------------
CASE_PIDS=()

stop_case_stack() {
  local pid
  for pid in "${CASE_PIDS[@]:-}"; do
    stop_pid "$pid" "case-stack process"
  done
  CASE_PIDS=()
}

start_case_stack() { # <case_dir>
  local case_dir="$1" storage="$case_dir/dfm_storage"

  # Fail fast on ports held by processes we do not own.
  port_open 7690 && die "port 7690 already in use (stop the previous SOVD bridge first)"
  port_open 8080 && die "port 8080 already in use (stop the previous guardian first)"

  rm -rf "$storage"
  log "  starting dfm_bin (storage: $storage)"
  "$BIN_DFM" --catalog-dir "$CONFIG_DIR" --storage-dir "$storage" \
    >"$case_dir/dfm_bin.log" 2>&1 &
  CASE_PIDS+=($!)

  log "  starting dfm_sovd_bridge on :7690"
  DFM_SOVD_PATH="$GUARDIAN_SOVD_PATH" \
    "$BIN_SOVD_BRIDGE" >"$case_dir/dfm_sovd_bridge.log" 2>&1 &
  CASE_PIDS+=($!)

  log "  starting guardian on :8080"
  GUARDIAN_CONFIG="$CONFIG_DIR/guardian_model.yaml" \
  GUARDIAN_FAULT_CATALOG="$CONFIG_DIR/guardian_diagnostics.json" \
  ZENOH_CONNECT="$ZENOH_CONNECT" HOST=127.0.0.1 PORT=8080 \
    "$BIN_GUARDIAN" >"$case_dir/guardian.log" 2>&1 &
  CASE_PIDS+=($!)

  wait_port 7690 "dfm_sovd_bridge" 30
  # Readiness = SOVD actually lists the catalog faults (DFM answered).
  wait_http "http://127.0.0.1:7690/sovd/v1/components/$GUARDIAN_SOVD_PATH/data/faults" \
    "SOVD faults view" 30
  wait_http "http://127.0.0.1:8080/health" "guardian" 30
}

# ---------------------------------------------------------------------------
# One case = reset stack -> collect -> replay -> drain -> verdict
# ---------------------------------------------------------------------------
run_case() { # <name> <prefix_abs> <case_dir>
  local name="$1" prefix="$2" case_dir="$3"
  log "case $name: preparing"
  mkdir -p "$case_dir"

  prepare_case "$name" "$prefix"
  ensure_vss_bridge_alive
  start_case_stack "$case_dir"

  local -a observer_args=()
  if [ "$E2E_OBSERVER" = 1 ]; then
    observer_args=(--observer --observer-addr "$E2E_OBSERVER_ADDR")
  fi
  log "case $name: starting evidence collector"
  ZENOH_CONNECT="$ZENOH_CONNECT" \
    "$BIN_COLLECTOR" "$prefix" \
      --idle-timeout "$E2E_IDLE_TIMEOUT_S" \
      --report "$case_dir/report.md" \
      "${observer_args[@]}" \
      >"$case_dir/collector.out" 2>"$case_dir/collector.log" &
  local col_pid=$!

  if [ "$E2E_OBSERVER" = 1 ]; then
    local w
    for ((w = 0; w < 40; w++)); do
      curl -fsS "http://$E2E_OBSERVER_ADDR/health" >/dev/null 2>&1 && break
      sleep 0.25
    done
    log "case $name: live observer on http://$E2E_OBSERVER_ADDR (until this case's collector exits)"
  fi

  # The collector prints "replay lasts ... ms" once it is subscribed.
  local i subscribed=0
  for ((i = 0; i < 40; i++)); do
    if grep -q "replay lasts" "$case_dir/collector.log" 2>/dev/null; then
      subscribed=1; break
    fi
    kill -0 "$col_pid" 2>/dev/null || break
    sleep 0.5
  done
  [ "$subscribed" = 1 ] || die "collector $name never became ready (see $case_dir/collector.log)"

  log "case $name: replaying ${prefix}.asc"
  "$TOOLS_DIR/start_can.sh" "${prefix}.asc" >"$case_dir/replay.log" 2>&1 \
    || warn "case $name: replay reported an error (see $case_dir/replay.log)"

  log "case $name: waiting for collector verdict (drain ${E2E_IDLE_TIMEOUT_S}s)"
  local waited=0 verdict="" timed_out=0
  while kill -0 "$col_pid" 2>/dev/null; do
    if [ "$waited" -ge "$E2E_CASE_TIMEOUT_S" ]; then
      warn "case $name: collector did not finish within ${E2E_CASE_TIMEOUT_S}s; killing"
      timed_out=1
      kill "$col_pid" 2>/dev/null || true
      break
    fi
    sleep 1; waited=$((waited + 1))
  done
  local rc=0
  wait "$col_pid" 2>/dev/null || rc=$?
  if [ "$timed_out" = 1 ]; then
    verdict="TIMEOUT"
  else
    case "$rc" in
      0) verdict="PASS" ;;
      1) verdict="FAIL" ;;
      2) verdict="INCONCLUSIVE" ;;
      *) verdict="ERROR($rc)" ;;
    esac
  fi
  grep -m1 "^[^ ]*: " "$case_dir/collector.out" >"$case_dir/verdict.txt" 2>/dev/null || \
    printf '%s: %s\n' "$name" "$verdict" > "$case_dir/verdict.txt"

  stop_case_stack
  log "case $name: verdict $verdict"
  printf '%s\n' "$verdict"
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
RUN_DIR="$COMPONENT_DIR/runs/golden-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$RUN_DIR/logs"

declare -A RESULTS=()

cleanup() {
  stop_case_stack
  local pid
  for pid in "${INFRA_PIDS[@]:-}"; do
    stop_pid "$pid" "infra process"
  done
}
trap cleanup EXIT

# Parse args: optional case-name filter.
WANT=("$@")
if [ "${#WANT[@]}" -gt 0 ]; then
  for want in "${WANT[@]}"; do
    local_found=0
    for entry in "${CASES[@]}"; do
      [ "${entry%%|*}" = "$want" ] && local_found=1
    done
    [ "$local_found" = 1 ] || die "unknown case '$want' (registered: ${CASES[*]%%|*})"
  done
fi

log "run dir: $RUN_DIR"
log "repo: $REPO_DIR ($(git -C "$REPO_DIR" rev-parse --short HEAD 2>/dev/null || echo 'no git'))"
build
start_infra

for entry in "${CASES[@]}"; do
  name="${entry%%|*}"
  rel_prefix="${entry#*|}"
  if [ "${#WANT[@]}" -gt 0 ]; then
    wanted=0
    for want in "${WANT[@]}"; do [ "$want" = "$name" ] && wanted=1; done
    [ "$wanted" = 1 ] || continue
  fi
  prefix="$CASES_DIR/$rel_prefix"
  case_dir="$RUN_DIR/$name"
  verdict="$(run_case "$name" "$prefix" "$case_dir")"
  RESULTS["$name"]="$verdict"
done

log "================ summary ================"
overall=0
for entry in "${CASES[@]}"; do
  name="${entry%%|*}"
  [ -n "${RESULTS[$name]:-}" ] || continue
  verdict="${RESULTS[$name]}"
  log "  $name: $verdict"
  case "$verdict" in
    PASS) ;;
    FAIL|INCONCLUSIVE|TIMEOUT|ERROR*) overall=1 ;;
  esac
done
[ "$overall" = 0 ] && log "GOLDEN RUN: PASS" || log "GOLDEN RUN: NOT PASSING (see $RUN_DIR)"
exit "$overall"
