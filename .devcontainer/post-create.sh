#!/usr/bin/env bash
set -euo pipefail

# Python venv for the evidence collector / Robot Framework (kept outside the repo tree).
python3 -m venv "$HOME/.venv"
"$HOME/.venv/bin/pip" install --quiet --upgrade pip
"$HOME/.venv/bin/pip" install --quiet -r "$(dirname "$0")/requirements.txt"

echo "rustc:     $(rustc --version)"
echo "protoc:    $(protoc --version)"
echo "python:    $("$HOME/.venv/bin/python" --version)"
echo "toxiproxy: $(toxiproxy-server --version 2>&1 | head -1)"
