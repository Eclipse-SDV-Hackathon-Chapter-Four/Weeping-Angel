#!/usr/bin/bash
# Copyright (c) 2026 Michael Warmuth-Uhl
# Copyright (c) 2026 Matthias Knöfel
#
# This program and the accompanying materials are made available under
# the terms of the Eclipse Public License 2.0 which accompanies this
# distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
#
# AI Disclosure: This file was mostly AI-generated.
#
# SPDX-License-Identifier: EPL-2.0 and CC0-1.0

die() { 
    echo "$*" >&2
    exit 1
}

wait_http() {   # wait_http <url> <label> [attempts]
  local url="$1" label="$2" attempts="${3:-10}" code
  for ((i = 0; i < attempts; i++)); do
    code="$(curl -s -o /dev/null -w '%{http_code}' "$url" || true)"
    if [[ "$code" == "200" || "$code" == "404" ]]; then
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
OPENSOVD_BIN="$ROOT_DIR/product/components/opensovd-core/target/debug/opensovd-gateway"
GATEWAY_URL="http://localhost:7690"

mkdir -p "$RUN_DIR"

if [ -x "$OPENSOVD_BIN" ] ; then
    "$OPENSOVD_BIN" >"$RUN_DIR/opensovd.log" 2>&1 &
    wait_http "$GATEWAY_URL/" "OpenSOVD gateway"
else
    die "OpenSOVD binary $OPENSOVD_BIN not found, did you build it?" 
fi

echo "OpenSOVD Gateway up and running"
