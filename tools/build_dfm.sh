#!/usr/bin/bash
# Copyright (c) 2026 Michael Warmuth-Uhl
# Copyright (c) 2026 Alwin Berger
#
# This program and the accompanying materials are made available under
# the terms of the Eclipse Public License 2.0 which accompanies this
# distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
#
# AI Disclosure: This file was mostly AI-generated.
#
# SPDX-License-Identifier: EPL-2.0 and CC0-1.0
# Assisted-by: DeepSeek v4.1 Flash

die() {
    echo "$*" >&2
    exit 1
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$ROOT_DIR/product/components/fault-lib" || die "Missing fault_lib directory"
# The evidence chain (run_golden.sh, run_case.sh / run_campaign.sh) consumes
# the CARGO-built dfm_bin — cargo build is the artifact of record. If this
# build fails there is nothing meaningful to fall back to, so die here.
# NOTE: the toolchain-specific upstream bazel build of the DFM sources
# (`bazel build //src/...`) was removed as dead code: it fails in this
# environment (iceoryx2-pal-posix: native static lib `socket_macros` not
# found) and was unreachable anyway because a successful workspace build
# always leaves target/debug/dfm_bin in place. Re-add it as an explicit,
# separately verifiable step if the bazel workflow is ever revived.
cargo build --workspace || die "Error building fault_lib"
