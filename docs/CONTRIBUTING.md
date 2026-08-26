# Contributing to TrustBridge Contract

Thank you for your interest in contributing to **trustbridge-contract**! This guide covers setup, workflow, and standards.

Related docs: [README](../README.md) · [ARCHITECTURE](ARCHITECTURE.md) · [ABI](ABI.md) · [DEPLOYMENT](DEPLOYMENT.md) · [Testnet Checklist](TESTNET_CHECKLIST.md)

---

## Code of Conduct

Be respectful, constructive, and inclusive. Harassment and discrimination are not tolerated. Report issues to the maintainers via GitHub.

---

## Getting Started

### 1. Fork and clone

```bash
git clone https://github.com/YOUR_USERNAME/trustbridge-contract.git
cd trustbridge-contract
```

### 2. Install dependencies

```bash
rustup target add wasm32v1-none
# Stellar CLI (optional but recommended)
cargo install --locked stellar-cli@26.1.0
```

### 3. Verify the build

```bash
make check
```

This runs formatting, clippy, tests, and contract build — the same checks as CI.

---

## Development Workflow

1. Open an issue for significant changes (new functions, storage layout changes, breaking ABI changes)
2. Create a branch from `main`:

   ```bash
   git checkout -b feat/your-feature
   ```

3. Make changes following the code standards below
4. Run tests:

   ```bash
   make test
   cargo clippy --all-targets -- -D warnings
   cargo fmt --all
   ```

5. Open a pull request with a clear description and test plan

---

## Code Standards

### Rust / Soroban

- Match existing module structure (`lib.rs`, `storage.rs`, `events.rs`, `error.rs`)
- Use `ContractError` for recoverable errors; `require_auth()` for auth failures
- Add unit tests in `#[cfg(test)] mod test` within `lib.rs`
- Keep functions focused; storage helpers belong in `storage.rs`
- Document non-obvious auth or storage decisions inline

### Documentation

- Update `ABI.md` for any interface change
- Update `ARCHITECTURE.md` for storage or auth model changes
- Update `README.md` if user-facing behavior changes

### Rustdoc

Every public function and type in `src/` that integrators interact with must have a rustdoc summary. Follow these conventions:

- One-line summary first (sentence ending in `.`), then a blank line, then optional detail.
- `# Auth` section on any function that calls `require_auth()` — state exactly which identity must sign.
- `# Errors` section on every `Result`-returning function — list each `ContractError` variant that can be returned and when.
- Keep docs short and factual. Do not duplicate examples from `ABI.md`.

**Example:**

```rust
/// Registers or updates a GitHub username → Stellar address mapping.
///
/// # Auth
///
/// Requires auth from `stellar_address`.
///
/// # Errors
///
/// - [`ContractError::NotInitialized`] if `initialize` has not been called.
/// - [`ContractError::Paused`] if the contract is paused.
/// - [`ContractError::InvalidUsername`] if the username fails validation.
pub fn register(env: Env, github_username: String, stellar_address: Address) -> Result<(), ContractError> {
```

### Building the docs locally

```bash
# Open in browser (recommended for contributors)
cargo doc --no-deps --open

# Build only, no browser (useful in CI or headless environments)
cargo doc --no-deps
```

Docs are generated into `target/doc/trustbridge_contract/`. The `--no-deps` flag skips dependencies and keeps the output focused on the contract's own API.

To also include private items (useful when exploring internals):

```bash
cargo doc --no-deps --document-private-items --open
```

### CI doc build

`cargo doc --no-deps` runs as part of the CI quality gate. A doc build failure (broken intra-doc links, missing `# Errors` on `Result` functions, etc.) blocks merge just like a test failure.

---

## Commit Messages

Use clear, imperative subjects:

```text
Add batch lookup helper for dashboard sync
Fix verified count on address change re-registration
Update CI to Stellar CLI 26.1.0
```

---

## Clippy Lint Policy

The full plan is in `docs/CLIPPY_PEDANTIC_PLAN.md`. Summary:

- Default lint gate: `cargo clippy --all-targets -- -D warnings` (enforced in CI and `make lint`).
- Phase 1 pedantic lints are active as `warn` in `src/lib.rs`; they will be promoted to `deny` once the fix PRs land.
- Any `#[allow(clippy::*)]` attribute must have an inline comment explaining why. Bare allow attributes without rationale are rejected in code review.
- Do not add `#![allow(clippy::pedantic)]` or `#![allow(clippy::nursery)]` at the crate level — enable and fix lints individually per the phased plan.

---

## Testing Guidelines

Required test coverage for new behavior:

| Scenario | Expected |
|---|---|
| Register + lookup | Roundtrip returns correct record |
| Unauthorized removal | Non-owner gets `NotAuthorized` |
| Admin-only functions | Non-admin auth fails |
| Re-registration | Record updates; verified resets on address change |
| Stats | Counts increment/decrement correctly |

Run tests:

```bash
cargo test
```

### WASM Integration Tests

The contract includes a WASM upgrade + cooldown integration test in `tests/integration.rs` that exercises the upgrade path using a real WASM artifact. This test is gated behind the `wasm-test` feature flag and is not run by default.

**Running WASM tests locally:**

```bash
# Build the WASM artifact first (required for the test)
cargo build --target wasm32v1-none --release

# Run the WASM upgrade cooldown test
cargo test --features wasm-test --test integration -- \
  test_integration_wasm_upgrade_cooldown --nocapture --exact
```

This test is automatically run in CI as part of the `wasm-integration-test` job.

---

## Snapshot Stabilization Policy

### What are snapshots?

Soroban's test environment can emit ledger/env state dumps into `test_snapshots/`. These are machine-generated files that represent a serialized snapshot of the simulated Soroban environment after a test sequence runs.

### Are snapshots committed?

No. `test_snapshots/` is listed in `.gitignore` and is intentionally excluded from version control. This is deliberate:

- Snapshots are large, binary-adjacent files that produce high-noise diffs.
- They are fully reproducible by running `cargo test` locally.
- Committing them would cause spurious PR noise any time the SDK or env changes.

### When should you use snapshot-based tests?

Prefer assert-based tests in almost all cases:

| Use case | Recommended approach |
|---|---|
| Auth enforcement (`require_auth`) | `assert_eq!(result, Err(ContractError::NotAuthorized))` |
| Error codes | `assert_eq!(err.code(), 3)` |
| Storage round-trips | Direct `get_address` / `get_stats` assertions |
| Event emission | Assert on `env.events().all()` |
| Large `Env` state dumps | Snapshot acceptable as a last resort |

Only reach for snapshots when you need to capture a full environment dump that would be impractical to assert field-by-field (e.g. a complex multi-step migration sequence). If the assertion can be written explicitly, write it explicitly.

### How to refresh snapshots

If a code change legitimately alters snapshot output (e.g. a storage layout migration), regenerate them locally with:

```bash
cargo test -- --include-ignored
```

Or for a specific test:

```bash
cargo test <test_name> -- --include-ignored
```

The regenerated files will appear in `test_snapshots/`. They are not committed; each developer regenerates them from the current source.

### Reviewing snapshot diffs in PRs

Because `test_snapshots/` is gitignored, snapshot files will never appear in a PR diff. If a PR author mentions a snapshot change:

1. Check out the branch locally and run `cargo test`.
2. Inspect the generated files in `test_snapshots/` for unexpected storage key additions, removed fields, or size regressions.
3. If the snapshot change is intentional (schema migration, new field), the PR description must explain the change and include before/after output or a brief diff summary in prose.

### CI behavior

CI does not compare snapshots against a baseline — snapshots are gitignored and therefore not present in the checkout. CI validates correctness through `cargo test`, which must pass with zero failures. If your test relies on a snapshot file being present, the test must either regenerate it at the start of the run or be converted to an assert-based test.

**Summary:** assert-based tests are the default; snapshots are an escape hatch for full-env dumps; `test_snapshots/` stays gitignored; CI stays green via `cargo test`.

---

## Writing Contract Tests

This repo uses the Soroban SDK test host. Tests live in `src/lib.rs` inside `#[cfg(test)] mod test` for unit tests and in `tests/integration.rs` for integration tests.

### Test host setup

Every test starts with an `Env` and a deployed contract instance:

```rust
let env = Env::default();
let admin = Address::generate(&env);
let user = Address::generate(&env);
let contract_id = env.register(TrustBridgeContract, ());
env.as_contract(&contract_id, || {
    TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
});
```

`env.mock_all_auths()` disables signature checks for the current frame, so you can call admin-only entry points without constructing real key pairs:

```rust
env.mock_all_auths();
env.as_contract(&contract_id, || {
    TrustBridgeContract::register(env.clone(), username, user.clone()).unwrap();
});
```

When you need to test an auth failure, clear the mocked auths:

```rust
env.set_auths(&[]);
let result = client.try_register(&name, &other);
assert!(result.is_err());
```

### Common patterns in this repo

- Storage helpers live in `src/storage.rs`. Call them directly when you want to inspect or construct state without going through the public ABI.
- Events are asserted via `env.events().all()`. Compare against a fully populated event struct so topic symbols and data fields are pinned.
- Ledger control: `env.ledger().set_timestamp(ts)` is useful when an event payload includes a timestamp you want to assert exactly.

### Exemplar tests to study

| Test file | Test name | What it covers |
|---|---|---|
| `src/lib.rs` | `test_register_and_get_address_roundtrip` | Basic register + lookup |
| `src/lib.rs` | `test_removed_event_payload_is_complete` | Event topic + data shape |
| `src/lib.rs` | `test_register_transfer_requires_current_owner_auth` | Auth enforcement |
| `src/lib.rs` | `test_verifier_role_can_verify` | Role-based access |
| `tests/integration.rs` | `test_integration_full_registry_lifecycle_and_events` | End-to-end lifecycle |

### Snapshot policy

Soroban may emit snapshot files under `test_snapshots/` when an event or type layout changes. These files are gitignored. If `cargo test` fails with a snapshot mismatch after an intentional ABI change, delete the stale snapshot and re-run — the test suite will regenerate it.

---

## Pull Request Checklist

- [ ] `make check` passes locally
- [ ] New behavior has unit tests
- [ ] Documentation updated (ABI, Architecture, README as applicable)
- [ ] No secrets or `.env` files committed
- [ ] PR description explains why the change is needed

---

## Testnet Deployment Checklist

Before promoting to futurenet or mainnet, run the testnet smoke checklist:

```bash
make testnet-checklist
```

See `TESTNET_CHECKLIST.md` for the full numbered steps.

GitHub Actions runs on every push and PR to `main`, `master`, and `develop`:

- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`
- `stellar contract build`

See `.github/workflows/ci.yml`.

---

## Reporting Security Issues

Do not open public issues for security vulnerabilities. See `SECURITY.md` for responsible disclosure instructions.

---

## Questions?

- Open a GitHub Discussion or Issue
- Review `ARCHITECTURE.md` for design context
- Check Stellar Soroban docs

We appreciate your contributions to decentralized open-source identity on Stellar!