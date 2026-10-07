.DEFAULT_GOAL := help

PYTHON ?= python3
PROJECT_DIR := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
HARNESS := $(PROJECT_DIR)/harness.py
CAMPAIGN ?=
SCENARIO ?=
SELECT := $(if $(CAMPAIGN),--campaign $(CAMPAIGN),) $(if $(SCENARIO),--scenario $(SCENARIO),)

.PHONY: help validate plan generate test

help: ## Show available targets
	@awk 'BEGIN {FS = ":.*## "; printf "\nTargets:\n"} \
		/^[a-zA-Z_-]+:.*## / {printf "  %-12s %s\n", $$1, $$2}' \
		$(MAKEFILE_LIST)

validate: ## Validate Golden Scenarios and campaign configuration
	$(PYTHON) "$(HARNESS)" validate

plan: ## List experiments (optionally CAMPAIGN=... SCENARIO=...)
	$(PYTHON) "$(HARNESS)" plan $(SELECT)

generate: ## Generate experiment bundles (optionally CAMPAIGN=... SCENARIO=...)
	$(PYTHON) "$(HARNESS)" generate $(SELECT)

test: validate ## Run lightweight Harness unit tests
	$(PYTHON) -m unittest discover -s "$(PROJECT_DIR)/tests" -v
