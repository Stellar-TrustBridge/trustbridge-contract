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
CONTRACT_ID ?= $(CONTRACT)
GITHUB_USER ?=
STELLAR_ADDR ?=
CALLER      ?=
THRESHOLD   ?=
USERNAMES   ?=
FUZZ_SEEDS  ?=
FUZZ_RUNS   ?= 1000
BENCH_OUT   ?= bench-results.txt
NORM_BENCH_OUT ?= bench-username-normalization.txt
REGISTER_BUDGET_CPU_MAX ?= 25000000
REGISTER_BUDGET_MEM_MAX ?= 300000
BINDINGS_DIR ?= bindings/typescript
PKG_MANAGER  ?= pnpm
EXPORT_FILE ?= registry-export-$(NETWORK).json
ADMIN_SOURCE ?=
WASM_SIZE_LIMIT ?= 204800
# Set to 1 (as CI does) to fail `wasm-hash-pin` while wasm-hash.pin is still PLACEHOLDER.
WASM_HASH_STRICT ?= 0
FUTURENET_RPC_URL ?= https://rpc-futurenet.stellar.org
FUTURENET_FRIENDBOT_URL ?= https://friendbot-futurenet.stellar.org
FUTURENET_IDENTITY ?= $(SOURCE)
FUTURENET_DRY_RUN ?= false

.PHONY: help build build-legacy test test-rehearsal fuzz fuzz-parser storage-keys-check bindings-golden bench bench-export bench-username bench-double-verify bench-register-budget bench-budget-ci bench-update-samples fmt lint docs docs-check abi check ci clean \
        deploy-testnet deploy-mainnet bindings bindings-build invoke-version require-contract-id \
        invoke-register invoke-lookup invoke-init invoke-stats install-target invoke-extend-ttl \
        invoke-verify invoke-revoke-verification invoke-get-all-registered invoke-export-paginated \
        invoke-public-paginated invoke-remove invoke-set-paused \
        invoke-batch-remove invoke-set-batch-remove-threshold invoke-get-batch-remove-threshold \
        invoke-propose-batch-remove invoke-execute-batch-remove invoke-cancel-batch-remove \
        invoke-get-pending-batch-remove \
        ttl-keeper \
	export-registry validate-registry dr-test futurenet-smoke assert-build \
	xdr-fixtures diff-test

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

test-rehearsal: build ## Run protocol-upgrade rehearsal (requires pre-built WASM)
	cargo test test_protocol_upgrade_rehearsal --features wasm-test -- --nocapture
	@echo "PASS: protocol-upgrade rehearsal completed — all getters survived simulated WASM upgrade"

test-scale: ## Run the opt-in 10k-user pagination boundary load test
	cargo test --test integration --features scale-test test_paginated_export_at_10k_users -- --nocapture --test-threads=1

xdr-fixtures: ## Generate XDR fixtures for differential tests
	cargo test --test generate_xdr_fixtures -- --ignored --nocapture --exact generate_xdr_fixtures

diff-test: ## Run TypeScript differential tests
	cd ts-differential-tests && npm install && npm test

fuzz: ## Run the invariant property fuzzing suite (seeds: tests/fuzz/seeds.txt or FUZZ_SEEDS=0x1,0x2,...)
	@out=$$(FUZZ_SEEDS="$(FUZZ_SEEDS)" cargo test --lib fuzz -- --nocapture 2>&1) || { echo "$$out"; exit 1; }; \
	echo "$$out"; \
	n=$$(echo "$$out" | sed -nE 's/^test result: ok\. ([0-9]+) passed.*/\1/p' | awk '{s+=$$1} END {print s+0}'); \
	if [ "$$n" -eq 0 ]; then echo "make fuzz: no fuzz tests executed" >&2; exit 1; fi; \
	echo "make fuzz: $$n fuzz tests passed"

fuzz-parser: ## Fuzz export cursor parsing (install cargo-fuzz + nightly; FUZZ_RUNS=1000 for CI smoke, 0 for continuous)
	cargo +nightly fuzz run export_cursor -- -runs=$(FUZZ_RUNS) -max_len=9 -timeout=5 -rss_limit_mb=1024

storage-keys-check: ## Fail if a storage.rs key is missing from docs/STORAGE_KEYS.md
	./scripts/check_storage_keys.sh

bindings-golden: ## Verify the get_address simulate golden fixture (UPDATE_GOLDEN=1 to regenerate)
	cargo test --test bindings_golden -- --nocapture

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

bench-budget-ci: ## Run all bench tests + regression check against ci/bench-samples.csv (mirrors CI bench-budget job)
	@echo "Running benchmark tests (--test-threads=1 for stable metering)..."
	@cargo test \
		test_bench_username_case_normalization \
		test_bench_export_cpu_cost \
		test_bench_double_verify_rejection \
		test_report_register_budget_samples \
		-- --nocapture --test-threads=1 2>/dev/null \
	| tee bench-output.txt
	@echo ""
	@echo "Checking regression against ci/bench-samples.csv..."
	@bash scripts/check_bench_regression.sh \
		--samples ci/bench-samples.csv \
		--threshold 15 \
		--hard-cpu-cap 25000000 \
		--hard-mem-cap 3000000 \
		< bench-output.txt

bench-update-samples: ## Regenerate ci/bench-samples.csv from current test measurements (run after intentional cost changes)
	@echo "Regenerating bench baselines — this will overwrite ci/bench-samples.csv."
	@echo "Review the diff before committing."
	@cargo test \
		test_bench_username_case_normalization \
		test_bench_export_cpu_cost \
		test_bench_double_verify_rejection \
		test_report_register_budget_samples \
		-- --nocapture --test-threads=1 2>/dev/null \
	| awk -F',' ' \
		/^operation,/ { if (!header_done) { print; header_done=1 } next } \
		/^[a-z_]+,[^,]+,[0-9]+,[0-9]+$$/ { print } \
	' > bench-update-tmp.csv
	@if [ ! -s bench-update-tmp.csv ]; then \
		echo "ERROR: no CSV lines captured — check that bench tests emit output."; \
		rm -f bench-update-tmp.csv; exit 1; \
	fi
	@echo "# TrustBridge contract — checked-in benchmark budget baselines." > ci/bench-samples.csv
	@echo "#" >> ci/bench-samples.csv
	@echo "# Format: operation,input_label,cpu_instructions,memory_bytes" >> ci/bench-samples.csv
	@echo "#" >> ci/bench-samples.csv
	@printf "# LAST UPDATED: %s  (soroban-sdk $$(grep 'soroban-sdk' Cargo.toml | head -1 | grep -oP '[0-9]+\.[0-9]+\.[0-9]+' | head -1), stable toolchain)\n" "$$(date +%Y-%m-%d)" >> ci/bench-samples.csv
	@cat bench-update-tmp.csv >> ci/bench-samples.csv
	@rm -f bench-update-tmp.csv
	@echo "Done. Updated ci/bench-samples.csv:"
	@cat ci/bench-samples.csv

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

abi: ## Generate the machine-readable ABI JSON artifact
	python3 scripts/generate_abi_json.py

abi-check: ## Fail if docs/abi.json is stale vs docs/ABI.md (no files written)
	python3 scripts/generate_abi_json.py --check

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

error-codes: ## Verify ContractError discriminants agree across enum, golden, and ABI.md (Issue #402)
	./scripts/check_error_codes.sh

event-topics: ## Verify the indexer's topic table matches src/events.rs (Issue #399)
	./scripts/check_event_topics.sh

check: fmt lint error-codes event-topics abi-check test build docs-check wasm-size ## Run full local quality gate

wasm-hash-pin: build ## Verify release WASM hash matches wasm-hash.pin (mirrors CI hash gate)
	@if [ -f $(WASM_V1) ]; then WASM=$(WASM_V1); elif [ -f $(WASM_LEGACY) ]; then WASM=$(WASM_LEGACY); else echo "ERROR: No WASM artifact found. Run 'make build' first."; exit 1; fi; \
	ACTUAL=$$(sha256sum "$$WASM" | awk '{print $$1}'); \
	echo "WASM SHA-256: $$ACTUAL"; \
	PINNED=$$(grep -v '^#' wasm-hash.pin | grep -v '^$$' | tr -d '[:space:]'); \
	if [ "$$PINNED" = "PLACEHOLDER" ]; then \
		echo "WARNING: wasm-hash.pin contains PLACEHOLDER — run 'make wasm-hash-update' to pin."; \
		if [ "$(WASM_HASH_STRICT)" = "1" ]; then echo "ERROR: WASM_HASH_STRICT=1 requires a real pinned hash."; exit 1; fi; \
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

ci: ## Alias for CI-equivalent checks (fmt + lint + test + build + docs + wasm-size + strict hash pin)
	$(MAKE) check WASM_HASH_STRICT=1

clean: ## Remove build artifacts
	cargo clean
	rm -rf target/wasm32v1-none target/wasm32-unknown-unknown $(BINDINGS_DIR)

bindings: ## Generate TypeScript bindings from WASM or CONTRACT_ID
	@if [ -n "$(WASM)" ]; then \
		$(STELLAR) contract bindings typescript \
			--wasm "$(WASM)" \
			--output-dir $(BINDINGS_DIR) \
			--overwrite; \
	elif [ -n "$(CONTRACT_ID)" ]; then \
		$(STELLAR) contract bindings typescript \
			--network $(NETWORK) \
			--contract-id $(CONTRACT_ID) \
			--output-dir $(BINDINGS_DIR) \
			--overwrite; \
	else \
		echo "Set WASM=<path> or CONTRACT_ID=<C...> to generate bindings."; exit 1; \
	fi

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
		echo "ERROR: set CONTRACT_ID=<C...> (or CONTRACT=<C...>) for this target."; exit 1; \
	fi

require-caller:
	@test -n "$(CALLER)" || { echo "ERROR: set CALLER=<G...> to the signing identity's address." >&2; exit 1; }

# SEND=no omits --send=yes so operators can simulate before submitting.
admin-invoke = $(STELLAR) contract invoke --id "$(CONTRACT_ID)" --source-account "$(SOURCE)" --network "$(NETWORK)" $(if $(filter yes,$(SEND)),--send=yes,) --

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

invoke-verify: require-contract-id require-caller ## Mark a contributor as verified (GITHUB_USER, CALLER, SOURCE=admin)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		$(if $(filter yes,$(SEND)),--send=yes,) \
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

invoke-revoke-verification: require-contract-id require-caller ## Revoke verification (GITHUB_USER, CALLER, REVOKE_REASON_CODE required)
	@test -n "$(REVOKE_REASON_CODE)" || { echo "ERROR: set REVOKE_REASON_CODE=1,2,3,4,5,6,99." >&2; exit 1; }
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		$(if $(filter yes,$(SEND)),--send=yes,) \
		-- revoke_verification --caller $(CALLER) --github-username $(GITHUB_USER) --reason-code $(REVOKE_REASON_CODE)

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

invoke-pause: require-contract-id ## Pause the contract (SOURCE=admin; PAUSE_REASON_CODE=1 by default)
	$(admin-invoke) pause --reason-code "$(PAUSE_REASON_CODE)"

invoke-unpause: require-contract-id ## Resume the contract (SOURCE=admin; UNPAUSE_REASON_CODE=4 by default)
	$(admin-invoke) unpause --reason-code "$(UNPAUSE_REASON_CODE)"

invoke-set-paused: require-contract-id ## Idempotent pause toggle (PAUSED=true|false, REASON_CODE required)
	@case "$(PAUSED)" in true|false) ;; *) echo "ERROR: set PAUSED=true or PAUSED=false." >&2; exit 1;; esac
	@test -n "$(REASON_CODE)" || { echo "ERROR: set REASON_CODE=1,2,3,4,99." >&2; exit 1; }
	$(admin-invoke) set_paused --paused "$(PAUSED)" --reason-code "$(REASON_CODE)"

invoke-set-guardian: require-contract-id ## Set the guardian (GUARDIAN_ADDRESS=<G...>, SOURCE=admin)
	@test -n "$(GUARDIAN_ADDRESS)" || { echo "ERROR: set GUARDIAN_ADDRESS=<G...>." >&2; exit 1; }
	$(admin-invoke) set_guardian --guardian "$(GUARDIAN_ADDRESS)"

invoke-remove-guardian: require-contract-id ## Remove the guardian (SOURCE=admin)
	$(admin-invoke) remove_guardian

invoke-emergency-pause: require-contract-id require-caller ## Emergency freeze (SOURCE=guardian or admin, CALLER=<signer G...>)
	$(admin-invoke) emergency_pause --caller "$(CALLER)"

invoke-clear-emergency-pause: require-contract-id ## Lift emergency freeze (SOURCE=admin)
	$(admin-invoke) clear_emergency_pause

invoke-set-role: require-contract-id ## Grant role (TARGET_ADDRESS=<G...>, ROLE=Verifier|Revoker|Upgrader|Admin)
	@test -n "$(TARGET_ADDRESS)" && test -n "$(ROLE)" || { echo "ERROR: set TARGET_ADDRESS=<G...> and ROLE=<variant>." >&2; exit 1; }
	$(admin-invoke) set_role --target "$(TARGET_ADDRESS)" --role "$(ROLE)"

invoke-remove-role: require-contract-id ## Revoke role (TARGET_ADDRESS=<G...>, SOURCE=admin)
	@test -n "$(TARGET_ADDRESS)" || { echo "ERROR: set TARGET_ADDRESS=<G...>." >&2; exit 1; }
	$(admin-invoke) remove_role --target "$(TARGET_ADDRESS)"

invoke-set-cooldown: require-contract-id ## Set upgrade cooldown (COOLDOWN_SECONDS=<seconds>, SOURCE=admin)
	@test -n "$(COOLDOWN_SECONDS)" || { echo "ERROR: set COOLDOWN_SECONDS=<seconds> (0 disables)." >&2; exit 1; }
	$(admin-invoke) set_cooldown --cooldown-seconds "$(COOLDOWN_SECONDS)"

invoke-adopt-network-tag: require-contract-id ## Tag an untagged legacy instance (SOURCE=admin)
	$(admin-invoke) adopt_network_tag

invoke-batch-remove: require-contract-id ## Directly remove a batch of registrations (CALLER, USERNAMES='["user1",...]', SOURCE=admin, CONTRACT_ID)
	@if [ -z "$(CALLER)" ]; then \
		echo "ERROR: set CALLER=<G...> for this target."; exit 1; \
	fi
	@if [ -z "$(USERNAMES)" ]; then \
		echo "ERROR: set USERNAMES='[\"user1\",\"user2\"]' for this target."; exit 1; \
	fi
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		--send=yes \
		-- batch_remove --caller $(CALLER) --usernames '$(USERNAMES)'

invoke-set-batch-remove-threshold: require-contract-id ## Set dual-control threshold for batch_remove (THRESHOLD, SOURCE=admin, CONTRACT_ID)
	@if [ -z "$(THRESHOLD)" ]; then \
		echo "ERROR: set THRESHOLD=<count> (0 to disable) for this target."; exit 1; \
	fi
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		--send=yes \
		-- set_batch_remove_threshold --threshold $(THRESHOLD)

invoke-get-batch-remove-threshold: require-contract-id ## Read configured dual-control batch_remove threshold (read-only)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		-- get_batch_remove_threshold

invoke-propose-batch-remove: require-contract-id ## Propose a dual-control batch removal (CALLER, USERNAMES='["user1",...]', SOURCE=admin, CONTRACT_ID)
	@if [ -z "$(CALLER)" ]; then \
		echo "ERROR: set CALLER=<G...> for this target."; exit 1; \
	fi
	@if [ -z "$(USERNAMES)" ]; then \
		echo "ERROR: set USERNAMES='[\"user1\",\"user2\"]' for this target."; exit 1; \
	fi
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		--send=yes \
		-- propose_batch_remove --caller $(CALLER) --usernames '$(USERNAMES)'

invoke-execute-batch-remove: require-contract-id ## Execute pending dual-control batch removal from second key (CALLER, SOURCE=second_key, CONTRACT_ID)
	@if [ -z "$(CALLER)" ]; then \
		echo "ERROR: set CALLER=<G...> for this target."; exit 1; \
	fi
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		--send=yes \
		-- execute_batch_remove --caller $(CALLER)

invoke-cancel-batch-remove: require-contract-id ## Cancel pending dual-control batch removal (CALLER, SOURCE=admin, CONTRACT_ID)
	@if [ -z "$(CALLER)" ]; then \
		echo "ERROR: set CALLER=<G...> for this target."; exit 1; \
	fi
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		--send=yes \
		-- cancel_batch_remove --caller $(CALLER)

invoke-get-pending-batch-remove: require-contract-id ## View pending dual-control batch removal proposal (read-only)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--source-account $(SOURCE) \
		--network $(NETWORK) \
		-- get_pending_batch_remove

dr-test: ## Run a non-destructive export/validate round-trip on a disposable instance
	CONTRACT_ID=$(CONTRACT_ID) SOURCE=$(SOURCE) ADMIN_SOURCE=$(ADMIN_SOURCE) \
		NETWORK=$(NETWORK) STELLAR=$(STELLAR) ./scripts/dr_test.sh

futurenet-smoke: ## Deploy and smoke-check Futurenet using pinned RPC and Friendbot endpoints
	ADMIN=$(ADMIN) SOURCE=$(SOURCE) IDENTITY=$(FUTURENET_IDENTITY) \
		RPC_URL=$(FUTURENET_RPC_URL) FRIENDBOT_URL=$(FUTURENET_FRIENDBOT_URL) \
		STELLAR=$(STELLAR) DRY_RUN=$(FUTURENET_DRY_RUN) ./scripts/futurenet_smoke_test.sh
TTL_KEEPER_BATCH_SIZE ?=
TTL_KEEPER_DRY_RUN ?= false

ttl-keeper: require-contract-id ## Extend TTL for all registry records — see docs/STORAGE_RENT.md#keeper-implementation (CONTRACT_ID, SOURCE, TTL_KEEPER_DRY_RUN=true to preview)
	CONTRACT_ID=$(CONTRACT_ID) SOURCE=$(SOURCE) NETWORK=$(NETWORK) STELLAR=$(STELLAR) \
		./scripts/ttl_keeper.sh \
		$(if $(filter true,$(TTL_KEEPER_DRY_RUN)),--dry-run,) \
		$(if $(TTL_KEEPER_BATCH_SIZE),--batch-size $(TTL_KEEPER_BATCH_SIZE),)

export-registry: require-contract-id ## Export full registry to JSON (admin) — see docs/DEPLOYMENT.md#registry-export--import (SOURCE=admin, CONTRACT_ID, EXPORT_FILE)
	CONTRACT_ID=$(CONTRACT_ID) SOURCE=$(SOURCE) NETWORK=$(NETWORK) OUTPUT_FILE=$(EXPORT_FILE) ./scripts/export_registry.sh

validate-registry: require-contract-id ## Validate a registry export JSON against live state, no writes (CONTRACT_ID, EXPORT_FILE, ADMIN_SOURCE=admin for full diff)
	CONTRACT_ID=$(CONTRACT_ID) SOURCE=$(SOURCE) ADMIN_SOURCE=$(ADMIN_SOURCE) NETWORK=$(NETWORK) ./scripts/validate_registry.sh $(EXPORT_FILE)

assert-build: require-contract-id ## Verify a locally built WASM hash matches stored provenance (WASM_HASH, CONTRACT_ID)
	@test -n "$(WASM_HASH)" || (echo "WASM_HASH is required — run 'stellar contract build' and hash the artifact" && exit 1)
	$(STELLAR) contract invoke \
		--id $(CONTRACT_ID) \
		--network $(NETWORK) \
		-- assert_build --hash $(WASM_HASH)
