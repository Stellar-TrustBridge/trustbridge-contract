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
# Full regression check (mirrors CI exactly):
make bench-budget-ci

# Update the checked-in baselines after an intentional cost change:
make bench-update-samples

# Individual benchmark targets:
make bench-register-budget    # register CPU/mem (with hard-cap check)
make bench-export             # get_all_registered sweep
make bench-username           # usernames_match sweep
make bench-double-verify      # verify success vs rejection
```

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
