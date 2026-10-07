# Recipe for the base dev-container image published as
# ghcr.io/waeschd/mod2sdv-hackathon:latest.
#
# Reconstructed from two sources:
#  1. the committed recipe in .devcontainer/Dockerfile before commit f69aa21
#     ("use prebuilt ghcr.io image, keep recipe in base.Dockerfile"), which
#     replaced that file with `FROM ghcr.io/...`;
#  2. the extra layers actually present in the published image (verified with
#     `docker history --no-trunc`): `unzip` was added to the apt list and a
#     KUKSA Data Broker + VSS catalogue layer was appended — neither was in the
#     committed recipe.
#
# The published image additionally carries the dev-container features
# (common-utils, git, rust, docker-in-docker) and the `devcontainer.metadata`
# label. Those are applied by the dev-container CLI *on top of* this
# Dockerfile, not by it: the base mcr rust image already provides
# rust/git/common-utils, and the CLI adds docker-in-docker. Rebuild the exact
# published image with the dev-container CLI:
#
#   devcontainer build --workspace-folder . \
#       --image-name ghcr.io/waeschd/mod2sdv-hackathon:latest
#
# (point devcontainer.json's "build.dockerfile" at this file for that). For the
# base layers only, without the docker-in-docker feature:
#
#   docker build -f .devcontainer/base.Dockerfile \
#       -t ghcr.io/waeschd/mod2sdv-hackathon:latest .devcontainer
#
# The tag matches the FROM in .devcontainer/Dockerfile, so a locally built
# image is used instead of the registry copy.

FROM mcr.microsoft.com/devcontainers/rust:1-bookworm

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

# KUKSA Data Broker + VSS catalogue (present in the published image, recovered
# from `docker history`; not part of the committed .devcontainer/Dockerfile).
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
