#!/usr/bin/env bash
# Start the KUKSA Data Broker in the background with the official VSS catalogue
# (KUKSA_VSS_FILE, set by the dev container image).
# Loopback only, like zenohd: all components run inside the dev container.
# Skips if databroker is missing or port 55555 is already taken.
set -euo pipefail

ADDRESS="127.0.0.1"
PORT=55555
VSS="${KUKSA_VSS_FILE:-/usr/local/share/kuksa/vss_release_6.0.json}"
LOG="/tmp/databroker.log"

if ! command -v databroker >/dev/null; then
  echo "databroker: not installed (rebuild the dev container), not starting"
  exit 0
fi
if [ ! -f "$VSS" ]; then
  echo "databroker: VSS catalogue $VSS not found (rebuild the dev container), not starting"
  exit 0
fi
if (exec 3<>/dev/tcp/$ADDRESS/$PORT) 2>/dev/null; then
  echo "databroker: port $PORT already in use, not starting"
  exit 0
fi

# setsid: keep running after the starting shell exits.
# --insecure: no TLS and no auth tokens; fine on loopback.
setsid databroker --address "$ADDRESS" --port "$PORT" --vss "$VSS" --insecure \
  >"$LOG" 2>&1 < /dev/null &
echo "databroker: started on $ADDRESS:$PORT with $VSS (log: $LOG)"
