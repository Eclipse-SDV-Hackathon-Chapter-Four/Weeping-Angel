.DEFAULT_GOAL := help

DOCKER ?= docker
IMAGE ?= battery-guardian-dev:local
HTTP_PORT ?= 8080
ZENOH_CONNECT ?= tcp/host.docker.internal:7447

PROJECT_DIR := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
# Mount the product directory so the ../fault-lib path dependency and the
# shared catalog in ../../config/catalog resolve.
PRODUCT_DIR := $(abspath $(PROJECT_DIR)/../..)
WORKDIR := /workspace/components/$(notdir $(PROJECT_DIR))
LOCAL_UID := $(shell id -u)
LOCAL_GID := $(shell id -g)

CARGO_REGISTRY_VOLUME := battery-guardian-cargo-registry
CARGO_GIT_VOLUME := battery-guardian-cargo-git
TARGET_VOLUME := battery-guardian-target

MOUNTS := \
	--mount type=bind,source="$(PRODUCT_DIR)",target=/workspace \
	--mount type=volume,source=$(CARGO_REGISTRY_VOLUME),target=/usr/local/cargo/registry \
	--mount type=volume,source=$(CARGO_GIT_VOLUME),target=/usr/local/cargo/git \
	--mount type=volume,source=$(TARGET_VOLUME),target=$(WORKDIR)/target

RUN := $(DOCKER) run --rm --init \
	$(MOUNTS) \
	-w $(WORKDIR) \
	$(IMAGE)

# Formatting runs as the host user because it modifies source files.
RUN_AS_USER := $(DOCKER) run --rm --init \
	--user $(LOCAL_UID):$(LOCAL_GID) \
	-e CARGO_TARGET_DIR=/tmp/guardian-target \
	--mount type=bind,source="$(PRODUCT_DIR)",target=/workspace \
	-w $(WORKDIR) \
	$(IMAGE)

.PHONY: help image cache test test-model test-runtime build check \
	fmt fmt-check clippy run shell clean

help: ## Show available targets
	@awk 'BEGIN {FS = ":.*## "; printf "\nTargets:\n"} \
		/^[a-zA-Z_-]+:.*## / {printf "  %-16s %s\n", $$1, $$2}' \
		$(MAKEFILE_LIST)

image: ## Build the development image
	$(DOCKER) build \
		-f "$(PROJECT_DIR)/Dockerfile.dev" \
		-t $(IMAGE) \
		"$(PROJECT_DIR)"

cache: ## Create persistent Cargo volumes
	@$(DOCKER) volume create $(CARGO_REGISTRY_VOLUME) >/dev/null
	@$(DOCKER) volume create $(CARGO_GIT_VOLUME) >/dev/null
	@$(DOCKER) volume create $(TARGET_VOLUME) >/dev/null

test: image cache ## Run all tests
	$(RUN) cargo test --locked --all-targets

test-model: image cache ## Run physical-model tests
	$(RUN) cargo test --locked guardian_model::

test-runtime: image cache ## Run runtime and freshness tests
	$(RUN) cargo test --locked guardian_runtime::

build: image cache ## Build the Guardian binary
	$(RUN) cargo build --locked --bin guardian

fmt: image ## Format Rust sources
	$(RUN_AS_USER) cargo fmt --all

fmt-check: image ## Verify formatting without modifying files
	$(RUN_AS_USER) cargo fmt --all -- --check

clippy: image cache ## Run the Rust linter
	$(RUN) cargo clippy --locked --all-targets -- -D warnings

check: fmt-check clippy test ## Run all quality checks

run: image cache ## Start Guardian and expose its HTTP service
	$(DOCKER) run --rm --init -it \
		--add-host=host.docker.internal:host-gateway \
		-p $(HTTP_PORT):8080 \
		-e ZENOH_CONNECT=$(ZENOH_CONNECT) \
		$(MOUNTS) \
		-w $(WORKDIR) \
		$(IMAGE) \
		cargo run --locked --bin guardian

shell: image cache ## Open a shell in the development container
	$(DOCKER) run --rm --init -it \
		$(MOUNTS) \
		-w $(WORKDIR) \
		$(IMAGE) \
		sh

clean: image cache ## Remove compiled artifacts, retain downloaded crates
	$(RUN) cargo clean
