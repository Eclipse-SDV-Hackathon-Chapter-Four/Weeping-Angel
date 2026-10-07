#!/usr/bin/bash

die() { 
    echo "$*" >&2
    exit 1
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Needs libclang (bindgen for iceoryx2-pal-posix), e.g. apt install libclang-dev clang
cd "$ROOT_DIR/product/components/guardien" || die "Missing guardien directory"
cargo build --locked --bin guardian || die "Error building Guardian"
