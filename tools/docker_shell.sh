#!/usr/bin/env bash
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
# Start the hackathon dev container for this repo and open a shell, or run the
# given command inside it. If a container with the same name already exists, it
# is reused (started if stopped) instead of starting a second one.
#
#   tools/docker_shell.sh                          # interactive bash
#   tools/docker_shell.sh cargo test --features observer
#   tools/docker_shell.sh tools/run_case.sh product/config/signal_out_of_range
#
# Run as the host UID/GID (--user) so files created in the bind-mounted repo are
# owned by you, not root. The repo is mounted at /app (the container's workdir).
# When joining an existing container, its configured user and mounts are kept.
#
# Env: E2E_IMAGE (default weeping-angel-devcontainer:latest),
#      E2E_CONTAINER (container name), E2E_OBSERVER_PORT (host port, default 8090),
#      E2E_GUARDIAN_PORT (host port for Guardian HTTP 8080 if set).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${E2E_IMAGE:-weeping-angel-devcontainer:latest}"
NAME="${E2E_CONTAINER:-weeping-angel-shell}"

# uidmap: run as the invoking user so the bind-mounted repo stays user-owned.
USERMAP=(--user "$(id -u):$(id -g)")

# Ports: observer 8090 by default; set E2E_OBSERVER_PORT to remap, or to the
# empty string to skip. E2E_GUARDIAN_PORT optionally publishes Guardian HTTP 8080.
PORTS=()
if [ -n "${E2E_OBSERVER_PORT-8090}" ]; then PORTS+=(-p "${E2E_OBSERVER_PORT:-8090}:8090"); fi
if [ -n "${E2E_GUARDIAN_PORT:-}" ]; then PORTS+=(-p "${E2E_GUARDIAN_PORT}:8080"); fi

# Interactive only when both stdin and stdout are a TTY.
TTY=(); [ -t 0 ] && [ -t 1 ] && TTY=(-it)

# Default to a shell; otherwise run the script's arguments as the command.
CMD=("$@"); [ ${#CMD[@]} -eq 0 ] && CMD=(bash)

# Reuse an existing container with this name: start it if needed, then exec in.
if docker container inspect "$NAME" >/dev/null 2>&1; then
  if [ "$(docker inspect -f '{{.State.Running}}' "$NAME")" = "true" ]; then
    echo "docker_shell: joining running container '$NAME'" >&2
  else
    echo "docker_shell: starting existing container '$NAME'" >&2
    docker start "$NAME" >/dev/null
  fi
  # Keep the container's configured user; use /app when the repo is mounted there.
  EXEC_WORKDIR=()
  if docker exec "$NAME" test -d /app >/dev/null 2>&1; then EXEC_WORKDIR=(-w /app); fi
  exec docker exec "${TTY[@]}" "${EXEC_WORKDIR[@]}" "$NAME" "${CMD[@]}"
fi

exec docker run --rm "${TTY[@]}" \
  --name "$NAME" \
  "${USERMAP[@]}" \
  -v "$ROOT:/app" \
  -w /app \
  "${PORTS[@]}" \
  "$IMAGE" "${CMD[@]}"
