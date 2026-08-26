# TrustBridge Contract — Makefile
#
# Common tasks for building, testing, and deploying the Soroban registry contract.
# Requires: Rust (≥ 1.84), wasm target, Stellar CLI (≥ 26.x recommended).

CRATE       := trustbridge-contract
WASM_CRATE  := $(subst -,_,$(CRATE))
WASM_V1     := target/wasm32v1-none/release/$(WASM_CRATE).wasm
WASM_LEGACY := target/wasm32-unknown-unknown/release/$(WASM_CRATE).wasm
STELLAR     ?= stellar
SOURCE      ?= default
NETWORK     ?= testnet
ADMIN       ?= $(shell $(STELLAR) keys address $(SOURCE) 2>/dev/null || echo "")
CONTRACT_ID ?=
GITHUB_USER ?=
STELLAR_ADDR ?=
CALLER      ?=
BENCH_OUT   ?= bench-results.txt
NORM_BENCH_OUT ?= bench-username-normalization.txt
REGISTER_BUDGET_CPU_MAX ?= 25000000
REGISTER_BUDGET_MEM_MAX ?= 300000
BINDINGS_DIR ?= bindings/typescript
PKG_MANAGER  ?= pnpm
EXPORT_FILE ?= registry-export-$(NETWORK).json
ADMIN_SOURCE ?=
WASM_SIZE_LIMIT ?= 204800

.PHONY: help build build-legacy test fuzz bench bench-export bench-username bench-double-verify bench-register-budget fmt lint docs docs-check check ci clean \
        deploy-testnet deploy-mainnet bindings bindings-build invoke-version require-contract-id \
        invoke-register invoke-lookup invoke-init invoke-stats install-target invoke-extend-ttl \
        export-registry validate-registry

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-25s\033[0m %s\n", $$1, $$2}'

install-target: ## Install wasm compilation targets
	rustup target add wasm32v1-none wasm32-unknown-unknown

build: install-target ## Build optimized WASM via Stellar CLI (recommended)
	$(STELLAR) contract build

build-legacy: install-target ## Build with cargo directly (wasm32-unknown-unknown)
	cargo build --target wasm32-unknown-unknown --release

test: ## Run unit tests
	cargo test

fuzz: ## Run the invariant property fuzzing suite (deterministic seeds)
	cargo test fuzz -- --nocapture

bench: ## Report CPU/memory cost per contract operation
	cargo test bench -- --nocapture --test-threads=1

bench-export: ## Write export CPU benchmark results to $(BENCH_OUT)
	cargo test test_bench_export -- --nocapture --test-threads=1 | tee $(BENCH_OUT)
	@echo "Benchmark results written to $(BENCH_OUT)"

bench-username: ## Write username case-normalization benchmark results to $(NORM_BENCH_OUT)
	cargo test test_bench_username_case_normalization -- --nocapture --test-threads=1 | tee $(NORM_BENCH_OUT)
	@echo "Benchmark results written to $(NORM_BENCH_OUT)"

bench-double-verify: ## Report CPU/memory cost of double-verify rejection vs successful verify
	cargo test test_bench_double_verify_rejection -- --nocapture --test-threads=1

bench-register-budget: ## Validate register cost stays under CPU/memory thresholds (baseline + max-length username)
	@echo "Running register budget sampling (CPU<=$(REGISTER_BUDGET_CPU_MAX), MEM<=$(REGISTER_BUDGET_MEM_MAX))"
	@cargo test test_report_register_budget_samples -- --nocapture --test-threads=1 | \
	awk -F',' -v cpu_max=$(REGISTER_BUDGET_CPU_MAX) -v mem_max=$(REGISTER_BUDGET_MEM_MAX) '\
	BEGIN { baseline=0; stressed=0; failed=0 } \
	/^register,(baseline|max_username_len),/ { \
	  input=$$2; cpu=$$3+0; mem=$$4+0; \
	  if (input=="baseline") baseline=1; \
	  if (input=="max_username_len") stressed=1; \
	  if (cpu > cpu_max || mem > mem_max) { \
	    failed=1; \
	    printf("ERROR: register budget exceeded for input=%s (cpu=%d, mem=%d, limits cpu<=%d mem<=%d)\n", input, cpu, mem, cpu_max, mem_max); \
	  } \
	} \
	END { \
	  if (!baseline || !stressed) { \
	    print "ERROR: register budget output missing baseline or max_username_len sample"; \
	    exit 2; \
	  } \
	  if (failed) exit 1; \
	  print "OK: register budget samples are within configured thresholds"; \
	}'

fmt: ## Check formatting
	cargo fmt --all -- --check

lint: ## Run clippy
	cargo clippy --all-targets -- -D warnings

docs: ## Build rustdoc for public API (opens in browser)
	cargo doc --no-deps --open

docs-check: ## Build rustdoc without opening browser (CI-equivalent)
	RUSTDOCFLAGS="-D warnings" cargo doc --no-deps

wasm-size: build ## Report release WASM size and check against budget (WASM_SIZE_LIMIT)
	@if [ -f $(WASM_V1) ]; then \
		WASM=$(WASM_V1); \
	elif [ -f $(WASM_LEGACY) ]; then \
		WASM=$(WASM_LEGACY); \
	else \
		echo "ERROR: No WASM artifact found. Run 'make build' first."; exit 1; \
	fi; \
	SIZE=$$(wc -c < "$$WASM"); \
	LIMIT=$(WASM_SIZE_LIMIT); \
	LIMIT_KB=$$(( LIMIT / 1024 )); \
	SIZE_KB=$$(( SIZE / 1024 )); \
	echo "──────────────────────────────────────────"; \
	echo "  WASM size report"; \
	echo "──────────────────────────────────────────"; \
	echo "  File   : $$WASM"; \
	echo "  Size   : $$SIZE bytes (~$${SIZE_KB} KB)"; \
	echo "  Limit  : $$LIMIT bytes ($${LIMIT_KB} KB)"; \
	echo "──────────────────────────────────────────"; \
	if [ "$$SIZE" -gt "$$LIMIT" ]; then \
		echo ""; \
		echo "FAIL: WASM size $$SIZE bytes exceeds budget $$LIMIT bytes (over by $$(( SIZE - LIMIT )) bytes)"; \
		echo "Raise WASM_SIZE_LIMIT in Makefile and .github/workflows/ci.yml if growth is intentional."; \
		exit 1; \
	else \
		echo "  Headroom: $$(( LIMIT - SIZE )) bytes remaining"; \
		echo ""; \
		echo "PASS: WASM size is within budget."; \
	fi

check: fmt lint test build docs-check wasm-size ## Run full local quality gate

wasm-hash-pin: build ## Verify release WASM hash matches wasm-hash.pin (mirrors CI hash gate)
	@if [ -f $(WASM_V1) ]; then WASM=$(WASM_V1); elif [ -f $(WASM_LEGACY) ]; then WASM=$(WASM_LEGACY); else echo "ERROR: No WASM artifact found. Run 'make build' first."; exit 1; fi; \
	ACTUAL=$$(sha256sum "$$WASM" | awk '{print $$1}'); \
	echo "WASM SHA-256: $$ACTUAL"; \
	PINNED=$$(grep -v '^#' wasm-hash.pin | grep -v '^$$' | tr -d '[:space:]'); \
	if [ "$$PINNED" = "PLACEHOLDER" ]; then \
		echo "WARNING: wasm-hash.pin contains PLACEHOLDER — run 'make wasm-hash-update' to pin."; \
	elif [ "$$ACTUAL" != "$$PINNED" ]; then \
		echo "ERROR: WASM hash mismatch! Expected: $$PINNED  Actual: $$ACTUAL"; \
		echo "Run 'make wasm-hash-update' if this change is intentional."; \
		exit 1; \
	else \
		echo "OK: WASM hash matches pin."; \
	fi

wasm-hash-update: build ## Recompute and update wasm-hash.pin with the current build hash
	@if [ -f $(WASM_V1) ]; then WASM=$(WASM_V1); elif [ -f $(WASM_LEGACY) ]; then WASM=$(WASM_LEGACY); else echo "ERROR: No WASM artifact found. Run 'make build' first."; exit 1; fi; \
	HASH=$$(sha256sum "$$WASM" | awk '{print $$1}'); \
	sed -i "s/^PLACEHOLDER$$/$$HASH/" wasm-hash.pin; \
	sed -i "s/^[a-f0-9]\{64\}$$/$$HASH/" wasm-hash.pin; \
	echo "Updated wasm-hash.pin to $$HASH"

ci: check ## Alias for CI-equivalent checks (fmt + lint + test + build + docs + wasm-size)

clean: ## Remove build artifacts
	cargo clean
	rm -rf target/wasm32v1-none target/wasm32-unknown-unknown $(BINDINGS_DIR)

bindings: ## Generate the TypeScript bindings package (CONTRACT_ID required)
	@if [ -z "$(CONTRACT_ID)" ]; then \
		echo "Set CONTRACT_ID=<C...> to generate bindings."; exit 1; \
	fi
	$(STELLAR) contract bindings typescript \
		--network $(NETWORK) \
		--contract-id $(CONTRACT_ID) \
		--output-dir $(BINDINGS_DIR) \
		--overwrite

bindings-build: bindings ## Generate and build the TypeScript bindings package
	cd $(BINDINGS_DIR) && $(PKG_MANAGER) install && $(PKG_MANAGER) run build

deploy-testnet: build ## Deploy to Stellar Testnet
	NETWORK=testnet ADMIN=$(ADMIN) ./scripts/deploy.sh

deploy-mainnet: build ## Deploy to Stellar Mainnet (requires explicit ADMIN and CONFIRM_MAINNET=yes)
	@if [ -z "$(ADMIN)" ]; then echo "Set ADMIN to the G-address of the contract admin."; exit 1; fi
	@if [ "$(CONFIRM_MAINNET)" != "yes" ]; then echo "ERROR: CONFIRM_MAINNET=yes is required for mainnet deployment to prevent accidental mainnet deploys."; exit 1; fi
	NETWORK=mainnet ADMIN=$(ADMIN) ./scripts/deploy.sh

require-contract-id:
	@if [ -z "$(CONTRACT_ID)" ]; then \
		echo "ERROR: set CONTRACT_ID=<C...> for this target."; exit 1; \
	fi

invoke-init: require-contract-id ## Initialize contract (CONTRACT_ID and ADMIN required)
	@if [ -z "$(ADMIN)" ]; then \
		echo "ERROR: set ADMIN to the G-address of the contract admin."; exit 1; \
	fi
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		--send=yes \
		-- initialize --admin $(ADMIN)

invoke-register: require-contract-id ## Register a GitHub username (GITHUB_USER, STELLAR_ADDR, CONTRACT_ID)
	@if [ -z "$(GITHUB_USER)" ]; then \
		echo "ERROR: set GITHUB_USER=<username> for this target."; exit 1; \
	fi
	@if [ -z "$(STELLAR_ADDR)" ]; then \
		echo "ERROR: set STELLAR_ADDR=<G...> for this target."; exit 1; \
	fi
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		--send=yes \
		-- register \
		--github-username $(GITHUB_USER) \
		--stellar-address $(STELLAR_ADDR)

invoke-lookup: require-contract-id ## Look up a GitHub username (read-only simulation)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		-- get_address --github-username $(GITHUB_USER)

invoke-version: require-contract-id ## Read the deployed contract version (read-only)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		-- version

invoke-stats: require-contract-id ## Read registry statistics (read-only)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		-- get_stats

BULK_VERIFY_FILE ?= usernames.txt
BULK_VERIFY_LOG  ?= bulk-verify-audit.log
BULK_VERIFY_PACE ?= 500

bulk-verify-dry-run: require-contract-id ## Dry-run bulk verify from BULK_VERIFY_FILE (no transactions submitted)
	@echo "=== Dry-run bulk verify from $(BULK_VERIFY_FILE) ==="
	@bash scripts/bulk_verify.sh \
		--file $(BULK_VERIFY_FILE) \
		--contract $(CONTRACT_ID) \
		--source $(SOURCE) \
		--network $(NETWORK) \
		--dry-run \
		--pace-ms $(BULK_VERIFY_PACE)

bulk-verify: require-contract-id ## Bulk verify from BULK_VERIFY_FILE with audit log and pacing
	@bash scripts/bulk_verify.sh \
		--file $(BULK_VERIFY_FILE) \
		--contract $(CONTRACT_ID) \
		--source $(SOURCE) \
		--network $(NETWORK) \
		--audit-log $(BULK_VERIFY_LOG) \
		--continue-on-error \
		--pace-ms $(BULK_VERIFY_PACE)

invoke-verify: ## Mark a contributor as verified (admin-only) (GITHUB_USER, SOURCE=admin, CONTRACT_ID)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		--send=yes \
		-- verify --caller $(CALLER) --github-username $(GITHUB_USER)

BULK_REVOKE_FILE ?= usernames.txt
BULK_REVOKE_LOG  ?= bulk-revoke-audit.log

bulk-revoke-dry-run: require-contract-id ## Dry-run bulk revoke from BULK_REVOKE_FILE (no transactions submitted)
	@echo "=== Dry-run bulk revoke from $(BULK_REVOKE_FILE) ==="
	@bash scripts/bulk_revoke.sh \
		--file $(BULK_REVOKE_FILE) \
		--contract $(CONTRACT_ID) \
		--source $(SOURCE) \
		--network $(NETWORK) \
		--dry-run

bulk-revoke: require-contract-id ## Bulk revoke from BULK_REVOKE_FILE with audit log (--yes skips confirm, add CONFIRM=yes for mainnet)
	@bash scripts/bulk_revoke.sh \
		--file $(BULK_REVOKE_FILE) \
		--contract $(CONTRACT_ID) \
		--source $(SOURCE) \
		--network $(NETWORK) \
		--audit-log $(BULK_REVOKE_LOG) \
		--continue-on-error \
		$(if $(filter yes,$(CONFIRM)),--yes,)

invoke-revoke-verification: ## Revoke contributor verification (admin-only) (GITHUB_USER, SOURCE=admin, CONTRACT_ID)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		--send=yes \
		-- revoke_verification --caller $(CALLER) --github-username $(GITHUB_USER)

invoke-get-all-registered: ## Export full registry mapping (admin-only) (SOURCE=admin, CONTRACT_ID)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		-- get_all_registered

invoke-export-paginated: ## Export paginated records with cursor (admin-only) (CURSOR, LIMIT, SOURCE=admin, CONTRACT_ID)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		-- get_registered_paginated --cursor $(CURSOR) --limit $(LIMIT)

invoke-public-paginated: ## Public paginated read for indexer/dashboard (CURSOR, LIMIT, CONTRACT_ID)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		-- get_public_paginated --cursor $(CURSOR) --limit $(LIMIT)

invoke-remove: ## Remove a registration (CALLER, GITHUB_USER, CONTRACT_ID)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		--send=yes \
		-- remove --caller $(CALLER) --github-username $(GITHUB_USER)

invoke-set-paused: ## Toggle contract pause state (PAUSED, SOURCE=admin, CONTRACT_ID)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		--send=yes \
		-- set_paused --paused $(PAUSED)

export-registry: require-contract-id ## Export full registry to JSON (admin) — see docs/DEPLOYMENT.md#registry-export--import (SOURCE=admin, CONTRACT_ID, EXPORT_FILE)
	CONTRACT_ID=$(CONTRACT_ID) SOURCE=$(SOURCE) NETWORK=$(NETWORK) OUTPUT_FILE=$(EXPORT_FILE) ./scripts/export_registry.sh

validate-registry: require-contract-id ## Validate a registry export JSON against live state, no writes (CONTRACT_ID, EXPORT_FILE, ADMIN_SOURCE=admin for full diff)
	CONTRACT_ID=$(CONTRACT_ID) SOURCE=$(SOURCE) ADMIN_SOURCE=$(ADMIN_SOURCE) NETWORK=$(NETWORK) ./scripts/validate_registry.sh $(EXPORT_FILE)
