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

die() { 
    echo "$*" >&2
    exit 1
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

$SCRIPT_DIR/run_dfm.sh                || die "Could not start DFM"
$SCRIPT_DIR/run_dfm_sovd_bridge.sh    || die "Could not start SOVD provider"
$SCRIPT_DIR/run_guardian.sh           || die "Could not start Guardian"
$SCRIPT_DIR/run_vss_bridge.sh         || die "Could not start VSS Bridge"
$SCRIPT_DIR/run_kuksa_data_broker.sh  || die "Could not start Kuksa Data Broker"
$SCRIPT_DIR/start_can.sh "$ROOT_DIR"/product/config/battery_temp_with_ts.asc || die "Could not start KUKSA CAN provider" &

