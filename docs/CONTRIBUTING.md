# Contributing to TrustBridge Contract

Thank you for your interest in contributing to **trustbridge-contract**! This guide covers setup, workflow, and standards.

Related docs: [README](../README.md) · [ARCHITECTURE](ARCHITECTURE.md) · [ABI](ABI.md) · [CHANGELOG](../CHANGELOG.md) · [DEPLOYMENT](DEPLOYMENT.md) · [Testnet Checklist](TESTNET_CHECKLIST.md)

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

`make check` is the full local quality gate (`make fmt`, `make lint`,
`make test`, `make docs-check`, plus error-codes / event-topics / abi-check /
build / wasm-size). Every pull request and push to `main` runs these jobs from
`.github/workflows/ci.yml`; each one fails the workflow on its first error:

| CI job | Make targets |
|---|---|
| `quality` | `cargo test --test homoglyph_corpus` (named blocking step), then `make check` (`make fmt`, `make lint`, `make test`, `make docs-check`, plus error-codes / event-topics / abi-check / wasm-size) |
| `build` | `make build` (`stellar contract build`), `make wasm-size`, `make wasm-hash-pin WASM_HASH_STRICT=1` |
| `abi-consistency` | `./scripts/check_error_codes.sh` (`make error-codes`) and `./scripts/check_event_topics.sh` (`make event-topics`) — fails on any `ContractError` / event-topic drift |
| `oracle-proof-tests` | `cargo test --lib oracle_proof` |
| `cargo-deny` | `cargo deny --config deny.toml check` (see [Supply-chain checks](#supply-chain-checks)) |
| `mutants` | `cargo mutants --config mutants.toml --file src/utils.rs` — nightly and `workflow_dispatch` only (see [Mutation Testing](#mutation-testing--utilsrs-issue-243)) |

`make ci` runs the same set with the strict hash gate enabled, so it fails while
`wasm-hash.pin` still contains `PLACEHOLDER` or does not match the built WASM.

### Codespaces / devcontainer setup

The repository devcontainer installs the same pinned CLI and WASM target used
by the documented build flow: Stellar CLI `26.1.0` and `wasm32v1-none` —
plus the `make test` / `make fuzz` prerequisites (Python 3, `jq`, Node LTS).
No secrets are baked into the container config. After the container is
created, smoke-verify the setup with:

```bash
stellar --version
rustup target list --installed | grep wasm32v1-none
make test          # smoke-verify a fresh container
make build
```

The Rust channel remains controlled by `rust-toolchain.toml`; the container
uses the stable Rust image to match CI.

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
- Avoid module-wide `allow(dead_code)`. Wire required helpers into their intended
   call sites or remove obsolete code. For intentionally staged items, put a
   narrow `#[allow(dead_code)]` on the item and document the issue and why it
   remains staged.

### Documentation

- Update `ABI.md` for any interface change
- Update `CHANGELOG.md` with a new semantic version heading when a public
  function signature in `ABI.md` changes. Use a minor version for additive
  signatures, a major version for breaking changes, and a patch version for
  compatible corrections.
- Update `ARCHITECTURE.md` for storage or auth model changes
- Update `README.md` if user-facing behavior changes

### ABI changelog policy

CI compares `docs/ABI.md` with the pull request base revision. Changes to
public function signature headings require a new versioned `CHANGELOG.md`
heading, for example `## [1.2.0] - 2026-08-28`, with a description of the
change. Typo fixes and prose-only ABI edits do not require a changelog entry.
The check is intentionally a heuristic: it watches function signature
headings rather than trying to understand the complete ABI. If a legitimate
prose edit is reported as an ABI change, add a reason in the changed changelog
as an HTML comment:

```markdown
<!-- changelog-check: skip - explain why this signature-looking edit is not a public ABI change -->
```

Do not use the skip marker for an actual ABI change; record that change under a
new version instead. Run the same check locally with the base commit:

```bash
bash scripts/check_changelog_abi.sh main
```

### ABI JSON regeneration

`docs/abi.json` is generated from `docs/ABI.md` by
`scripts/generate_abi_json.py` — never hand-edit it. Regenerate it whenever
you touch a function signature heading, the `ContractError` table, or an
event topic/data block in `docs/ABI.md`:

```bash
make abi        # regenerate docs/abi.json
make abi-check  # fail with a diff if docs/abi.json or the public-entrypoint golden is stale
```

Public function names and arity are snapshotted in
`tests/testdata/abi/public_entrypoints.golden.json` (Issue #631). After an
intentional public interface change:

```bash
python3 scripts/generate_abi_json.py --update-entrypoints-golden
```

Commit that golden together with `docs/ABI.md` / `docs/abi.json` so reviewers
see the entrypoint list change in Git. Do not refresh it for documentation-only
edits. Error-enum codes remain in `abi/contract_error_codes.golden`.

`make abi-check` runs in `make check` and in the CI `quality` job, so a
release that skips regeneration fails the gate. Commit the regenerated
`docs/abi.json` alongside the `docs/ABI.md` change.

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

## Mutation Testing — utils.rs (Issue #243)

Username validation in `src/utils.rs` is a security boundary: it blocks
homoglyph attacks, enforces consecutive-hyphen rules, and rejects non-ASCII
bytes. Normal unit tests can miss a _weakened_ check (e.g. a hyphen guard
accidentally commented out). Mutation testing makes surviving mutants a CI
failure.

### How it works

[`cargo-mutants`](https://mutants.rs) applies small syntactic mutations to
`src/utils.rs` one at a time and re-runs the test suite. A mutant that passes
all tests "survives" — meaning the test suite did **not** catch the weakened
behaviour. The CI job fails when any mutant survives.

Configuration lives in `mutants.toml` at the repository root. The scope is
intentionally limited to `src/utils.rs`; the rest of the crate is excluded to
keep job time bounded.

The `mutants` job in `.github/workflows/ci.yml` runs nightly (03:00 UTC) and
on demand via **Actions → CI → Run workflow**; it does not run on pull
requests because a full run is long: about 60 mutants, each needing an
incremental build plus a test run (~18 minutes on an 8-core workstation;
expect 30–60 minutes on a GitHub-hosted runner, hence the 90-minute job
timeout). It fails when any mutant survives and uploads
`mutants.out/missed.txt` (the survived list) and `outcomes.json` as the
`mutants-survived` artifact. Each mutant is
tested against the `utils::` unit tests and `tests/homoglyph_corpus.rs` only
(`additional_cargo_test_args` in `mutants.toml`), which keeps runs short and
independent of unrelated test failures elsewhere in the crate.

### Running locally

```bash
# Install (one-time; same version as CI)
cargo install --locked cargo-mutants@27.1.0

# Run mutations on utils.rs only (same command as CI; --config is required
# because cargo-mutants otherwise only reads .cargo/mutants.toml)
cargo mutants --config mutants.toml --file src/utils.rs

# View survived mutants (cargo-mutants calls them "missed")
cat mutants.out/missed.txt
```

### Interpreting results

| Outcome | Meaning |
|---------|---------|
| **Caught** | Test suite detected the mutation — good |
| **Survived** | Test suite missed a weakened check — add a test |
| **Unviable** | Mutant caused a compile error — not a failure |
| **Timeout** | Mutant triggered an infinite loop — not a failure |

When a mutant survives, add a test that specifically exercises the mutated
line or branch. Do not simply add an assertion that happens to pass — the new
test must **fail** on the mutated code and **pass** on the original.

### Do not mutate test-only code

Test helpers and `#[cfg(test)]` modules are excluded in `mutants.toml`.
Mutating test code produces meaningless results (a mutation that breaks a test
is a bug in the test, not a gap in coverage).

### Why `src/error.rs` is excluded (Issue #434)

`src/error.rs` is an exhaustive discriminant table (`code()` /
`from_code()`), a total retry-classification match (`category()` /
`is_retryable()`), and derived impls — there is no branching logic for a
mutant to weaken. Mutating it yields brittle noise: discriminant swaps and
reordered `from_code` arms either fail to compile or are semantically
identical, and a `category()` re-label is already pinned by
`tests/contract_error_codes.rs` plus the `scripts/check_error_codes.sh`
golden gate (enum ↔ `from_code` ↔ `abi/contract_error_codes.golden` ↔
`docs/ABI.md`). `mutants.toml` therefore excludes `error.rs` boilerplate
(`src/error.rs`, `fn code`, `fn from_code`, `fn category`, `fn is_retryable`)
so mutation runs keep covering validation, storage, and business logic
instead of re-proving a mapping that the error-code gate already freezes.

---

## Coverage (Issue #248)

CI measures line coverage for `src/storage.rs` using `cargo-tarpaulin`. The
threshold is 60% line coverage; sustained drops below that level trigger a
review flag in the CI output. The job is non-blocking on PRs but annotates the
run with a pass/warning message.

### Running coverage locally

```bash
# Install tarpaulin (one-time)
cargo install --locked cargo-tarpaulin

# Coverage for storage.rs only
cargo tarpaulin --lib --include-files 'src/storage.rs'

# Full library coverage (slower)
cargo tarpaulin --lib

# Alternative: llvm-cov (requires nightly or llvm-tools-preview)
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov
cargo llvm-cov --lib
```

### dead_code hygiene policy

Every `#[allow(dead_code)]` attribute in `src/` must:

1. Have an inline comment explaining **why** it is staged and **what future
   work** will wire it in (or a reference to the tracking issue).
2. Be covered by at least one test in the same file's `#[cfg(test)]` module.

The crate-level `#![allow(dead_code)]` in `utils.rs` has been removed in favour
of per-item attributes (Issue #248). New crate-level dead_code suppressions are
rejected in code review.

---

## Fuzz Testing

The `fuzz/` directory contains `cargo-fuzz` targets that complement the
homoglyph corpus and unit tests with random byte-level exploration.

**Requires a nightly toolchain** — `cargo-fuzz` is a nightly-only tool.
Install it once:

```bash
rustup toolchain install nightly
cargo install --locked cargo-fuzz
```

### Available targets

| Target | Path | What it fuzzes |
|---|---|---|
| `export_cursor` | `fuzz/fuzz_targets/export_cursor.rs` | Paginated-export cursor parsing (`get_public_paginated`) |
| `username_validate` | `fuzz/fuzz_targets/username_validate.rs` | `is_valid_github_username` — no-panic guarantee |
| `github_username` | `fuzz/fuzz_targets/github_username.rs` | `is_valid_github_username` — non-ASCII rejection invariant |

### Running a target

```bash
# Run the github_username target for 1 000 iterations (smoke test)
cargo +nightly fuzz run github_username -- -max_total_time=10

# Longer soak (10 million inputs)
cargo +nightly fuzz run github_username -- -runs=10000000
```

The `github_username` target asserts two invariants on every input:

1. **No panics** — the validator must never trap.
2. **Non-ASCII rejection** — any byte ≥ 0x80 must cause the validator
   to return `false`.  This rules out homoglyph substitution via
   multi-byte UTF-8 (Cyrillic, Arabic, CJK, accented Latin, emoji).

### Adding a new target

1. Create `fuzz/fuzz_targets/<name>.rs`.
2. Add a `[[bin]]` entry in `fuzz/Cargo.toml` pointing to it.
3. Document the target in the table above.
4. Ensure the target contains an assertion (not just a no-panic check)
   so it provides more than the existing `username_validate` coverage.

---

## Supply-chain checks

The `cargo-deny` CI job runs `cargo deny --config deny.toml check` on every
pull request (including fork PRs) and push to `main`; any `deny`-level finding
fails the job. These catch:

- **Known vulnerabilities** in dependencies (via RustSec advisory database)
- **Yanked crates** that should not be in the dependency tree
- **License violations** against the allow-list in `deny.toml`
- **Untrusted sources** (unknown registries or git dependencies)

### Running locally

```bash
cargo install --locked cargo-deny@0.20.2
cargo deny --config deny.toml check
```

### Adding an exception

If a advisory is a false positive or cannot be patched upstream, add it to the
`ignore` list in `deny.toml` with a comment explaining why and when it expires.
Undocumented exceptions are rejected in code review.

---

## Lint Policy

- Local lint: `make lint` (`cargo clippy --all-targets -- -D warnings`).
- CI runs `cargo fmt --check` and `cargo test`.
- Prefer fixing clippy warnings locally before opening a PR.
- Any `#[allow(clippy::*)]` attribute should include a short rationale comment.

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

### Homoglyph corpus

The homoglyph corpus in `tests/homoglyph_corpus.rs` is a security regression
suite for lookalike Unicode characters, invisible formatting characters,
bidirectional controls, and any entry point that must reject them. Extend it
whenever a new confusable or spoofing vector is reported, discovered during
review, or introduced by a change to username handling. Keep the ASCII-only
acceptance cases as positive controls.

Add each attack string to the relevant corpus category with a brief description
and code point, and add an entry-point regression test when the vector could
expose a validation bypass. Update the documented corpus scope and expected
minimum in `test_homoglyph_corpus_coverage_complete` as needed. Do not normalize
or permit a new character as a local exception; changes to the ASCII-only
policy require an explicit security review.

Run the dedicated test before submitting changes:

```bash
cargo test --test homoglyph_corpus
# equivalent: make test-homoglyph
```

CI runs this target as a dedicated blocking job on every pull request and
push to `main`: the `homoglyph_corpus` job in `.github/workflows/ci.yml`
executes `cargo test --test homoglyph_corpus` (no path filters, no
`continue-on-error`), so a failing corpus test fails the `homoglyph_corpus`
check. Maintainers should mark `homoglyph_corpus` as a required status check
in branch protection. The `smoke` job's `cargo test` also includes this suite.

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

### Register instruction budget report

`test_report_register_budget_samples` (in `src/lib.rs`) measures the metered CPU
instructions and memory used by `register`, for a baseline username and a
max-length username, and prints them as CSV.

Run it locally, checked against the configured thresholds:

```bash
make bench-register-budget
```

Thresholds are configurable via `REGISTER_BUDGET_CPU_MAX` / `REGISTER_BUDGET_MEM_MAX`
make variables (see `Makefile`).

To reproduce and validate batch register size limits (`MAX_WRITE_BATCH = 25`) and metered
CPU/memory budget headroom locally:

```bash
make bench-batch-register-budget
# Or directly via cargo:
cargo test --test integration test_batch_register -- --nocapture --test-threads=1
# Optional extended multi-batch load test:
cargo test --test integration test_scale_batch_register --features scale-test -- --nocapture
```

In CI, the `quality` job runs this test, publishes the results as a job summary
table, and uploads the raw CSV as the `register-budget-report` build artifact
(14-day retention). This check is **advisory**: the step is allowed to fail
(`continue-on-error`) without failing the overall job, so a cap breach is
visible in the CI summary and artifact without blocking merges.

### Bindings golden refresh (simulate XDR fixtures)

`tests/bindings_golden.rs` pins the base64 XDR that `simulateTransaction`
returns for `get_address` (hit and miss) against the checked-in fixture
`tests/testdata/bindings/get_address_simulate.v1.json`. `trustbridge-action`
(`src/soroban.ts`) vendors that fixture instead of hand-mocking XDR, so an
intentional return-type change here must ship with a coordinated fixture
bump — see [`tests/testdata/bindings/README.md`](../tests/testdata/bindings/README.md)
for the fixture format and the action-side consumption steps.

To verify the fixture matches the current contract encoding:

```bash
make bindings-golden
# equivalent: cargo test --test bindings_golden
```

To regenerate after an intentional change to `get_address`'s return type or
the fixture format:

```bash
UPDATE_GOLDEN=1 cargo test --test bindings_golden
```

Then, in the same PR:

1. **Bump `FIXTURE_VERSION`** in `tests/bindings_golden.rs` and rename the
   fixture file to `get_address_simulate.vN.json` on **any breaking change**
   (return-type shape, `ScMap` key set/order, or field types). Additive-only
   changes that keep the existing decode path working keep the version.
2. **Update the TypeScript side.** `scripts/check_fixture_pairing.sh` fails
   when the Rust fixtures change without a matching `ts-differential-tests/`
   update (or the reverse), so land the decoder/fixture change in
   `ts-differential-tests/` in the same PR.
3. **Notify `trustbridge-action` consumers.** Open or update an issue in
   `Stellar-TrustBridge/trustbridge-action` so the pinned fixture version is
   bumped in the action's tests in the same release window. CI uploads the
   fixture as the `bindings-golden-fixtures` workflow artifact, which the
   action can vendor from a tagged run.
4. **Note the bump in `CHANGELOG.md`.**

CI fails the fixture comparison until the regenerated JSON is committed, so
an un-bumped golden blocks the pipeline by design.

Cross-ABI goldens that follow the same "regenerate + commit" rule:
`abi/contract_error_codes.golden` (error-code stability, Issue #402) and
`abi/export_page.layout.golden` (export page layout). The
`abi` make target checks the error-code golden; see `make help` for the
individual targets.

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

### Authorization matrix

The checked-in [auth matrix](../tests/auth_matrix.csv) is the source of truth
for actor coverage across public mutating entrypoints. Each row contains an
entrypoint, an actor class (`admin`, `second_admin`, `guardian`, `verifier`,
`revoker`, `upgrader`, `user`, `random`, or `none`), and an optional expected
`ContractError` variant.

[`tests/auth_matrix.rs`](../tests/auth_matrix.rs) executes every row
(`cargo test --test auth_matrix`): it builds a fresh fixture, sets up the
operation's preconditions, and invokes the entrypoint with only that actor
signing. An empty `expected_error` must return `Ok`; `NotAuthorized` also
matches a host auth failure for entrypoints that `require_auth` a fixed
identity such as the stored admin. Failures cite the CSV line, operation, and
actor.

When adding a mutating public function, add rows for every meaningful actor
boundary (especially Admin, Verifier, Revoker, Upgrader, registrant, and an
unrelated address) and a dispatcher arm in `plan()` in `tests/auth_matrix.rs`.
The driver fails when an entrypoint that calls `require_auth` has no row, or a
row has no dispatcher arm.

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

## Benchmark Budget CI (Issue #247)

Every PR is gated by a **benchmark regression job** (`bench-budget` in CI).
It runs the Soroban metered-cost tests, compares the results against the
checked-in baselines in `ci/bench-samples.csv`, and fails if any operation
regresses by more than **15 %** or breaches a hard absolute cap.

### Why this matters

Soroban applies a per-transaction instruction budget. A cost regression that
slips past unit tests will surface as failed user transactions during a Wave —
not as a CI failure. The bench job catches regressions before they reach the
network.

### Running locally

```bash
# Full regression check — identical to the CI bench-budget job:
make bench-budget-ci

# Individual bench targets (output only, no regression gate):
make bench-register-budget        # register CPU/mem with hard-cap check
make bench-batch-register-budget  # batch register ceiling CPU/mem integration tests
make bench-export                 # get_all_registered sweep
make bench-username               # usernames_match sweep
make bench-double-verify          # verify success vs rejection
```

Always run with `--test-threads=1` (the Makefile targets do this automatically).
Parallel execution contaminates the Soroban host's global budget state and
produces unstable measurements.

### Updating baselines after an intentional cost change

1. Make your code or dependency change.
2. Run `make bench-update-samples` to regenerate `ci/bench-samples.csv`.
3. Review the diff — confirm only the expected operations changed, by a
   plausible amount.
4. Commit `ci/bench-samples.csv` alongside your code change.
5. Include a before/after cost table in the PR description and a brief
   explanation of why the cost changed.

Do **not** bump baselines to silence an unexplained regression. A cost
increase that cannot be explained is a bug.

### Thresholds at a glance

| Limit type | Value |
|---|---|
| Regression gate | +15 % vs checked-in baseline |
| `register` CPU hard cap | 25 000 000 instructions |
| `register` memory hard cap | 3 000 000 bytes |
| `verify` CPU hard cap | 25 000 000 instructions |
| `verify` memory hard cap | 3 000 000 bytes |

Full details, including host-vs-release notes and SDK upgrade guidance, are in
`docs/BENCHMARK_BUDGETS.md`.

---

## Pull Request Checklist

- [ ] `make check` passes locally
- [ ] `make bench-budget-ci` passes (or baselines updated with `make bench-update-samples` and diff reviewed)
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
- `cargo test`

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

---

## Storage Key Best Practices

Storage keys are **on-chain correctness concerns**: a collision is a catastrophic bug, and key layout changes must stay synchronized across `docs/STORAGE_KEYS.md`, `src/storage.rs`, and test fixtures.

### When adding a new storage key

1. **Pick the partition**:
   - **Instance**: Use for singleton state (admin address, pause flag, network ID, etc.). No TTL, no per-entry context.
   - **Persistent**: Use for per-entry data (registrations, roles, pending operations). TTL-managed; requires a context type (username, address, etc.).

2. **Pick a unique symbol**:
   - Use `symbol_short!("name")` — limited to 9 bytes (ASCII alphanumerics, underscore).
   - Inspect `src/storage.rs` to ensure your symbol is not already defined.
   - Run `make storage-keys-check` to lint; CI will fail if the symbol is not documented.

3. **For persistent keys, choose a context type**:
   - `String` for usernames (context: GitHub username, e.g. `(Symbol("reg"), "alice")`).
   - `Address` for Stellar accounts (context: account ID).
   - `u32` for integers (context: chunk index, ledger sequence).
   - Different context types serialize to different bytes in Soroban's XDR — no collision even if two namespaces use overlapping values.

4. **Verify collision-free**:
   - Instance keys: Ensure your symbol is not already used in the instance table in `docs/STORAGE_KEYS.md`.
   - Persistent keys with new discriminator: Ensure the symbol is not in the persistent table.
   - Persistent keys reusing an existing discriminator: **Never**. Each persistent namespace must have a unique discriminator.
   - If in doubt, see `docs/STORAGE_KEYS.md` § Key Namespace Diagram.

5. **Document before committing**:
   - Add a row to the appropriate table in `docs/STORAGE_KEYS.md`.
   - Add a row to the "Complete Key Inventory" section.
   - Run `make storage-keys-check` to confirm both tables are consistent.
   - If persistent, update the TTL behavior table.
   - Link the issue or feature in the "Purpose" column if applicable.

6. **Update tests**:
   - Add a test to `tests/storage_keys_collision.rs` (see below) to verify the key is stored/read correctly.
   - If the key participates in a workflow, add an integration test to `tests/integration.rs`.

### Collision detection automation

`scripts/check_storage_keys.sh` is run in CI as `make storage-keys-check`. It:
- Parses all `const NAME: Symbol = symbol_short!("...")` from `src/storage.rs`.
- Verifies each constant name and symbol string appear in `docs/STORAGE_KEYS.md`.
- Rejects `Symbol::new()` — all keys must be statically defined constants.
- Fails if `CHUNK_CNT_KEY` is not exactly one definition with symbol `"chkcnt"` (Issue #468).

**Fix**: Add your key to both the inline constant table and the Complete Key Inventory section of `docs/STORAGE_KEYS.md`, then re-run.

### TTL best practices (persistent keys only)

- **Default TTL threshold**: `LEDGERS_PER_DAY * 30` (~30 days).
- **Default TTL bump**: `LEDGERS_PER_DAY * 90` (~90 days).
- Document the exact conditions when TTL is extended (read vs. write, or both).
- See `docs/STORAGE_RENT.md` before changing TTL constants — storage rent is sensitive.

### Testing storage keys

New storage key workflows are tested in:
- `tests/storage_keys_collision.rs` — automated collision detection and namespace validation.
- `tests/integration.rs` — end-to-end workflows using the key.

Add a test case for the happy path (key is stored, retrieved, updated correctly) and at least one failure path (e.g. attempting to write while paused, or accessing a non-existent entry).

### Example: Adding a new instance key

```rust
// src/storage.rs
pub const MY_STATE_KEY: Symbol = symbol_short!("my_state");

// docs/STORAGE_KEYS.md — Instance Storage table
| `"my_state"` | `MY_STATE_KEY` | `MyStateType` | Description of the state |

// docs/STORAGE_KEYS.md — Complete Key Inventory
| `"my_state"` | `MY_STATE_KEY` | line_number | Description |

// tests/storage_keys_collision.rs
#[test]
fn test_my_state_key_stored_correctly() {
    let (env, _admin, _user1, _user2, contract_id) = setup_test_env();
    env.as_contract(&contract_id, || {
        // Happy path: set and read
        TrustBridgeContract::my_getter(env.clone()).unwrap();
        // Failure path: verify error on invalid input
    });
}

// Run
make storage-keys-check
make test
```