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
- `get_stats` — total and verified registration counts
- `pause` / `unpause` / `is_paused` — emergency circuit breaker to pause mutating contract state
- `set_role` / `remove_role` / `get_role` — Role-Based Access Control (`Admin`, `Upgrader`, `Verifier`)
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
│   ├── lib.rs          # Contract implementation + unit tests
│   ├── storage.rs      # Storage keys, Role enum, accessors
│   ├── events.rs       # Contract event definitions (Registered, Verified, Upgraded, Paused, Role, etc.)
│   └── error.rs        # ContractError enum (includes Paused, CooldownActive, etc.)
├── tests/
│   └── integration.rs  # End-to-end integration test suite & event tracking
├── scripts/
│   └── deploy.sh       # Network-aware deploy + initialize script
├── docs/
│   ├── ARCHITECTURE.md # Design, storage, auth, events
│   ├── ABI.md          # Function & event reference
│   ├── DEPLOYMENT.md   # Testnet/mainnet deployment guide
│   ├── REGISTRY_INVARIANTS.md # Invariants and the property fuzzing suite
│   └── CONTRIBUTING.md # How to contribute
├── .github/workflows/
│   └── ci.yml          # fmt, clippy, test, contract build
├── Makefile            # build, test, deploy, invoke targets
├── Cargo.toml
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
step. Seeds are fixed constants, so failures replay deterministically.

> **Note on WASM targets:** `soroban-sdk` 26.x requires the `wasm32v1-none` target. Building with `wasm32-unknown-unknown` on Rust 1.82+ is unsupported by the Soroban environment. The release profile uses `opt-level = "z"` and `lto = true` as specified in `Cargo.toml`.

---

## Deploy to Testnet

Full walkthrough: [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md)

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
| `revoke_verification(caller, github_username)` | Admin **or** `Verifier`-role | ✅ | Clear a verification |
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

> **`remove` and Soroban auth:** Soroban requires an explicit `caller` address argument so the contract can validate which identity signed the transaction. The caller must equal either the registered Stellar address or the contract admin.

---

## Documentation Index

| Document | Description |
|----------|-------------|
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Storage layout, auth model, event design, data flow |
| [docs/ABI.md](docs/ABI.md) | Complete function, event, and error reference |
| [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) | Testnet/mainnet deployment, env vars, troubleshooting |
| [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) | Development workflow, PR guidelines, code standards |
| [docs/SECURITY.md](docs/SECURITY.md) | Threat model and security considerations |
| [docs/STORAGE_RENT.md](docs/STORAGE_RENT.md) | Storage rent economics, TTL management, keeper checklist |

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
