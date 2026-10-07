#!/usr/bin/bash

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
