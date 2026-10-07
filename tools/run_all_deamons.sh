#!/usr/bin/bash

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
$SCRIPT_DIR/start_can.sh "$ROOT_DIR/product/config/battery_temp_with_ts.asc" || die "Could not start KUKSA CAN provider"

