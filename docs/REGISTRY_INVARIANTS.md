# Registry Invariants

These invariants help reviewers reason about safe contract changes. Every
invariant below is enforced by the property fuzzing suite in `src/lib.rs`, so a
change that breaks one fails CI rather than reaching testnet.

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
