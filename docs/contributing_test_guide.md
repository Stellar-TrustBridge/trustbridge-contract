# Contributing: Soroban Test Guide

Clear guidelines for contributors on writing, executing, and submitting contract unit and integration tests for `trustbridge-contract`.

---

## Running the Tests

### All tests (unit + integration)

```bash
cargo test
```

### Unit tests only (src/lib.rs)

```bash
cargo test --lib
```

### Integration tests only (tests/integration.rs)

```bash
cargo test --test integration
```

### Filtered runs — run only the tests for a specific issue

```bash
# Issue #199 — attestation hash, expiry, provenance chain
cargo test attest
cargo test provenance

# Issue #209 — index compaction
cargo test compact

# Issue #211 — typed pause reason codes
cargo test pause

# Issue #213 — reserved username list
cargo test reserved
```

### WASM-gated tests (feature = "wasm-test")

Some tests require a compiled WASM binary and are gated behind the `wasm-test` Cargo feature.
Build the WASM first, then run:

```bash
stellar contract build   # produces target/wasm32v1-none/release/trustbridge_contract.wasm
cargo test --features wasm-test
```

These tests are **not** gated by `#[ignore]` — they use `#[cfg(feature = "wasm-test")]` so they run automatically when the feature is enabled and are fully skipped in normal `cargo test` runs.
CI enables this feature only after a successful `stellar contract build` step.

---

## Test Areas

### Issue #199 — WASM Attestation, Expiry, Provenance Chain

Located in `tests/integration.rs` under the `// Issue #199` section.

| Test | What it covers |
|------|----------------|
| `test_attest_upgrade_stores_attestation` | `attest_upgrade` with future expiry stores attestation |
| `test_attest_upgrade_rejects_past_expiry` | `attest_upgrade` with `expires_at == now` fails (`AttestationExpired`) |
| `test_attest_upgrade_rejects_already_expired_expiry` | `attest_upgrade` with past timestamp fails |
| `test_attest_upgrade_overwrites_previous_attestation` | Second attestation replaces the first |
| `test_clear_attestation_removes_it` | `clear_attestation` makes getter return `None` |
| `test_get_attestation_returns_none_when_absent` | No attestation on a fresh contract |
| `test_get_provenance_none_before_any_upgrade` | Provenance is `None` before any upgrade |
| `test_provenance_written_after_first_upgrade` *(wasm-test)* | Provenance set after first upgrade; `previous_wasm_hash` is `None` |
| `test_provenance_chain_links_successive_upgrades` *(wasm-test)* | Second upgrade's `previous_wasm_hash` links to first |
| `test_attested_upgrade_sets_provenance_attested_flag` *(wasm-test)* | Matching attestation sets `attested = true`; attestation consumed |
| `test_upgrade_with_mismatched_attestation_hash_fails` *(wasm-test)* | Wrong hash → `UnattestedWasm` |
| `test_upgrade_with_expired_attestation_fails_and_clears_it` *(wasm-test)* | Expired attestation → `AttestationExpired`; stale record auto-cleared |

### Issue #209 — Index Compaction

Located in `tests/integration.rs` under the `// Issue #209` section.

| Test | What it covers |
|------|----------------|
| `test_compact_index_empty_registry` | `compact_index` on empty registry returns 0 chunks |
| `test_compact_index_no_op_on_dense_registry` | Compaction on a dense registry leaves pagination unchanged |
| `test_compact_index_after_sparse_removals_restores_dense_pagination` | Compaction after removals removes holes; paginated results match surviving records |
| `test_compact_index_single_entry_registry` | Single-entry registry compacts to one chunk |
| `test_compact_index_is_idempotent` | Running compaction twice produces identical results |
| `test_compact_index_does_not_change_stats` | Compaction never alters `total` or `verified` counters |

### Issue #211 — Typed Pause Reason Codes

Located in `tests/integration.rs` under the `// Issue #211` section.

| Test | What it covers |
|------|----------------|
| `test_pause_stores_reason_code` | `pause(2)` stores `PauseReason::SecurityIncident` |
| `test_unpause_stores_reason_code` | `unpause(4)` stores `PauseReason::Unpause` |
| `test_all_valid_pause_reason_codes_accepted` | All valid codes (1, 2, 3, 99) are accepted |
| `test_pause_with_invalid_reason_code_fails` | Unknown code → `InvalidPauseReason`; contract stays unpaused |
| `test_unpause_with_invalid_reason_code_fails` | Invalid code on `unpause` → contract stays paused |
| `test_set_paused_stores_reason` | `set_paused(true, 3)` stores `PauseReason::RegulatoryHold` |
| `test_pause_reason_readable_while_paused` | `get_pause_reason` is callable while paused |
| `test_pause_reason_overwrite_on_second_pause` | Second `pause` call overwrites stored reason |

### Issue #213 — Reserved Username List

Located in `tests/integration.rs` under the `// Issue #213` section.

| Test | What it covers |
|------|----------------|
| `test_reserved_username_cannot_be_registered` | Reserved name → `UsernameReserved` on `register` |
| `test_is_reserved_reflects_add_remove` | `is_reserved` tracks `add_reserved`/`remove_reserved` |
| `test_reserved_check_is_case_insensitive` | Case variants of a reserved name are all blocked |
| `test_add_reserved_duplicate_fails` | Duplicate add → `AlreadyReserved` |
| `test_remove_reserved_not_present_fails` | Remove non-existent → `NotReserved` |
| `test_non_admin_cannot_add_reserved` | Admin gating is enforced |
| `test_removed_reserved_name_can_be_registered` | After removal, name can be registered again |
| `test_get_reserved_list_returns_all_entries` | List contains all added names |
| `test_adding_reserved_does_not_evict_existing_registration` | Reserving an already-registered name does not evict it |

---

## Writing New Tests

Follow the existing patterns:

- Use `setup_test_env()` (integration) or `setup(&env)` (unit) for boilerplate.
- Call `env.mock_all_auths()` **before** each `env.as_contract(...)` block that requires auth. Do not nest multiple `mock_all_auths` calls inside the same `as_contract` block — Soroban will panic with `Error(Auth, ExistingValue)`.
- Name tests descriptively: `test_<subject>_<condition>_<expected>`.
- Add a comment linking the GitHub issue (`// Issue #NNN`).

---

## CI

All tests without `#[cfg(feature = "wasm-test")]` run unconditionally in CI via:

```yaml
- run: cargo test
```

WASM-gated tests run in a separate CI step after `stellar contract build`:

```yaml
- run: stellar contract build
- run: cargo test --features wasm-test
```

Do not gate non-WASM tests behind `wasm-test` — that defeats the purpose of the feature flag.
