#!/usr/bin/bash
# Copyright (c) 2026 Michael Warmuth-Uhl
#
# This program and the accompanying materials are made available under
# the terms of the Eclipse Public License 2.0 which accompanies this
# distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
#
# AI Disclosure: This file was mostly AI-generated.
#
# SPDX-License-Identifier: EPL-2.0 and CC0-1.0

fail() { 
    echo "$*" >&2
    exit 1
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
RUN_DIR="$ROOT_DIR/run/"
VSS_BIN="$ROOT_DIR/product/components/vss_bridge/target/debug/vss_publisher"

mkdir -p "$RUN_DIR"

if [ -x $VSS_BIN ] ; then
    DATABROKER_ADDR="http://127.0.0.1:55555" bash -c "$VSS_BIN" 2>&1 &
else
    fail "VSS bridge binary $VSS_BIN not found, did you build it?" 
fi

echo "VSS bridge up and running"
