#!/usr/bin/env bash
# Start the Zenoh router (uProtocol bus, ADR-005) in the background.
# Loopback only: all components run inside the dev container, and with
# rootless Podman the container may share the host's LAN/Wi-Fi interface.
# Skips if zenohd is missing or port 7447 is already taken.
set -euo pipefail

ENDPOINT="tcp/127.0.0.1:7447"
LOG="/tmp/zenohd.log"

if ! command -v zenohd >/dev/null; then
  echo "zenohd: not installed (rebuild the dev container), not starting"
  exit 0
fi
if (exec 3<>/dev/tcp/127.0.0.1/7447) 2>/dev/null; then
  echo "zenohd: port 7447 already in use, not starting"
  exit 0
fi

# setsid: keep running after the starting shell exits.
# No multicast scouting: clients find the router via ZENOH_CONNECT.
setsid zenohd -l "$ENDPOINT" --no-multicast-scouting >"$LOG" 2>&1 < /dev/null &
echo "zenohd: started on $ENDPOINT (log: $LOG)"
