FROM ghcr.io/waeschd/mod2sdv-hackathon:latest

USER root

# KUKSA CAN provider (dbcfeeder): Python source release in its own venv.
# requirements.in, not the pinned requirements.txt: the latter needs Python >= 3.12.
# The wrapper supplies the two defaults the feeder otherwise only finds when
# started from its own directory (--canport is just a name for CAN log replay).
ARG KUKSA_CAN_PROVIDER_VERSION=0.5.0
ARG KUKSA_CAN_PROVIDER_DIR=/opt/kuksa-can-provider
RUN git clone --depth 1 --branch "${KUKSA_CAN_PROVIDER_VERSION}" \
        https://github.com/eclipse-kuksa/kuksa-can-provider.git "${KUKSA_CAN_PROVIDER_DIR}" \
    && rm -rf "${KUKSA_CAN_PROVIDER_DIR}/.git" \
    && python3 -m venv "${KUKSA_CAN_PROVIDER_DIR}/venv" \
    && "${KUKSA_CAN_PROVIDER_DIR}/venv/bin/pip" install --no-cache-dir --upgrade pip \
    && "${KUKSA_CAN_PROVIDER_DIR}/venv/bin/pip" install --no-cache-dir \
        -r "${KUKSA_CAN_PROVIDER_DIR}/requirements.in" \
    && printf '%s\n' '#!/bin/sh' \
        "exec ${KUKSA_CAN_PROVIDER_DIR}/venv/bin/python ${KUKSA_CAN_PROVIDER_DIR}/dbcfeeder.py \\" \
        "  --canport vcan0 --dbc-default ${KUKSA_CAN_PROVIDER_DIR}/dbc_default_values.json \"\$@\"" \
        > /usr/local/bin/kuksa-can-provider \
    && chmod +x /usr/local/bin/kuksa-can-provider

# Python environment for the Evidence Collector / Robot Framework tests and the
# CAN replay / campaign harness (cantools, python-can, kuksa-client, ...).
# Baked into the image so .devcontainer/post-create.sh only has to provision it
# as a fallback; devcontainer.json points VIRTUAL_ENV and PATH at
# /home/vscode/.venv. requirements.txt is the devcontainer build context.
COPY requirements.txt /tmp/dev-requirements.txt
RUN python3 -m venv /home/vscode/.venv \
    && /home/vscode/.venv/bin/pip install --no-cache-dir --upgrade pip \
    && /home/vscode/.venv/bin/pip install --no-cache-dir -r /tmp/dev-requirements.txt \
    && /home/vscode/.venv/bin/pip install --no-cache-dir -r "${KUKSA_CAN_PROVIDER_DIR}/requirements.in" \
    && rm /tmp/dev-requirements.txt \
    && chown -R vscode:vscode /home/vscode/.venv

# Chromium (headless) for screenshots of the generated HTML reports, e.g.
#   chromium --headless --no-sandbox --screenshot=report.png file:///app/report.html
RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
    && apt-get install -y --no-install-recommends chromium fonts-liberation \
    && rm -rf /var/lib/apt/lists/*
