#!/usr/bin/env bash
# Replay one .asc CAN log once through the KUKSA CAN provider into the Data
# Broker (127.0.0.1:55555, start it first with start-databroker.sh).
# Runs in the foreground and stops the provider once the replay is done
# (the provider itself keeps running after a single pass).
#
#   product/components/start_can.sh <file.asc>
set -euo pipefail

if [ $# -ne 1 ]; then
  echo "usage: $0 <file.asc>" >&2
  exit 2
fi
ASC="$1"

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FEEDER_DIR="$REPO/product/components/kuksa-can-provider"
DBC="$REPO/product/config/battery_temp.dbc"
MAPPING="$REPO/product/config/vss_dbc.json"
ADDRESS="127.0.0.1"
PORT=55555

for f in "$ASC" "$DBC" "$MAPPING" "$FEEDER_DIR/dbcfeeder.py"; do
  [ -f "$f" ] || { echo "start_can: $f not found" >&2; exit 1; }
done
if ! (exec 3<>/dev/tcp/$ADDRESS/$PORT) 2>/dev/null; then
  echo "start_can: no Data Broker on $ADDRESS:$PORT (run start-databroker.sh first)" >&2
  exit 1
fi

# --canport is only a name for the virtual bus during a log replay.
coproc FEEDER {
  KUKSA_ADDRESS="$ADDRESS" KUKSA_PORT="$PORT" exec python3 "$FEEDER_DIR/dbcfeeder.py" \
    --canport vcan0 \
    --dbc-default "$FEEDER_DIR/dbc_default_values.json" \
    --dbcfile "$DBC" \
    --mapping "$MAPPING" \
    --dumpfile "$ASC" 2>&1
}
# Bash unsets FEEDER/FEEDER_PID (and closes the fd) once the coprocess ends,
# so keep our own copies.
feeder_pid=$FEEDER_PID
exec {feeder_out}<&"${FEEDER[0]}"

# The provider shuts down gracefully on SIGTERM, which takes a moment;
# wait for it (up to 5 s), then force it.
stop_feeder() {
  kill "$feeder_pid" 2>/dev/null || return 0
  for _ in $(seq 50); do
    kill -0 "$feeder_pid" 2>/dev/null || return 0
    sleep 0.1
  done
  kill -KILL "$feeder_pid" 2>/dev/null || true
}
trap stop_feeder EXIT

replayed=false
while IFS= read -r line <&"$feeder_out"; do
  echo "$line"
  if [[ "$line" == *"Replayed all messages"* ]]; then
    replayed=true
    break
  fi
done

if $replayed; then
  echo "start_can: replay of $ASC complete"
else
  echo "start_can: provider stopped before the replay completed" >&2
  exit 1
fi
