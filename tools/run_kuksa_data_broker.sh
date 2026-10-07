#!/usr/bin/bash

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
