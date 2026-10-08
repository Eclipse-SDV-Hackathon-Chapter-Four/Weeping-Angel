#!/usr/bin/env bash
# Generate the battery campaign and run every experiment end to end.
#
#   tools/run_campaign.sh [--campaign ID] [--scenario ID]
#                             e.g. tools/run_campaign.sh --campaign signal.spike
#
# 1. The campaign harness generates all experiments (or the selected ones)
#    into a fresh folder reports/campaign-<timestamp>/experiments/.
# 2. Every experiment with a case.asc runs once via run_case.sh. Experiments
#    the harness cannot construct (unsatisfiable.yaml, no replay) are not
#    tests and are left out.
# 3. Per experiment, the report and logs go to
#    reports/campaign-<timestamp>/<campaign>--<scenario>/, and a verdict
#    summary to reports/campaign-<timestamp>/summary.txt.
# Exit code: 0 if every run experiment passed, 1 otherwise, 3 if build or
# generation failed.
# Env: E2E_VENV (python env for harness + replay, self-provisioned),
#      E2E_CASE_TIMEOUT_S (per-experiment collector deadline, default 240),
#      E2E_OBSERVER (1 = serve the live observer during each case, default 1),
#      E2E_OBSERVER_ADDR (observer bind address, default 0.0.0.0:8090).
set -uo pipefail

TOOLS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$TOOLS/.." && pwd)"
OUT="$ROOT/reports/campaign-$(date +%Y%m%d-%H%M%S)"
EXPERIMENTS="$OUT/experiments"
SUMMARY="$OUT/summary.txt"

mkdir -p "$OUT" "$OUT/logs"
cd "$ROOT"

# Live Scenario Observer (ADR-016): the campaign serves it for every case by
# default; set E2E_OBSERVER=0 to run without it. Read by run_case.sh.
E2E_OBSERVER="${E2E_OBSERVER:-1}"
E2E_OBSERVER_ADDR="${E2E_OBSERVER_ADDR:-0.0.0.0:8090}"
export E2E_OBSERVER E2E_OBSERVER_ADDR

# Shared self-provisioning: the harness needs pyyaml, the replay needs
# cantools/python-can — one venv covers both (same convention as run_golden).
source "$TOOLS/ensure_python_env.sh"
ensure_python_env \
  || { echo "run_campaign: python environment incomplete (E2E_VENV=${E2E_VENV:-$HOME/.venv})" >&2; exit 3; }

# Always build (incremental): a binary older than the pulled config fails to start.
echo "== building components"
for b in guardian vss_bridge dfm dfm_sovd_bridge; do
  bash "$TOOLS/build_$b.sh" || { echo "run_campaign: build_$b.sh failed" >&2; exit 3; }
done
collector_features=()
[ "$E2E_OBSERVER" = 1 ] && collector_features=(--features observer)
(cd product/components/evidence_collector && cargo build "${collector_features[@]}") \
  || { echo "run_campaign: evidence collector build failed" >&2; exit 3; }
# The mutator and its oracle binary feed generation directly; a stale
# cross-toolchain binary in the shared target/ (host nix vs container) would
# silently encode pre-change designs, so build both here like the collector.
(cd product/components/case_mutator && cargo build -q --bins) \
  || { echo "run_campaign: case mutator build failed" >&2; exit 3; }

echo "== generating experiments into $EXPERIMENTS"
python3 product/components/battery_campaign_harness/harness.py generate \
  --output-dir "$EXPERIMENTS" "$@" \
  || { echo "run_campaign: generation failed" >&2; exit 3; }

# Infra logs belong in the campaign dump, not /tmp (zenohd and databroker are
# only started by run_case.sh if not already running; first start truncates).
export ZENOH_LOG="$OUT/logs/zenohd.log"
export DATABROKER_LOG="$OUT/logs/databroker.log"

verdict_name() {
  case "$1" in
    0) echo PASS ;;
    1) echo FAIL ;;
    2) echo INCONCLUSIVE ;;
    *) echo ERROR ;;
  esac
}

declare -A counts=()
all_passed=true
: >"$SUMMARY"

for dir in "$EXPERIMENTS"/*/*/; do
  dir="${dir%/}"
  scenario="$(basename "$dir")"
  campaign="$(basename "$(dirname "$dir")")"
  id="$campaign--$scenario"
  case_dir="$OUT/$id"

  # Unsatisfiable: the harness kept no replay, so there is nothing to run.
  [ -f "$dir/case.asc" ] || continue
  echo
  echo "################ $id"
  mkdir -p "$case_dir"
  bash "$TOOLS/run_case.sh" "$dir/case" "$case_dir/report.json"
  verdict="$(verdict_name $?)"
  cp "$ROOT"/run/*.log "$ROOT/run/collector.out" "$case_dir/" 2>/dev/null

  [ "$verdict" = PASS ] || all_passed=false
  counts[$verdict]=$(( ${counts[$verdict]:-0} + 1 ))
  printf '%-13s %s\n' "$verdict" "$id" | tee -a "$SUMMARY"
done

{
  echo
  for v in PASS FAIL INCONCLUSIVE ERROR; do
    [ -n "${counts[$v]:-}" ] && printf '%s: %s  ' "$v" "${counts[$v]}"
  done
  echo
} >>"$SUMMARY"

echo
# Evidence Reporter (ADR-018): aggregate campaign report, links the per-run ones.
( cd "$ROOT" && python3 product/components/evidence_reporter/source/evidence_reporter.py campaign "$OUT" ) \
  || echo "run_campaign: evidence reporter failed (non-fatal)" >&2

echo "== campaign summary ($OUT)"
cat "$SUMMARY"
$all_passed
