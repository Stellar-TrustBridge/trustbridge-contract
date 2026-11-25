# Benchmark Budgets & Recommended Page Sizes

Operational guidance for dashboard and indexer consumers of the paginated
registry reads. It records the contract's hard limits, a recommended page size
for each reader, and the failure symptoms that mean "page too big".

Related: [DASHBOARD_SYNC](DASHBOARD_SYNC.md) · [ABI](ABI.md) ·
[STORAGE_FOOTPRINT](STORAGE_FOOTPRINT.md) · `src/storage.rs`, `src/lib.rs`.

---

## Hard limits (contract-enforced)

These are constants in `src/storage.rs` and are part of the ABI; they are
**not** network tuning knobs:

| Constant | Value | Behavior |
|----------|------:|----------|
| `DEFAULT_PAGE_LIMIT` | `20` | Used when the caller sends `limit = 0` |
| `MAX_PAGE_LIMIT` | `100` | `limit > 100` is **clamped** to `100`, not rejected |
| `CHUNK_SIZE` | `50` | Usernames per persistent chunk entry (`(chunk, idx)`) |

The full index (`idx`) is one instance-storage entry that grows linearly with
the registry size, while each returned row also requires a persistent
`(reg, username)` read. A page is therefore bounded by both the instruction
budget and the ledger-footprint budget, not by `MAX_PAGE_LIMIT` alone. See
[STORAGE_FOOTPRINT.md](STORAGE_FOOTPRINT.md#estimated-footprint-by-registry-size)
for the sizing model and the ~3,259-entry crossover point of the legacy `idx`
entry.

---

## Recommended page sizes

| Consumer | Endpoint | Recommended `limit` | Rationale |
|----------|----------|--------------------:|-----------|
| Public dashboard poll | `get_public_paginated` | `20` (`DEFAULT_PAGE_LIMIT`) | Smallest footprint, no admin key, cheap to retry; blocks while paused |
| Indexer backfill / reconciliation | `get_public_paginated` | `50` (`CHUNK_SIZE`) | Aligns one page with one persistent chunk entry |
| Admin export / audit snapshot | `get_registered_paginated` | `50` | Same footprint as above, plus one admin signature per page |
| Recovery from a suspected OOG | Either | `10`, then increase | Establish a working baseline before tuning upward |

`100` is the ceiling, not the target. Use it only after a successful
`make bench-export` on a comparable registry size. A request above `100`
silently returns `100`, so an apparently "ignored" page size is the clamp, not
a server bug.

CLI examples:

```bash
# Public dashboard poll (default page)
stellar contract invoke --id $ID --network testnet \
  -- get_public_paginated --cursor 0 --limit 20

# Admin export aligned to one chunk
stellar contract invoke --id $ID --source-account admin --network testnet \
  -- get_registered_paginated --cursor 0 --limit 50
```

Always follow `next_cursor` until `has_more == false` / `next_cursor == null`;
never increase `limit` to compensate for a stalled cursor.

---

## Failure symptoms of an oversized page

| Symptom | Meaning | Action |
|---------|---------|--------|
| `Budget exceeded` / `Exceeded resource limit` / instruction-limit error | CPU budget exhausted while materialising the page | Halve `limit`, retry with the same cursor |
| Ledger footprint / `max_contract_data_entry_size` error | Too many distinct ledger entries touched | Drop to `20`, then walk chunks one at a time |
| RPC timeout or 5xx while returning a large page | Response/invoke budget exceeded off-chain | Use `20`, increase client timeout, keep the cursor loop |
| Page shorter than requested without `has_more` | Registry exhausted, or a stale cursor | Trust `has_more`/`next_cursor`, re-read `get_stats()` |
| `Paused` on `get_public_paginated` | Circuit breaker active, not a budget issue | Retry after unpause; `get_address` remains readable |
| Requested 150, received 100 | `MAX_PAGE_LIMIT` clamp | Set the limit explicitly; split into two pages |

An oversized page fails the **whole** invocation — it never returns a partial
page. Keep the last successful cursor as the retry point.

---

## Reproducing the budget numbers

No absolute instruction counts are pinned here: they move with `soroban-sdk`
releases. The suite asserts on shape (monotonic cost, failure paths cheaper
than success) so a regression is caught without brittle constants.

```bash
make bench              # CPU/memory per operation, CSV output
make bench-export       # export-only run → bench-results.txt
make bench-username     # case-normalization cost → bench-username-normalization.txt
```

The benchmarks live in `src/lib.rs` under `// === Cost benchmarks`; see
[ABI.md § Cost and Benchmarks](ABI.md#cost-and-benchmarks) for the full list.
Revisit the recommended sizes above if `CHUNK_SIZE`, `MAX_PAGE_LIMIT`, the
`ContributorRecord` shape, or the target network's resource config changes.
# TrustBridge Contract — Benchmark Budgets

Soroban charges every invocation against a **metered instruction budget**.
A regression here means real user transactions fail during a Wave, not a test
failure on a developer's laptop — which is why these budgets are tracked in CI
rather than left to manual review.

---

## What is measured

The Soroban SDK's `env.cost_estimate().budget()` tracks two resources per
invocation:

| Resource | Unit | Soroban per-tx limit |
|---|---|---|
| CPU instructions | abstract instruction count | ~100 000 000 |
| Memory | bytes allocated | ~40 000 000 |

These are **simulated host measurements** from the test environment. Actual
on-chain costs may differ slightly due to host-version deltas, but the test
host uses the same metering model as the real network, so regressions caught
here reliably predict on-chain cost growth.

---

## Checked-in budget samples

`ci/bench-samples.csv` is the source of truth for regression gating. It
contains the measured cost of each benchmark operation at the time it was last
intentionally updated. The format is:

```
operation,input_label,cpu_instructions,memory_bytes
```

The file is updated by running:

```bash
make bench-update-samples
```

That command runs every `test_bench_*` / `test_report_*` test in release-like
conditions (`--test-threads=1`), captures their CSV output, and rewrites
`ci/bench-samples.csv`. Commit the result when a cost change is intentional
(new feature, dependency update, Soroban SDK bump).

---

## Regression threshold

CI fails when any operation's measured CPU or memory cost exceeds its
checked-in baseline by more than **15 %**.

```
threshold = baseline * 1.15
```

15 % is intentionally coarse:

- It absorbs legitimate noise between debug and release profiles, and between
  host versions in CI and on a developer's machine.
- It still catches the regressions that matter: a hot-path allocation loop,
  an accidentally-quadratic scan, or an unintended dependency being pulled in.

To raise the threshold for a specific operation, update `ci/bench-samples.csv`
with the new baseline and include a justification in the PR description.

---

## Tracked operations

| Operation | Benchmark test | Key scenario |
|---|---|---|
| `register` | `test_report_register_budget_samples` | Short username (`baseline`), max-length username (`max_username_len`) |
| `usernames_match` | `test_bench_username_case_normalization` | 10 / 50 / 100 / 200 comparisons |
| `get_all_registered` | `test_bench_export_cpu_cost` | 10 / 20 / 40 / 80 registry entries |
| `verify` (success) | `test_bench_double_verify_rejection` | First successful verify |
| `verify` (rejected) | `test_bench_double_verify_rejection` | Double-verify rejection must be cheaper than success |

### Hard absolute limits

These caps are enforced in addition to the regression check. They reflect the
maximum cost that can possibly succeed on-chain without a transaction exceeding
Soroban limits. Any single-operation test that breaches these limits fails CI
regardless of the regression percentage.

| Operation | CPU hard cap | Memory hard cap |
|---|---|---|
| `register` (any input) | 25 000 000 | 3 000 000 |
| `verify` (success) | 25 000 000 | 3 000 000 |
| `verify` (rejection) | 25 000 000 | 3 000 000 |

### Batch register limits and multi-register workloads (Issue #612)

Bulk registration workloads are bounded by `MAX_WRITE_BATCH = 25` (`src/batch.rs`).
Sequential register integration tests in `tests/integration.rs` (`test_batch_register_at_max_write_batch_ceiling`)
meter execution costs across the entire batch ceiling:

| Workload | Batch size | Observed CPU instructions | Observed Memory bytes | Headroom vs Soroban tx limit |
|---|---|---|---|---|
| Sequential `register` batch | 25 (`MAX_WRITE_BATCH`) | ~1 970 000 | ~910 000 | > 98 % CPU, > 97 % memory |

Oversized batches (`> MAX_WRITE_BATCH`) and empty batches (`0`) are rejected immediately before
contract budget consumption with `ContractError::InvalidBatchSize`.

---

## CI job

The `bench-budget` job in `.github/workflows/ci.yml` runs after the main
`quality` job. It:

1. Builds in standard test mode (no WASM needed — the test host simulates
   metering natively).
2. Runs all `test_bench_*` and `test_report_register_budget_samples` tests
   with `--nocapture --test-threads=1`.
3. Passes captured stdout through `scripts/check_bench_regression.sh`, which
   compares each reported sample against `ci/bench-samples.csv`.
4. Uploads the raw bench output as a CI artifact (`bench-budget-report`) with
   a 30-day retention for post-mortem inspection.
5. Fails the job if any regression exceeds 15 % or any hard cap is breached.

The job runs on every push and PR to `main`, `master`, and `develop`, the same
branches as the quality gate.

---

## Running locally

```bash
# Repeat register/verify pairs and record the per-operation high-water budget:
make soak-register-verify
REGISTER_VERIFY_SOAK_ITERATIONS=1000 make soak-register-verify

# Full regression check (mirrors CI exactly):
make bench-budget-ci

# Update the checked-in baselines after an intentional cost change:
make bench-update-samples

# Individual benchmark targets:
make bench-register-budget          # register CPU/mem (with hard-cap check)
make bench-batch-register-budget    # batch register ceiling CPU/mem integration tests
make bench-export                   # get_all_registered sweep
make bench-username                 # usernames_match sweep
make bench-double-verify            # verify success vs rejection
```

The opt-in register/verify soak checks every pair's resulting counts and
verification state, resetting the simulated budget between operations. It
reports the maximum observed CPU and memory for `register` and `verify`
separately and enforces the same 25M CPU / 3MB memory caps as the focused
budget checks. Increase `REGISTER_VERIFY_SOAK_ITERATIONS` for longer local
rehearsals; the ignored soak is not part of the default test suite.
The soak reports CPU/memory, not XLM rent; pair it with
`make storage-footprint-check` and `make storage-rent-estimator` when reviewing
persistent-storage cost changes.

---

## Host vs release differences

The test host runs in **debug mode by default** and uses a fixed-seed cost
model. A few things to keep in mind:

- **Debug vs release**: `cargo test` uses the debug profile. The Soroban SDK
  test host's metering model is deterministic regardless of the Rust
  compilation profile, so measurements from `cargo test` are stable enough for
  regression gating.
- **Host version pinned**: the SDK version in `Cargo.toml` pins the host. A
  `soroban-sdk` version bump can shift all measurements; update baselines when
  bumping the SDK.
- **CI runner vs local**: GitHub Actions runners are identical across runs for
  the same runner label, so measured values are stable in CI. Local
  measurements may differ by a few percent on different hardware, but the 15 %
  threshold absorbs this.
- **`--test-threads=1`**: required for stable measurement. Parallel test
  execution causes cross-thread measurement contamination in the Soroban test
  host's global budget state.

---

## Updating baselines intentionally

1. Make your code change.
2. Run `make bench-update-samples` to regenerate `ci/bench-samples.csv`.
3. Review the diff — confirm the numbers changed only in the operations you
   expected, and by the amount you expected.
4. Commit `ci/bench-samples.csv` alongside your code change.
5. In the PR description, include a before/after cost table for the affected
   operations and a brief explanation of why the cost changed.

Do **not** bump baselines to silence a regression without understanding its
cause. A cost increase that cannot be explained is a bug.

---

## Recalibrating after protocol or SDK upgrades

When Soroban protocol versions, `soroban-sdk` in `Cargo.toml`, or Stellar CLI
versions change, host metering costs can shift across all operations even when
contract logic is unchanged. Follow this process in the upgrade PR so Wave CI
does not false-fail on stale baselines:

1. **Observe deltas first:** Run `make bench-budget-ci` before updating
   baselines to see which operations exceed the 15% regression threshold or
   approach hard caps (25M CPU / 3M mem).
2. **Run protocol rehearsal (if applicable):** Run `make test-rehearsal`
   (`test_protocol_upgrade_rehearsal --features wasm-test`) to verify upgrade
   compatibility.
3. **Regenerate baselines:** Run `make bench-update-samples`. This executes
   `test_bench_username_case_normalization`, `test_bench_export_cpu_cost`,
   `test_bench_double_verify_rejection`, and `test_report_register_budget_samples`
   with `--nocapture --test-threads=1` and rewrites `ci/bench-samples.csv`,
   including the `# LAST UPDATED: <date> (soroban-sdk <version>, stable toolchain)`
   header.
4. **Review tables & caps:** Inspect `git diff ci/bench-samples.csv` and check
   whether any hard caps, recommended page sizes (`DEFAULT_PAGE_LIMIT` /
   `MAX_PAGE_LIMIT`), or `MAX_WRITE_BATCH` in this document need adjustment.
5. **PR expectations:** Commit `ci/bench-samples.csv` (and any updated tables in
   `docs/BENCHMARK_BUDGETS.md`) in the same upgrade PR, and include a
   before/after cost table noting the old and new protocol/`soroban-sdk`
   versions.

---

## Storage footprint budget

CPU/memory budgets above do not cover **bytes stored**, which drive rent.
Those are gated separately by `make storage-footprint-check`
(`storage_footprint_budget_tests` in `src/storage.rs`, CI job
`storage-budgets`), against the `max_bytes` table in
[STORAGE_FOOTPRINT.md](STORAGE_FOOTPRINT.md#measured-footprint-budget-ci-check).
A new storage key that grows with the registry must add a row there, or the
check fails.

