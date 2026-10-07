#!/usr/bin/env bash
set -euo pipefail

# Guardian build dependency; idempotent for already initialized worktrees.
git submodule update --init --recursive

# Python venv for the evidence collector / Robot Framework (kept outside the repo tree).
python3 -m venv "$HOME/.venv"
"$HOME/.venv/bin/pip" install --quiet --upgrade pip
"$HOME/.venv/bin/pip" install --quiet -r "$(dirname "$0")/requirements.txt"
# KUKSA CAN provider dependencies (cantools, python-can, kuksa-client, ...).
# requirements.in, not the pinned requirements.txt: the latter needs Python >= 3.12.
"$HOME/.venv/bin/pip" install --quiet -r product/components/kuksa-can-provider/requirements.in

echo "rustc:     $(rustc --version)"
echo "protoc:    $(protoc --version)"
echo "python:    $("$HOME/.venv/bin/python" --version)"
