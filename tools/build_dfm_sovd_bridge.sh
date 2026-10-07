#!/usr/bin/bash

die() { 
    echo "$*" >&2
    exit 1
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$ROOT_DIR/product/components/dfm_sovd_bridge" || die "Missing dfm_sovd_bridge directory"
cargo build --bin dfm_sovd_bridge || die "Error building DFM SOVD bridge"
