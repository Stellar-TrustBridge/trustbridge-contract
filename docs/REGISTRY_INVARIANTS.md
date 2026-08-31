# Registry Invariants

These invariants help reviewers reason about safe contract changes. The
invariants below are enforced by the property fuzzing suite in `src/lib.rs` or
the opt-in scale harness, so a change that breaks one is caught before reaching
testnet.

Related docs: [ABI](ABI.md) · [ARCHITECTURE](ARCHITECTURE.md) · [SECURITY](SECURITY.md)

---

## Identity mapping

- A GitHub username maps to at most one Stellar address at a time.
- Registration changes should keep the total count in sync with stored records.
- Removal should clear lookup state and update export indexes consistently.

## Verification

- Only the configured admin can mark a contributor as verified.
- Verification should never replace address ownership checks.
- Verified count must only change when a record crosses the unverified/verified boundary.

---

## Enumerated Invariants

| ID | Invariant | Enforced by |
|----|-----------|-------------|
| I1 | `get_stats().total` equals the number of live records | `assert_registry_invariants` |
| I2 | `get_stats().verified` equals the number of live records with `verified == true` | `assert_registry_invariants` |
| I3 | `get_verified_count()` never diverges from `get_stats().verified` | `assert_registry_invariants` |
| I4 | `verified <= total` at all times | `assert_registry_invariants` |
| I5 | The export index holds exactly one entry per live record (no leaks, no duplicates) | `assert_registry_invariants` |
| I6 | Every username resolves to the address last registered for it, with the expected verification flag | `assert_registry_invariants` |
| I7 | A rejected operation moves no counter and mutates no record | `test_fuzz_failure_paths_leave_invariants_intact` |
| I8 | Counters never underflow, including removal attempts against an empty registry | `test_fuzz_counters_never_underflow_on_empty_registry` |
| I9 | A complete paginated export visits every live username exactly once, with no duplicate, skip, or reorder across chunk boundaries | `test_paginated_export_at_10k_users` |

### `get_verified_count()` / `get_stats().verified` parity (Issue #90)

I3 above is also exercised directly, outside the fuzz suite, by
`test_verified_count_parity_across_all_mutation_paths` in `src/lib.rs`. That
test asserts `get_verified_count()` and `get_stats().verified` agree after
every mutation path that touches verification state — `register` (including
an address-change re-register), `verify`, `revoke_verification`, `remove`,
a re-verify cycle, and the empty registry — so the two counters cannot drift
apart silently. See [ABI.md#get_verified_count---u32](ABI.md#get_verified_count---u32).

### Verification carry-over rule

Re-registering an existing username keeps `verified == true` **only** when the
new Stellar address equals the stored one. Any address change resets the record
to unverified and decrements the verified count. The fuzz model mirrors this
rule directly, so a change to the carry-over logic surfaces as an I2 or I6
failure.

---

## How the Fuzzing Works

The suite lives alongside the unit tests in `src/lib.rs` under the
`// === Issue #200: Property fuzzing suite` section.

- **No external fuzzing crate.** The contract is `#![no_std]`, which rules out
  `proptest` and `arbitrary`. The suite uses a small xorshift64 generator
  (`Prng`) instead.
- **Deterministic seeds.** Seeds are fixed constants (`FUZZ_SEEDS`). A CI
  failure reproduces locally by rerunning the same test, with no flakes and no
  seed corpus to store.
- **Shadow model.** `Shadow` mirrors the registry outside contract storage.
  Every assertion compares contract state against that independent model, so a
  bug in the contract's own counters cannot mask itself.
- **Operation mix.** Each step picks a random username slot and one of
  `register`, `verify`, `revoke_verification`, or `remove`. Operations the model
  predicts will fail are asserted to return the exact expected `ContractError`,
  so both success and failure paths are covered.
- **Invariants after every step**, not only at the end of a run, so the failing
  operation is the one reported.

### Running

```bash
cargo test fuzz              # invariant suite only
cargo test                   # full suite
make fuzz                    # same as cargo test fuzz (Makefile target)
make check                   # fmt + clippy + test + build
```

### 10k-user pagination scale test

The host-side integration harness `test_paginated_export_at_10k_users` inserts
10,000 deterministic usernames, which fills 200 `CHUNK_SIZE`-50 chunks. It
walks both the admin and public cursor-based exports with 100-record pages and
asserts the total, page sizes, cursor progression, and exact username sequence.
The test disables Soroban test-environment resource limits because registration
and the full host-side walk are intentionally larger than a single mainnet
invocation. It is feature-gated and excluded from the default fast suite.

Run it with:

```bash
make test-scale
```

This is a local or scheduled load check; it does not represent a mainnet
transaction or deployment test.

### Coverage profile

| Test | Steps | Purpose |
|------|-------|---------|
| `test_fuzz_invariants_hold_across_random_operation_sequences` | 4 seeds × 64 | Broad operation mixing |
| `test_fuzz_invariants_hold_at_contributor_scale` | 256 | Repeated churn on 16-username pool (stresses chunk boundaries) |
| `test_fuzz_failure_paths_leave_invariants_intact` | 48 | Rejected operations are side-effect free (I7) |
| `test_fuzz_counters_never_underflow_on_empty_registry` | 32 | Saturating-arithmetic guard (I8) |

### Extending the suite

When adding a contract function that mutates state:

1. Add a match arm in `run_fuzz_session` covering the new operation.
2. Update `Shadow` so the model predicts the new state transition.
3. Add the resulting invariant to the table above.

A new mutating function with no fuzz arm is a review blocker.

---

## Bounded Model-Check Proofs for Counter Invariants (Issue #241)

### Why bounded model checking instead of Kani

[Kani](https://model-checking.github.io/kani/) provides LLVM-level symbolic
execution, but requires a separate LLVM toolchain and adds significant time to
every CI run. The issue allows a smaller checker as long as it is
machine-checked and not merely prose.

The harnesses in `tests/counter_proofs.rs` use **exhaustive enumeration over
small, bounded domains**: they cover every reachable state up to a bounded
number of operations. For counter arithmetic with no unbounded loops this is
equivalent to bounded model checking — a pass is a machine-verified proof for
the bounded domain, not a probabilistic sample.

### Proved invariants

| ID | Invariant | Harness | Bounded domain |
|----|-----------|---------|----------------|
| P1 | `verify` increments `verified_count` by exactly 1 | `proof_verify_increments_verified_count` | 1 register + 1 verify |
| P1b | Verifying N records increments by exactly N | `proof_verify_increments_count_by_n` | N ∈ {2, 4, 8} |
| P2 | `remove` of verified record decrements `verified_count` by 1 | `proof_remove_decrements_verified_count` | 1 register + 1 verify + 1 remove |
| P3 | `remove` of unverified record does NOT touch `verified_count` | `proof_remove_does_not_decrement_count_for_unverified` | 1 register + 1 remove |
| P4 | `verify` is idempotent — second call does NOT double-increment | `proof_verify_is_idempotent_on_verified_count` | 1 register + 2 verify |
| P5 | `verified <= total` after every operation | `proof_verified_never_exceeds_total_exhaustive` | N ∈ {1, 2, 3, 4} × all phases |
| P6 | Counters never underflow (saturating-arithmetic guard) | `proof_counters_never_underflow` | 32 remove-spam attempts; 4 register + partial verify + full remove |
| P7 | `revoke_verification` decrements and is idempotent | `proof_revoke_decrements_and_is_idempotent` | 1 register + 1 verify + 2 revoke |

### Running the proof harness

```bash
# Run all proof harnesses
cargo test --test counter_proofs

# Run with output (shows the bounded domain being exercised)
cargo test --test counter_proofs -- --nocapture
```

CI runs these in the dedicated `counter-invariant-proofs` job
(see `.github/workflows/ci.yml`). The job is also indirectly run by the main
`quality` job via `cargo test` so every PR is covered.

### Saturating vs wrapping counters

The proofs in P6 specifically guard against the `saturating_sub` vs
`wrapping_sub` distinction. The contract uses saturating arithmetic
(`saturating_sub`) for all counter decrements, so an attempted underflow
saturates to 0 rather than wrapping to `u32::MAX`. P6 asserts both that:

1. Failed `remove` calls return `ContractError::NotRegistered` and leave
   counters at 0 (no side effects on invalid input).
2. After removing all records from a partially-verified registry, both
   `total` and `verified` reach 0 and neither counter ever equals `u32::MAX`.

### Unregistered-verify guard

P3 documents the "unregistered verify" edge case: the contract returns
`ContractError::NotRegistered` before touching any counter, so a verify
call on a non-existent username cannot inflate `verified_count`. This is
covered by the property-fuzzing suite (I7) and confirmed by P3 and P6.

### Extending the proof suite

When adding a contract function that mutates `total` or `verified`:

1. Add a `proof_*` test in `tests/counter_proofs.rs`.
2. Add a row to the table above with the bounded domain.
3. Ensure the harness asserts the parity invariant (`stats().verified ==
   get_verified_count()`) after every mutating step.
