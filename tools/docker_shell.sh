#!/usr/bin/env bash
# Start the hackathon dev container for this repo and open a shell, or run the
# given command inside it.
#
#   tools/docker_shell.sh                          # interactive bash
#   tools/docker_shell.sh cargo test --features observer
#   tools/docker_shell.sh tools/run_case.sh product/config/signal_out_of_range
#
# Run as the host UID/GID (--user) so files created in the bind-mounted repo are
# owned by you, not root. The repo is mounted at /app (the container's workdir).
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

exec docker run --rm "${TTY[@]}" \
  --name "$NAME" \
  "${USERMAP[@]}" \
  -v "$ROOT:/app" \
  -w /app \
  "${PORTS[@]}" \
  "$IMAGE" "${CMD[@]}"
