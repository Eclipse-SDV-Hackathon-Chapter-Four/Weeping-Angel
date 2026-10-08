#!/usr/bin/env bash
# Copyright (c) 2026 Sebastian Russer
# Copyright (c) 2026 Peter Ulbrich
# Copyright (c) 2026 Michael Warmuth-Uhl
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
set -euo pipefail

# Guardian build dependency; idempotent for already initialized worktrees.
git submodule update --init --recursive

# Python venv for the evidence collector / Robot Framework (kept outside the repo tree).
# It is baked into the devcontainer image (.devcontainer/Dockerfile); provision it
# here only as a fallback for images that predate that.
if ! "$HOME/.venv/bin/python" -c "import robot, requests, yaml, jsonschema, can, cantools" >/dev/null 2>&1; then
  python3 -m venv "$HOME/.venv"
  "$HOME/.venv/bin/pip" install --quiet --upgrade pip
  "$HOME/.venv/bin/pip" install --quiet -r "$(dirname "$0")/requirements.txt"
  # KUKSA CAN provider dependencies (cantools, python-can, kuksa-client, ...).
  # requirements.in, not the pinned requirements.txt: the latter needs Python >= 3.12.
  "$HOME/.venv/bin/pip" install --quiet -r product/components/kuksa-can-provider/requirements.in
fi

# Make the prepared venv fully accessible to any user (read, write, execute), so
# a shell started as the host UID/GID can install packages without sudo.
[ -d "$HOME/.venv" ] && chmod -R a+rwX "$HOME/.venv" || true

echo "rustc:     $(rustc --version)"
echo "protoc:    $(protoc --version)"
echo "python:    $("$HOME/.venv/bin/python" --version)"
