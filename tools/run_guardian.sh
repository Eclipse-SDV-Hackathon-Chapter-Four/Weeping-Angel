#!/usr/bin/bash
# Copyright (c) 2026 Matthias Knöfel
# Copyright (c) 2026 Peter Ulbrich
# Copyright (c) 2026 Sebastian Russer
#
# This program and the accompanying materials are made available under
# the terms of the Eclipse Public License 2.0 which accompanies this
# distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
#
# AI Disclosure: This file was mostly AI-generated.
#
# SPDX-License-Identifier: EPL-2.0 and CC0-1.0
# Assisted-by: Claude Sonnet 5.5, Claude Opus 5.5

die() { 
    echo "$*" >&2
    exit 1
}

wait_http() {   # wait_http <url> <label> [attempts]
  local url="$1" label="$2" attempts="${3:-10}" code
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
GUARDIAN_BIN="$ROOT_DIR/product/components/guardien/target/debug/guardian"

# Environment variables read by the Guardian (override by exporting them before calling this script)
export ZENOH_CONNECT="${ZENOH_CONNECT:-tcp/127.0.0.1:7447}"   # Zenoh router endpoint (unset/empty = peer discovery)
[ -n "$ZENOH_LISTEN" ] && export ZENOH_LISTEN                  # optional Zenoh listen endpoint
export GUARDIAN_CONFIG="${GUARDIAN_CONFIG:-$ROOT_DIR/product/config/battery_guardian/guardian_model.yaml}"
export GUARDIAN_FAULT_CATALOG="${GUARDIAN_FAULT_CATALOG:-$ROOT_DIR/product/config/battery_guardian/guardian_diagnostics.json}"
export GUARDIAN_SOVD_PATH="${GUARDIAN_SOVD_PATH:-battery_guardian}"
export HOST="${HOST:-0.0.0.0}"
export PORT="${PORT:-8080}"
export RUST_LOG="${RUST_LOG:-guardian=info,battery_guardian=info,info}"

[ -z "$ZENOH_CONNECT" ] && unset ZENOH_CONNECT

mkdir -p "$RUN_DIR"

if [ -x "$GUARDIAN_BIN" ] ; then
    # From product/: iceoryx2 reads config/iceoryx2.toml there (subscriber buffer)
    (cd "$ROOT_DIR/product" && exec "$GUARDIAN_BIN") >"$RUN_DIR/guardian.log" 2>&1 &
    wait_http "http://127.0.0.1:$PORT/health" "Guardian"
else
    die "Guardian binary $GUARDIAN_BIN not found, did you build it?"
fi

echo "Guardian up and running"
