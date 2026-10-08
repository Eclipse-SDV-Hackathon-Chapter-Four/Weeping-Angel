#!/usr/bin/env bash
# Copyright (c) 2026 Sebastian Russer
# Copyright (c) 2026 Matthias Knöfel
# Copyright (c) 2026 Alwin Berger
#
# This program and the accompanying materials are made available under
# the terms of the Eclipse Public License 2.0 which accompanies this
# distribution, and is available at https://www.eclipse.org/legal/epl-2.0/
#
# AI Disclosure: This file was mostly AI-generated.
#
# SPDX-License-Identifier: EPL-2.0 and CC0-1.0
# Assisted-by: Claude Opus 5.5, DeepSeek v4.1 Flash
# Start the KUKSA Data Broker in the background with the official VSS catalogue
# (KUKSA_VSS_FILE, set by the dev container image) plus the product overlay
# (product/config/vss_overlay.json: custom paths such as SourceTimestamp).
# Loopback only, like zenohd: all components run inside the dev container.
# Skips if databroker is missing or port 55555 is already taken.
set -euo pipefail

ADDRESS="127.0.0.1"
PORT=55555
VSS="${KUKSA_VSS_FILE:-/usr/local/share/kuksa/vss_release_6.0.json}"
OVERLAY="$(cd "$(dirname "$0")/.." && pwd)/product/config/vss_overlay.json"
LOG="${DATABROKER_LOG:-/tmp/databroker.log}"

if ! command -v databroker >/dev/null; then
  echo "databroker: not installed (rebuild the dev container), not starting"
  exit 0
fi
if [ ! -f "$VSS" ]; then
  echo "databroker: VSS catalogue $VSS not found (rebuild the dev container), not starting"
  exit 0
fi
if [ ! -f "$OVERLAY" ]; then
  echo "databroker: VSS overlay $OVERLAY not found, not starting"
  exit 0
fi
if (exec 3<>/dev/tcp/$ADDRESS/$PORT) 2>/dev/null; then
  echo "databroker: port $PORT already in use, not starting"
  exit 0
fi

# setsid: keep running after the starting shell exits.
# --insecure: no TLS and no auth tokens; fine on loopback.
setsid databroker --address "$ADDRESS" --port "$PORT" --vss "$VSS,$OVERLAY" --insecure \
  >"$LOG" 2>&1 < /dev/null &
echo "databroker: started on $ADDRESS:$PORT with $VSS + $OVERLAY (log: $LOG)"
