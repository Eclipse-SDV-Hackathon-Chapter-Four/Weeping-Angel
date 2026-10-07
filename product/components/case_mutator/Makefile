.DEFAULT_GOAL := help

CARGO ?= cargo
PROJECT_DIR := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
MANIFEST := $(PROJECT_DIR)/Cargo.toml
REQUEST ?= $(PROJECT_DIR)/examples/out_of_range.yaml
OUTPUT_DIR ?= $(PROJECT_DIR)/cases

.PHONY: help build test fmt fmt-check clippy check run clean

help: ## Show available targets
	@awk 'BEGIN {FS = ":.*## "; printf "\nTargets:\n"} \
		/^[a-zA-Z_-]+:.*## / {printf "  %-12s %s\n", $$1, $$2}' \
		$(MAKEFILE_LIST)

build: ## Build the case-mutator binary
	$(CARGO) build --manifest-path "$(MANIFEST)" --locked

test: ## Run all case-mutator tests
	$(CARGO) test --manifest-path "$(MANIFEST)" --locked --all-targets

fmt: ## Format Rust sources
	$(CARGO) fmt --manifest-path "$(MANIFEST)" --all

fmt-check: ## Verify Rust formatting
	$(CARGO) fmt --manifest-path "$(MANIFEST)" --all -- --check

clippy: ## Run Clippy with warnings denied
	$(CARGO) clippy --manifest-path "$(MANIFEST)" --locked --all-targets -- -D warnings

check: fmt-check clippy test ## Run all quality checks

run: ## Generate one case from REQUEST into OUTPUT_DIR
	$(CARGO) run --manifest-path "$(MANIFEST)" --locked -- \
		--request "$(REQUEST)" --output-dir "$(OUTPUT_DIR)"

clean: ## Remove compiled artifacts
	$(CARGO) clean --manifest-path "$(MANIFEST)"
