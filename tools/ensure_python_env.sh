#!/usr/bin/env bash
# Shared self-provisioning for the replay/harness python environment
# (cantools, python-can, kuksa-client for the CAN replay; pyyaml for the
# campaign harness). Sourced by tools/run_case.sh and tools/run_campaign.sh;
# mirrors run_golden.sh's ensure_python_env (E2E_VENV convention).
ensure_python_env() {
  local venv="${E2E_VENV:-$HOME/.venv}" py
  py="$venv/bin/python"
  ROOT_ENV="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  if [ ! -x "$py" ] || ! "$py" -c "import cantools, can, yaml" >/dev/null 2>&1; then
    echo "== setting up the python environment in $venv" >&2
    [ -x "$py" ] || python3 -m venv "$venv" >&2 || return 1
    "$venv/bin/pip" install --quiet \
      -r "$ROOT_ENV/.devcontainer/requirements.txt" \
      -r "$ROOT_ENV/product/components/kuksa-can-provider/requirements.in" >&2 || return 1
  fi
  export PATH="$venv/bin:$PATH"
  "$py" -c "import cantools, can, yaml" >/dev/null 2>&1 || return 1
  return 0
}
