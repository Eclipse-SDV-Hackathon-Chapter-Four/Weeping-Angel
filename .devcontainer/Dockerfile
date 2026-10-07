FROM mcr.microsoft.com/devcontainers/rust:1-bookworm

RUN rustup component add rustfmt clippy

RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
    && apt-get install -y --no-install-recommends \
        build-essential pkg-config cmake clang libclang-dev libssl-dev \
        protobuf-compiler libprotobuf-dev \
        python3 python3-venv python3-pip \
        curl jq ca-certificates \
    && rm -rf /var/lib/apt/lists/*

ARG TOXIPROXY_VERSION=2.12.0
RUN arch="$(dpkg --print-architecture)" \
    && curl -fsSL -o /usr/local/bin/toxiproxy-server \
        "https://github.com/Shopify/toxiproxy/releases/download/v${TOXIPROXY_VERSION}/toxiproxy-server-linux-${arch}" \
    && curl -fsSL -o /usr/local/bin/toxiproxy-cli \
        "https://github.com/Shopify/toxiproxy/releases/download/v${TOXIPROXY_VERSION}/toxiproxy-cli-linux-${arch}" \
    && chmod +x /usr/local/bin/toxiproxy-server /usr/local/bin/toxiproxy-cli

ENV LIBCLANG_PATH=/usr/lib/llvm-14/lib

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
