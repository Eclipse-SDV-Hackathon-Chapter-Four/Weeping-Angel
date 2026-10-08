#!/usr/bin/bash
# Copyright (c) 2026 Michael Warmuth-Uhl
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
# Assisted-by: Claude Opus 5.5

fail() { 
    echo "$*" >&2
    exit 1
}

wait_http() {   # wait_http <url> <label> [attempts]
  local url="$1" label="$2" attempts="${3:-10}" code
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

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
RUN_DIR="$ROOT_DIR/run/"
DFM_BIN="$ROOT_DIR/product/components/fault-lib/target/debug/dfm_bin"
DFM_STORAGE="$RUN_DIR/dfm-storage"
CATALOG_DIR="$ROOT_DIR/product/config/battery_guardian"

mkdir -p "$RUN_DIR"

if [ -x $DFM_BIN ] ; then
    rm -rf "$DFM_STORAGE"
    mkdir -p "$DFM_STORAGE"
    # From product/: iceoryx2 reads config/iceoryx2.toml there (subscriber buffer)
    (cd "$ROOT_DIR/product" && exec "$DFM_BIN" --catalog-dir "$CATALOG_DIR" --storage-dir "$DFM_STORAGE") >"$RUN_DIR/dfm.log" 2>&1 &
else
    fail "DFM binary $DFM_BIN not found, did you build it?" 
fi

echo "DFM up and running"
