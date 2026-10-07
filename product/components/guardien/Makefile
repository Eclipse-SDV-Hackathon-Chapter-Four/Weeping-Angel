.DEFAULT_GOAL := help

CARGO ?= cargo
PYTHON ?= python3
HTTP_HOST ?= 0.0.0.0
HTTP_PORT ?= 8080
ZENOH_CONNECT ?= tcp/127.0.0.1:7447

PROJECT_DIR := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
PRODUCT_DIR := $(abspath $(PROJECT_DIR)/../..)
MANIFEST := $(PROJECT_DIR)/Cargo.toml
CONFIG_FILE ?= $(PRODUCT_DIR)/config/battery_guardian/guardian_model.yaml
FAULT_CATALOG ?= $(PRODUCT_DIR)/config/battery_guardian/guardian_diagnostics.json
INJECTION_MODEL ?= $(PRODUCT_DIR)/config/battery_guardian/fault_injection_model.yaml
INJECTION_VALIDATOR := $(PRODUCT_DIR)/scripts/validate_fault_injection_model.py

.PHONY: help test test-model test-runtime test-injection-config validate-injection-config \
	build check fmt fmt-check clippy run clean

help: ## Show available targets
	@awk 'BEGIN {FS = ":.*## "; printf "\nTargets:\n"} \
		/^[a-zA-Z_-]+:.*## / {printf "  %-16s %s\n", $$1, $$2}' \
		$(MAKEFILE_LIST)

test: ## Run all tests
	$(CARGO) test --manifest-path "$(MANIFEST)" --locked --all-targets

test-model: ## Run physical-model tests
	$(CARGO) test --manifest-path "$(MANIFEST)" --locked guardian_model::

test-runtime: ## Run runtime and freshness tests
	$(CARGO) test --manifest-path "$(MANIFEST)" --locked guardian_runtime::

test-injection-config: ## Run fault-injection configuration validator tests
	PYTHONPATH="$(PRODUCT_DIR)" $(PYTHON) -m unittest \
		discover -s "$(PRODUCT_DIR)/tests" -p 'test_fault_injection_model.py'

validate-injection-config: ## Validate the canonical fault-injection model
	$(PYTHON) "$(INJECTION_VALIDATOR)" "$(INJECTION_MODEL)"

build: ## Build the Guardian binary
	$(CARGO) build --manifest-path "$(MANIFEST)" --locked --bin guardian

fmt: ## Format Rust sources
	$(CARGO) fmt --manifest-path "$(MANIFEST)" --all

fmt-check: ## Verify formatting without modifying files
	$(CARGO) fmt --manifest-path "$(MANIFEST)" --all -- --check

clippy: ## Run the Rust linter
	$(CARGO) clippy --manifest-path "$(MANIFEST)" --locked --all-targets -- -D warnings

check: fmt-check clippy test validate-injection-config test-injection-config ## Run all quality checks

run: ## Start Guardian in the dev container
	GUARDIAN_CONFIG="$(CONFIG_FILE)" \
	GUARDIAN_FAULT_CATALOG="$(FAULT_CATALOG)" \
	HOST="$(HTTP_HOST)" \
	PORT="$(HTTP_PORT)" \
	ZENOH_CONNECT="$(ZENOH_CONNECT)" \
	$(CARGO) run --manifest-path "$(MANIFEST)" --locked --bin guardian

clean: ## Remove compiled artifacts
	$(CARGO) clean --manifest-path "$(MANIFEST)"
