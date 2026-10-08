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
# Shared self-provisioning for the replay/harness python environment
# (cantools, python-can, kuksa-client for the CAN replay; pyyaml for the
# campaign harness). Sourced by tools/run_case.sh and tools/run_campaign.sh;
# mirrors run_golden.sh's ensure_python_env (E2E_VENV convention).
ensure_python_env() {
  local venv="${E2E_VENV:-$HOME/.venv}" py
  # Prefer the image-baked shared venv when the current HOME has none, so any
  # user (e.g. the host UID/GID via tools/docker_shell.sh) uses the prepared one.
  if [ -z "${E2E_VENV:-}" ] && [ ! -x "$HOME/.venv/bin/python" ] \
      && [ -x /home/vscode/.venv/bin/python ]; then
    venv=/home/vscode/.venv
  fi
  py="$venv/bin/python"
  ROOT_ENV="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  if [ ! -x "$py" ] || ! "$py" -c "import cantools, can, yaml" >/dev/null 2>&1; then
    echo "== setting up the python environment in $venv" >&2
    [ -x "$py" ] || python3 -m venv "$venv" >&2 || return 1
    "$venv/bin/pip" install --quiet \
      -r "$ROOT_ENV/.devcontainer/requirements.txt" \
      -r "$ROOT_ENV/product/components/kuksa-can-provider/requirements.in" >&2 || return 1
  fi
  # Keep the environment usable for other users (best effort; no-op if already
  # world-accessible or if we do not own the files).
  chmod -R a+rwX "$venv" 2>/dev/null || true
  export PATH="$venv/bin:$PATH"
  "$py" -c "import cantools, can, yaml" >/dev/null 2>&1 || return 1
  return 0
}
