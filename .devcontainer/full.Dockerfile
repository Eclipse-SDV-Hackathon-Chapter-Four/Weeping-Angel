# Self-contained dev-container image: the reconstructed base recipe
# (.devcontainer/base.Dockerfile) plus the project layer
# (.devcontainer/Dockerfile) plus Chromium for HTML-report screenshots.
# Unlike .devcontainer/Dockerfile it does not depend on the pulled
# ghcr.io/waeschd/mod2sdv-hackathon base image.
#
#   docker build -f .devcontainer/full.Dockerfile -t weeping-angel-full:latest .devcontainer
#
# The dev-container features (docker-in-docker) are still applied by the
# dev-container CLI on top of this file; it reproduces the base + project
# layers only.

FROM mcr.microsoft.com/devcontainers/rust:1-bookworm

USER root

# --- base recipe (mirrors .devcontainer/base.Dockerfile) ---

RUN rustup component add rustfmt clippy

RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
    && apt-get install -y --no-install-recommends \
        build-essential pkg-config cmake clang libclang-dev libssl-dev \
        protobuf-compiler libprotobuf-dev \
        python3 python3-venv python3-pip \
        curl jq ca-certificates unzip \
    && rm -rf /var/lib/apt/lists/*

ENV LIBCLANG_PATH=/usr/lib/llvm-14/lib

ARG TOXIPROXY_VERSION=2.12.0
RUN arch="$(dpkg --print-architecture)" \
    && curl -fsSL -o /usr/local/bin/toxiproxy-server \
        "https://github.com/Shopify/toxiproxy/releases/download/v${TOXIPROXY_VERSION}/toxiproxy-server-linux-${arch}" \
    && curl -fsSL -o /usr/local/bin/toxiproxy-cli \
        "https://github.com/Shopify/toxiproxy/releases/download/v${TOXIPROXY_VERSION}/toxiproxy-cli-linux-${arch}" \
    && chmod +x /usr/local/bin/toxiproxy-server /usr/local/bin/toxiproxy-cli

ARG BAZEL_VERSION=8.3.0
RUN case "$(dpkg --print-architecture)" in \
        amd64) bazel_arch=x86_64 ;; \
        arm64) bazel_arch=arm64 ;; \
        *) echo "unsupported arch" >&2; exit 1 ;; \
    esac \
    && curl -fsSL -o /usr/local/bin/bazel \
        "https://github.com/bazelbuild/bazel/releases/download/${BAZEL_VERSION}/bazel-${BAZEL_VERSION}-linux-${bazel_arch}" \
    && chmod +x /usr/local/bin/bazel

# Zenoh router (uProtocol message bus). The plugins (rest, storage_manager)
# must stay next to the zenohd binary, which is where zenohd looks for them.
# Keep the version in line with the zenoh crate used by the Rust components.
ARG ZENOH_VERSION=1.10.1
RUN case "$(dpkg --print-architecture)" in \
        amd64) zenoh_arch=x86_64 ;; \
        arm64) zenoh_arch=aarch64 ;; \
        *) echo "unsupported arch" >&2; exit 1 ;; \
    esac \
    && curl -fsSL -o /tmp/zenoh.zip \
        "https://github.com/eclipse-zenoh/zenoh/releases/download/${ZENOH_VERSION}/zenoh-${ZENOH_VERSION}-${zenoh_arch}-unknown-linux-gnu-standalone.zip" \
    && unzip -o /tmp/zenoh.zip -d /usr/local/bin \
    && rm /tmp/zenoh.zip

# KUKSA Data Broker + VSS catalogue.
ARG KUKSA_DATABROKER_VERSION=0.7.1
ARG KUKSA_VSS_RELEASE=6.0
RUN arch="$(dpkg --print-architecture)" \
    && base="https://github.com/eclipse-kuksa/kuksa-databroker/releases/download/${KUKSA_DATABROKER_VERSION}" \
    && curl -fsSL "${base}/databroker-${arch}-${KUKSA_DATABROKER_VERSION}.tar.gz" \
        | tar -xzf - -C /usr/local/bin databroker \
    && curl -fsSL "${base}/databroker-cli-${arch}-${KUKSA_DATABROKER_VERSION}.tar.gz" \
        | tar -xzf - -C /usr/local/bin databroker-cli \
    && mkdir -p /usr/local/share/kuksa \
    && curl -fsSL -o "/usr/local/share/kuksa/vss_release_${KUKSA_VSS_RELEASE}.json" \
        "https://raw.githubusercontent.com/eclipse-kuksa/kuksa-databroker/${KUKSA_DATABROKER_VERSION}/data/vss-core/vss_release_${KUKSA_VSS_RELEASE}.json"
ENV KUKSA_VSS_FILE=/usr/local/share/kuksa/vss_release_${KUKSA_VSS_RELEASE}.json

# --- project layer (mirrors .devcontainer/Dockerfile) ---

# KUKSA CAN provider (dbcfeeder): Python source release in its own venv.
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
COPY requirements.txt /tmp/dev-requirements.txt
RUN python3 -m venv /home/vscode/.venv \
    && /home/vscode/.venv/bin/pip install --no-cache-dir --upgrade pip \
    && /home/vscode/.venv/bin/pip install --no-cache-dir -r /tmp/dev-requirements.txt \
    && /home/vscode/.venv/bin/pip install --no-cache-dir -r "${KUKSA_CAN_PROVIDER_DIR}/requirements.in" \
    && rm /tmp/dev-requirements.txt \
    && chown -R vscode:vscode /home/vscode/.venv

# --- screenshots ---

# Chromium (headless) for screenshots of the generated HTML reports, e.g.
#   chromium --headless --no-sandbox --screenshot=report.png file:///app/report.html
RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
    && apt-get install -y --no-install-recommends chromium fonts-liberation \
    && rm -rf /var/lib/apt/lists/*

# Make the prepared venv fully accessible to any user (read, write, execute).
RUN chmod 755 /home/vscode \
    && chmod -R a+rwX /home/vscode/.venv
