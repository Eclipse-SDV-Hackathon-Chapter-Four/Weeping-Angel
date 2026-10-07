#!/usr/bin/bash

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

