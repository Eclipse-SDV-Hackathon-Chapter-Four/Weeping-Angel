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

$SCRIPT_DIR/stop_dfm_sovd_bridge.sh
$SCRIPT_DIR/stop_dfm.sh               
$SCRIPT_DIR/stop_guardian.sh          
$SCRIPT_DIR/stop_vss_bridge.sh        
$SCRIPT_DIR/stop_kuksa_data_broker.sh           

