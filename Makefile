# TrustBridge Contract — Makefile
#
# Common tasks for building, testing, and deploying the Soroban registry contract.
# Requires: Rust (≥ 1.84), wasm target, Stellar CLI (≥ 26.x recommended).

CRATE       := trustbridge-contract
WASM_V1     := target/wasm32v1-none/release/$(CRATE).wasm
WASM_LEGACY := target/wasm32-unknown-unknown/release/$(CRATE).wasm
STELLAR     ?= stellar
SOURCE      ?= default
NETWORK     ?= testnet
ADMIN       ?= $(shell $(STELLAR) keys address $(SOURCE) 2>/dev/null || echo "")
CONTRACT_ID ?=
GITHUB_USER ?=
STELLAR_ADDR ?=

.PHONY: help build build-legacy test fmt lint check ci clean deploy-testnet deploy-mainnet \
        invoke-register invoke-lookup invoke-init invoke-stats install-target

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-22s\033[0m %s\n", $$1, $$2}'

install-target: ## Install wasm compilation targets
	rustup target add wasm32v1-none wasm32-unknown-unknown

build: install-target ## Build optimized WASM via Stellar CLI (recommended)
	$(STELLAR) contract build

build-legacy: install-target ## Build with cargo directly (wasm32-unknown-unknown)
	cargo build --target wasm32-unknown-unknown --release

test: ## Run unit tests
	cargo test

fmt: ## Check formatting
	cargo fmt --all -- --check

lint: ## Run clippy
	cargo clippy --all-targets -- -D warnings

check: fmt lint test build ## Run full local quality gate

ci: check ## Alias for CI-equivalent checks

clean: ## Remove build artifacts
	cargo clean
	rm -rf target/wasm32v1-none target/wasm32-unknown-unknown

deploy-testnet: build ## Deploy to Stellar Testnet
	NETWORK=testnet ADMIN=$(ADMIN) ./scripts/deploy.sh

deploy-mainnet: build ## Deploy to Stellar Mainnet (requires explicit ADMIN)
	@if [ -z "$(ADMIN)" ]; then echo "Set ADMIN to the G-address of the contract admin."; exit 1; fi
	NETWORK=mainnet ADMIN=$(ADMIN) ./scripts/deploy.sh

invoke-init: ## Initialize contract (CONTRACT_ID and ADMIN required)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		--send=yes \
		-- initialize --admin $(ADMIN)

invoke-register: ## Register a GitHub username (GITHUB_USER, STELLAR_ADDR, CONTRACT_ID)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		--send=yes \
		-- register \
		--github-username $(GITHUB_USER) \
		--stellar-address $(STELLAR_ADDR)

invoke-lookup: ## Look up a GitHub username (read-only simulation)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		-- get_address --github-username $(GITHUB_USER)

invoke-stats: ## Read registry statistics (read-only)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		-- get_stats
