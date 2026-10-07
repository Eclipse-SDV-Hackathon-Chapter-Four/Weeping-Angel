.DEFAULT_GOAL := help

CARGO ?= cargo
SOVD_URL ?= http://127.0.0.1:7690/sovd
DFM_SOVD_PATH ?= battery_guardian

PROJECT_DIR := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
MANIFEST := $(PROJECT_DIR)/Cargo.toml

.PHONY: help test build check fmt fmt-check clippy run clean

help: ## Show available targets
	@awk 'BEGIN {FS = ":.*## "; printf "\nTargets:\n"} \
		/^[a-zA-Z_-]+:.*## / {printf "  %-16s %s\n", $$1, $$2}' \
		$(MAKEFILE_LIST)

test: ## Run all tests
	$(CARGO) test --manifest-path "$(MANIFEST)" --all-targets

build: ## Build the bridge binary
	$(CARGO) build --manifest-path "$(MANIFEST)" --bin dfm_sovd_bridge

fmt: ## Format Rust sources
	$(CARGO) fmt --manifest-path "$(MANIFEST)" --all

fmt-check: ## Verify formatting without modifying files
	$(CARGO) fmt --manifest-path "$(MANIFEST)" --all -- --check

clippy: ## Run the Rust linter
	$(CARGO) clippy --manifest-path "$(MANIFEST)" --all-targets -- -D warnings

check: fmt-check clippy test ## Run all quality checks

run: ## Start the bridge in the dev container (DFM must be running)
	SOVD_URL="$(SOVD_URL)" DFM_SOVD_PATH="$(DFM_SOVD_PATH)" \
	$(CARGO) run --manifest-path "$(MANIFEST)" --bin dfm_sovd_bridge

clean: ## Remove compiled artifacts
	$(CARGO) clean --manifest-path "$(MANIFEST)"
