#!/usr/bin/bash
# Copyright (c) 2026 Matthias Knöfel
#
# This program and the accompanying materials are made available under
# the terms of the Eclipse Public License 2.0 which accompanies this
# distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
#
# AI Disclosure: This file was mostly AI-generated.
#
# SPDX-License-Identifier: EPL-2.0 and CC0-1.0
# Assisted-by: Claude Sonnet 5.5

die() { 
    echo "$*" >&2
    exit 1
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Needs libclang (bindgen for iceoryx2-pal-posix), e.g. apt install libclang-dev clang
cd "$ROOT_DIR/product/components/guardien" || die "Missing guardien directory"
cargo build --locked --bin guardian || die "Error building Guardian"
