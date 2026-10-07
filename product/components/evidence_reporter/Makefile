.DEFAULT_GOAL := help

PYTHON ?= python3
PROJECT_DIR := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
REPORTER := $(PROJECT_DIR)/source/evidence_reporter.py
RUN ?=
CAMPAIGN_DIR ?=
COMPARE ?=

.PHONY: help run campaign test

help: ## Show available targets
	@awk 'BEGIN {FS = ":.*## "; printf "\nTargets:\n"} \
		/^[a-zA-Z_-]+:.*## / {printf "  %-12s %s\n", $$1, $$2}' \
		$(MAKEFILE_LIST)

run: ## Render one run (RUN=reports/campaign-.../id)
	$(PYTHON) "$(REPORTER)" run "$(RUN)"

campaign: ## Render a campaign (CAMPAIGN_DIR=reports/campaign-...)
	$(PYTHON) "$(REPORTER)" campaign "$(CAMPAIGN_DIR)" $(if $(COMPARE),--compare "$(COMPARE)",)

test: ## Run unit and golden tests
	$(PYTHON) -m unittest discover -s "$(PROJECT_DIR)/tests" -v
