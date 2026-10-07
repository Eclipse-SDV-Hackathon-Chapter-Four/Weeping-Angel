#!/usr/bin/env bash
# Generate the battery campaign and run every experiment end to end.
#
#   tools/run_campaign.sh [--campaign ID] [--scenario ID]
#                             e.g. tools/run_campaign.sh --campaign signal.spike
#
# 1. The campaign harness generates all experiments (or the selected ones)
#    into a fresh folder reports/campaign-<timestamp>/experiments/.
# 2. Every experiment with a case.asc runs once via run_case.sh; experiments
#    the harness marked unsatisfiable are skipped.
# 3. Per experiment, the report and logs go to
#    reports/campaign-<timestamp>/<campaign>--<scenario>/, and a verdict
#    summary to reports/campaign-<timestamp>/summary.txt.
# Exit code: 0 if every experiment passed, 1 otherwise, 3 if build or generation failed.
# Env: E2E_VENV (python env for harness + replay, self-provisioned),
#      E2E_CASE_TIMEOUT_S (per-experiment collector deadline, default 240).
set -uo pipefail

TOOLS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$TOOLS/.." && pwd)"
OUT="$ROOT/reports/campaign-$(date +%Y%m%d-%H%M%S)"
EXPERIMENTS="$OUT/experiments"
SUMMARY="$OUT/summary.txt"

mkdir -p "$OUT" "$OUT/logs"
cd "$ROOT"

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
(cd product/components/evidence_collector && cargo build) \
  || { echo "run_campaign: evidence collector build failed" >&2; exit 3; }

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

  if [ ! -f "$dir/case.asc" ]; then
    verdict=SKIPPED   # unsatisfiable: the harness kept no replay
  else
    echo
    echo "################ $id"
    mkdir -p "$case_dir"
    bash "$TOOLS/run_case.sh" "$dir/case" "$case_dir/report.json"
    verdict="$(verdict_name $?)"
    cp "$ROOT"/run/*.log "$ROOT/run/collector.out" "$case_dir/" 2>/dev/null
  fi

  [ "$verdict" = PASS ] || all_passed=false
  counts[$verdict]=$(( ${counts[$verdict]:-0} + 1 ))
  printf '%-13s %s\n' "$verdict" "$id" | tee -a "$SUMMARY"
done

{
  echo
  for v in PASS FAIL INCONCLUSIVE ERROR SKIPPED; do
    [ -n "${counts[$v]:-}" ] && printf '%s: %s  ' "$v" "${counts[$v]}"
  done
  echo
} >>"$SUMMARY"

echo
echo "== campaign summary ($OUT)"
cat "$SUMMARY"
$all_passed
