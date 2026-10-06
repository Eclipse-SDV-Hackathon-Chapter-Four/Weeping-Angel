#!/usr/bin/bash

die() { 
    printf "$*" >&2
    exit 1
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$ROOT_DIR/fault-lib" || die "Missing fault_lib directory"
cargo build --workspace || die "Error building fault_lib"
bazel build //src/... || die "Error building DFM"
