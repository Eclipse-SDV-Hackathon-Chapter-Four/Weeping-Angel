#!/usr/bin/env bash
# Copyright (c) 2026 Sebastian Russer
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
# Start the Zenoh router (uProtocol bus, ADR-005) in the background.
# Loopback only: all components run inside the dev container, and with
# rootless Podman the container may share the host's LAN/Wi-Fi interface.
# Skips if zenohd is missing or port 7447 is already taken.
set -euo pipefail

ENDPOINT="tcp/127.0.0.1:7447"
LOG="${ZENOH_LOG:-/tmp/zenohd.log}"

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
