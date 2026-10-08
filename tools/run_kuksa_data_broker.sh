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
CONF_FILE="${ROOT_DIR}/product/config/vss_dbc.json"
BROKER_BIN="/usr/local/bin/databroker"

mkdir -p "$RUN_DIR"

if [ -x $BROKER_BIN ] ; then
    "$BROKER_BIN" --vss "$CONF_FILE" 2>&1 &
else
    fail "Kuksa data broker bin $BROKER_BIN not found." 
fi

echo "KUKSA broker up and running"
