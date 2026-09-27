# TrustBridge Contract

[![CI](https://github.com/Stellar-TrustBridge/trustbridge-contract/actions/workflows/ci.yml/badge.svg)](https://github.com/Stellar-TrustBridge/trustbridge-contract/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Soroban SDK](https://img.shields.io/badge/soroban--sdk-26.0.1-blue)](https://crates.io/crates/soroban-sdk)

**trustbridge-contract** is the on-chain registry for [TrustBridge](https://github.com/Stellar-TrustBridge) — a permissionless Soroban smart contract on Stellar that maps **GitHub usernames** to **Stellar G-addresses**.

It replaces a centralized database with a decentralized, auditable source of truth used by the TrustBridge GitHub Action and dashboard.

---

## Table of Contents

- [Why This Exists](#why-this-exists)
- [Features](#features)
- [Architecture Overview](#architecture-overview)
- [Project Structure](#project-structure)
- [Quick Start](#quick-start)
- [Build & Test](#build--test)
- [Deploy to Testnet](#deploy-to-testnet)
- [Invoke via Stellar CLI](#invoke-via-stellar-cli)
- [Contract ABI Summary](#contract-abi-summary)
- [Documentation Index](#documentation-index)
- [License](#license)

---

## Why This Exists

Open-source contributors earn recognition and rewards through TrustBridge. To pay them on Stellar, the system must know which G-address belongs to which GitHub identity.

This contract provides that mapping **on-chain**:

| Property | Detail |
|----------|--------|
| **Permissionless registration** | Anyone can register their own GitHub username by proving ownership of a Stellar address |
| **Admin verification** | A designated admin or verifier can mark accounts as verified after off-chain GitHub checks (no on-chain proof is performed automatically) |
| **Transparent events** | Every registration, removal, and verification emits a Soroban contract event |
| **No central DB** | GitHub Actions and the dashboard read directly from the ledger |

---

## Features

- `initialize` — one-time admin setup and initial role assignment
- `register` — map GitHub username → Stellar address (requires address auth)
- `get_address` — read-only lookup
- `remove` — self-service or admin removal
- `verify` — admin or `Verifier`-role holder marks contributor as GitHub-verified
- `revoke_verification` — admin or `Verifier`-role holder revokes verified status
- `get_all_registered` — admin-only full export for dashboard sync
- `scripts/export_registry.sh` / `scripts/validate_registry.sh` — CLI export to JSON and validate-only diff against live state (see [Registry Export & Import](docs/DEPLOYMENT.md#registry-export--import))
- `scripts/trustbridge_client.py` — typed Python wrappers for operator reads and batch operations
- `get_stats` — total and verified registration counts
- `pause` / `unpause` / `is_paused` — emergency circuit breaker to pause mutating contract state
- `set_role` / `remove_role` / `get_role` — Role-Based Access Control (`Admin`, `Upgrader`, `Verifier`, `Revoker`) — see [ABI Role Matrix](docs/ABI.md#role-u32-discriminant) and [Architecture](docs/ARCHITECTURE.md#authorization-model) for details.
- `set_cooldown` / `get_cooldown` — WASM upgrade timelock cooldown period configuration
- `upgrade` — admin/upgrader executable WASM code replacement
- `migrate` / `get_version` — schema version migration harness and tracking

See the full [ABI reference](docs/ABI.md) for argument types, return values, and events.

---

## Architecture Overview

```
┌─────────────────┐     register / lookup      ┌──────────────────────────┐
│  Contributor    │ ─────────────────────────► │  trustbridge-contract    │
│  (GitHub user)  │                            │  (Soroban on Stellar)    │
└─────────────────┘                            └────────────┬─────────────┘
                                                            │
         ┌──────────────────────────────────────────────────┼──────────────────────────┐
         │                                                  │                          │
         ▼                                                  ▼                          ▼
┌─────────────────┐                              ┌─────────────────┐        ┌─────────────────┐
│ GitHub Action   │  reads get_address             │ TrustBridge     │  reads │ Indexers /      │
│ (CI pipeline)   │  resolves payout address       │ Dashboard       │  stats │ Explorers       │
└─────────────────┘                              └─────────────────┘        └─────────────────┘
```

**Storage model** (see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for full detail):

| Key | Value |
|-----|-------|
| `Symbol("reg")` + `github_username` | `ContributorRecord { stellar_address, registered_at, verified }` |
| `Symbol("admin")` | Admin `Address` |
| `Symbol("count")` | Total registration count (`u32`) |
| `Symbol("vcount")` | Verified registration count (`u32`) |
| `Symbol("idx")` | Username index for admin export |
| `Symbol("pause")` | Emergency pause boolean state (`bool`) |
| `Symbol("cdown")` | WASM upgrade cooldown duration in seconds (`u64`) |
| `Symbol("lastupg")` | Timestamp of last WASM upgrade (`u64`) |
| `Symbol("ver")` | Contract schema version tuple (`(u32, u32, u32)`) |
| `Symbol("role")` + `Address` | Assigned user role enum (`Role`) |

---

## Project Structure

```
trustbridge-contract/
├── src/
│   ├── lib.rs              # Contract implementation + unit tests
│   ├── storage.rs          # Storage keys, types, accessors, TTL constants
│   ├── events.rs           # Contract event definitions (27 event types)
│   ├── error.rs            # ContractError enum (includes Paused, CooldownActive, etc.)
│   ├── domain.rs           # EventDomain for deployment identification (Issue #226)
│   ├── utils.rs            # Username canonicalization, validation helpers
│   ├── batch.rs            # Batch operations (verify, remove) with summaries
│   ├── audit.rs            # Audit logging & statistics
│   ├── merkle.rs           # Merkle tree for export attestations (Issue #216)
│   ├── version.rs          # Version parsing & compatibility checks
│   ├── staged_wasm.rs      # Staged WASM deployment (Issue #300)
│   ├── multisig_upgrade.rs # Multi-sig upgrade governance (Issue #301)
│   └── oracle_proof.rs     # Oracle-based verification proofs
├── tests/
│   ├── integration.rs      # End-to-end integration test suite & event tracking
│   ├── contract_error_codes.rs # Error code coverage tests
│   ├── event_replay.rs     # Event replay idempotency tests (Issue #135)
│   ├── cursor_pagination.rs # Opaque cursor pagination tests (Issue #215)
│   ├── pagination_parity.rs # Admin vs public pagination parity (Issue #294)
│   ├── extend_registry_ttl.rs # TTL extension tests
│   ├── merkle_export.rs    # Merkle export & proof tests
│   ├── cross_contract_register_deny.rs # Cross-contract auth tests
│   ├── registry_hole_policy.rs # Empty registry invariant tests
│   ├── homoglyph_corpus.rs # Username homoglyph attack tests
│   ├── username_case_fold.rs # Case-folding tests
│   ├── counter_proofs.rs   # Record existence proof tests
│   ├── repair_index.rs     # Index compaction tests
│   └── zero_address.rs     # Zero address rejection tests
├── scripts/
│   ├── deploy.sh           # Network-aware deploy + initialize
│   ├── event_indexer.sh    # Reference event indexer (Issue #288)
│   ├── export_registry.sh  # Page admin export to JSON snapshot
│   ├── export_registry.py  # Typed registry exporter (Python)
│   ├── validate_registry.sh # Diff export JSON against live state
│   ├── ttl_keeper.sh       # Walk index & bump persistent-entry TTLs
│   ├── bulk_verify.sh      # Batched verify from username list
│   ├── bulk_revoke.sh      # Batched revoke from username list
│   ├── simulate_pause.sh   # Exercise pause/unpause lifecycle
│   ├── futurenet_smoke_test.sh # End-to-end Futurenet smoke test
│   ├── storage_rent_estimator.py # Estimate on-chain storage entry counts
│   ├── trustbridge_client.py # Typed Python client for operator reads
│   ├── payout_allowlist.sh / payout_allowlist.py # Payout allowlist ops
│   ├── dr_test.sh          # Disaster recovery test (export/validate round-trip)
│   ├── generate_abi_json.py # Generate machine-readable ABI JSON
│   ├── check_changelog_abi.sh # Verify CHANGELOG/ABI consistency
│   └── check_bench_regression.sh # Benchmark regression detection
├── docs/
│   ├── README.md           # Documentation index
│   ├── ARCHITECTURE.md     # Design, storage, auth, events, data flow
│   ├── ABI.md              # Complete function, event, error reference
│   ├── DEPLOYMENT.md       # Testnet/mainnet deployment guide
│   ├── EVENT_INDEXING.md   # Event consumption, idempotency, lag detection
│   ├── DASHBOARD_SYNC.md   # Dashboard/indexer sync patterns
│   ├── STORAGE_KEYS.md     # Storage key inventory & collision analysis
│   ├── STORAGE_RENT.md     # Storage rent economics, TTL management
│   ├── STORAGE_RENT_ESTIMATOR.md # Storage rent estimator specification
│   ├── STORAGE_FOOTPRINT.md # Storage entry size analysis
│   ├── CONTRACT_HEALTH.md  # Health endpoint specification
│   ├── BENCHMARK_BUDGETS.md # CPU/memory budget baselines
│   ├── REGISTRY_INVARIANTS.md # Invariants & property fuzzing suite
│   ├── SECURITY.md         # Threat model & security considerations
│   ├── ADMIN_RUNBOOK.md    # Operational runbook for admins
│   ├── CONTRIBUTING.md     # Development workflow, PR guidelines
│   ├── FUTURENET_ONBOARDING.md # Futurenet deployment guide
│   ├── TESTNET_CHECKLIST.md # Testnet deployment checklist
│   ├── subgraph/
│   │   ├── schema.graphql  # GraphQL schema for subgraph indexing
│   │   └── README.md       # Subgraph deployment guide
│   ├── abi.json            # Machine-readable contract ABI
│   └── storage-rent-estimator.inputs.v1.json # Estimator input schema
├── .github/workflows/
│   └── ci.yml              # fmt, clippy, test, contract build, bench
├── Makefile                # build, test, deploy, invoke, bench targets
├── Cargo.toml
├── rust-toolchain.toml     # Pinned toolchain & wasm target
├── deny.toml               # Cargo deny configuration
├── mutants.toml            # Mutation testing configuration
├── wasm-hash.pin           # Release WASM SHA-256 pin
├── CHANGELOG.md
├── LICENSE
└── README.md
```

---

## Quick Start

### Prerequisites

| Tool | Version |
|------|---------|
| Rust | ≥ 1.84 (MSRV for `soroban-sdk` 26.x) |
| wasm target | `wasm32v1-none` (required for SDK 26+) |
| Stellar CLI | ≥ 26.x recommended |
| Node.js | v16+ recommended (for bindings & tests) |

```bash
# Install Rust targets
rustup target add wasm32v1-none

# Install Stellar CLI (pick one)
curl -fsSL https://github.com/stellar/stellar-cli/raw/main/install.sh | sh
# or: cargo install --locked stellar-cli@26.1.0

# Clone and enter the repo
git clone https://github.com/Stellar-TrustBridge/trustbridge-contract.git
cd trustbridge-contract
```

### Build & Test

```bash
make test          # Run unit tests
make fuzz          # Run the invariant property fuzzing suite
make bench         # Report CPU/memory cost per contract operation
make build         # Build optimized WASM (via stellar contract build)
make check         # fmt + clippy + test + build
make simulate-register CONTRACT_ID=$CONTRACT_ID STELLAR_ADDR=$STELLAR_ADDR
                   # Simulate register and print gas/fee fields (no --send)
```

The fuzzing suite drives randomized `register` / `verify` / `revoke_verification` /
`remove` sequences against an independent model of the registry and asserts the
invariants in [docs/REGISTRY_INVARIANTS.md](docs/REGISTRY_INVARIANTS.md) after every
step. Seeds come from the checked-in corpus `tests/fuzz/seeds.txt`, so failures
replay deterministically; override with `make fuzz FUZZ_SEEDS=0x1,0x2,0x3,0x4`
(at least 4 seeds). `make fuzz` fails if no fuzz tests executed, and CI runs it
on every PR.

### Devcontainer / Codespaces

The repository devcontainer installs Stellar CLI `26.1.0` and the required
`wasm32v1-none` target. After reopening the repository in the container, verify
the toolchain with `stellar --version` and
`rustup target list --installed | grep wasm32v1-none`.

> **Note on WASM targets:** `soroban-sdk` 26.x requires the `wasm32v1-none` target. Building with `wasm32-unknown-unknown` on Rust 1.82+ is unsupported by the Soroban environment. The release profile uses `opt-level = "z"` and `lto = true` as specified in `Cargo.toml`.

---

## Deploy to Testnet

Full walkthrough: [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md). After deployment,
follow the [Stellar Expert WASM verification recipe](docs/DEPLOYMENT.md#explorer-verification)
to compare the explorer's SHA-256 hash with `wasm-hash.pin`.

```bash
# 1. Create and fund a testnet account
stellar keys generate deployer --network testnet --fund
stellar keys use deployer

# 2. Set admin address (usually the same deployer or a multisig)
export ADMIN=$(stellar keys address deployer)

# 3. Build and deploy
make deploy-testnet

# 4. Record the contract ID from output / deployments/testnet.json
export CONTRACT_ID=$(jq -r .contract_id deployments/testnet.json)
```

---

## Invoke via Stellar CLI

Everything after `--` is passed to the contract's auto-generated CLI (derived from the embedded WASM schema).

### Initialize (done automatically by `deploy.sh`)

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account deployer \
  --network testnet \
  --send=yes \
  -- initialize --admin $ADMIN
```

### Register a GitHub username

The `--source-account` must correspond to the Stellar address being registered (it signs the auth payload).

```bash
make invoke-register \
  CONTRACT_ID=$CONTRACT_ID \
  GITHUB_USER=octocat \
  STELLAR_ADDR=G... \
  SOURCE=deployer
```

Or directly:

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account deployer \
  --network testnet \
  --send=yes \
  -- register \
  --github-username octocat \
  --stellar-address G...
```

### Look up an address (read-only, no `--send`)

```bash
make invoke-lookup CONTRACT_ID=$CONTRACT_ID GITHUB_USER=octocat
```

### Read statistics

```bash
make invoke-stats CONTRACT_ID=$CONTRACT_ID
```

More examples (verify, remove, admin export): [docs/ABI.md](docs/ABI.md)

---

## Contract ABI Summary

| Function | Auth | Mutates | Description |
|----------|------|---------|-------------|
| `initialize(admin)` | Deployer | ✅ | Set admin (once) |
| `register(github_username, stellar_address)` | `stellar_address` | ✅ | Register or update mapping |
| `get_address(github_username)` | None | ❌ | Lookup by username |
| `remove(caller, github_username)` | `caller` (registrant or admin) | ✅ | Remove a registration |
| `get_all_registered()` | Admin | ❌ | Export full registry |
| `verify(caller, github_username)` | Admin **or** `Verifier`-role | ✅ | Mark as GitHub-verified |
| `revoke_verification(caller, github_username)` | Admin **or** `Revoker`-role | ✅ | Clear a verification |
| `get_verified_count()` | None | ❌ | Verified registration count |
| `get_stats()` | None | ❌ | `{ total, verified }` |
| `version()` | None | ❌ | Deployed version as `(major, minor, patch)` |
| `is_compatible(major, minor, patch)` | None | ❌ | Client version handshake |

**Events:** `RegisteredEvent`, `RemovedEvent`, `VerifiedEvent`, `VerificationRevokedEvent` — see [docs/ABI.md](docs/ABI.md)

**Errors:** `AlreadyInitialized`, `NotInitialized`, `NotAuthorized`, `NotRegistered`, `AlreadyVerified`, `NotVerified`, `InvalidUsername`

> **Username validation:** `register` accepts 1 to 39 characters of alphanumerics, hyphens, and underscores, starting and ending alphanumeric. Anything else fails with `InvalidUsername` before auth is checked and before any write, so rejected calls leave the registry untouched. See [docs/SECURITY.md](docs/SECURITY.md#input-validation).

### TypeScript bindings

```bash
make bindings CONTRACT_ID=$CONTRACT_ID NETWORK=testnet
```

Generates a typed client package into `bindings/typescript` (git-ignored) from
the deployed WASM. Clients should call `is_compatible` at startup so a stale
client fails fast instead of on an unexpected ABI. Full walkthrough:
[docs/ABI.md](docs/ABI.md#typescript-bindings)

Generated bindings describe the interface of the WASM used to generate them;
they do not prove that a deployed instance exposes every generated method.
Check `is_compatible(1, 1, 0)` before using `batch_verify` when supporting older
deployments. The entry point was added in 1.1.0; bindings generated from a
newer WASM may expose it even when the deployed contract does not.

> **`remove` and Soroban auth:** Soroban requires an explicit `caller` address argument so the contract can validate which identity signed the transaction. The caller must equal either the registered Stellar address or the contract admin.

---

## Documentation Index

Full index: [docs/README.md](docs/README.md)

| Document | Description |
|----------|-------------|
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Storage layout, auth model, event design, data flow |
| [docs/ABI.md](docs/ABI.md) | Complete function, event, and error reference |
| [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) | Testnet/mainnet deployment, env vars, troubleshooting |
| [docs/EVENT_INDEXING.md](docs/EVENT_INDEXING.md) | Event consumption, idempotency, lag detection, domain separation |
| [docs/DASHBOARD_SYNC.md](docs/DASHBOARD_SYNC.md) | Dashboard/indexer sync patterns, paginated reads, re-verification |
| [docs/STORAGE_KEYS.md](docs/STORAGE_KEYS.md) | Storage key inventory, collision analysis, TTL behavior |
| [docs/STORAGE_RENT.md](docs/STORAGE_RENT.md) | Storage rent economics, TTL management, keeper checklist |
| [docs/STORAGE_RENT_ESTIMATOR.md](docs/STORAGE_RENT_ESTIMATOR.md) | Storage rent estimator specification & input schema |
| [docs/CONTRACT_HEALTH.md](docs/CONTRACT_HEALTH.md) | Health endpoint specification & monitoring |
| [docs/BENCHMARK_BUDGETS.md](docs/BENCHMARK_BUDGETS.md) | CPU/memory budget baselines & regression thresholds |
| [docs/REGISTRY_INVARIANTS.md](docs/REGISTRY_INVARIANTS.md) | Invariants & property fuzzing suite |
| [docs/SECURITY.md](docs/SECURITY.md) | Threat model and security considerations |
| [docs/ADMIN_RUNBOOK.md](docs/ADMIN_RUNBOOK.md) | Operational runbook for admins |
| [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) | Development workflow, PR guidelines, code standards |
| [docs/FUTURENET_ONBOARDING.md](docs/FUTURENET_ONBOARDING.md) | Futurenet deployment guide |
| [docs/TESTNET_CHECKLIST.md](docs/TESTNET_CHECKLIST.md) | Testnet deployment checklist |

---

## Contributing

We welcome contributions! Please read [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) before opening a PR.

```bash
make check    # Run the full local quality gate before submitting
```

---

## License

This project is licensed under the [MIT License](LICENSE).

Copyright © 2026 [Stellar-TrustBridge](https://github.com/Stellar-TrustBridge)

## Handsoff notes

<!-- handsoff-issue-307 -->
- #307: Formal audit prep pack: threat-model tests as code from SECURITY.md

<!-- handsoff-issue-361 -->
- #361: Add missing ContractError variants used by lib.rs

<!-- handsoff-issue-367 -->
- #367: Align ContributorRecord fields with callers

<!-- handsoff-issue-370 -->
- #370: Implement public admin-transfer entry points documented in ABI

<!-- handsoff-issue-376 -->
- #376: Consolidate role gates onto has_role_or_admin

<!-- handsoff-issue-377 -->
- #377: Fix stale ContractError rustdoc code table (codes 17+)

<!-- handsoff-issue-381 -->
- #381: Add integration tests for staged WASM flow
