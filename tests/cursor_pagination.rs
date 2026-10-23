//! Standalone verification for Issue #215 (opaque cursor tokens for
//! `get_registered_paginated` / `get_public_paginated`).
//!
//! Kept as its own integration test file, independent of `tests/integration.rs`,
//! so it can be built and run on its own via
//! `cargo test --test cursor_pagination` regardless of the state of that
//! other file.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, BytesN, Env, String, Vec};

use trustbridge_contract::{ContractError, TrustBridgeContract};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    (env, admin, contract_id)
}

fn s(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

/// Registers `n` freshly-generated users named `user{start}..user{start+n}`,
/// so repeated calls in one test (with increasing `start`) grow the registry
/// without re-registering (and thus not actually adding) an earlier name.
fn register_range(env: &Env, start: u32, n: u32) -> Vec<(String, Address)> {
    let mut out = Vec::new(env);
    for i in start..start + n {
        let addr = Address::generate(env);
        let name = s(env, &alloc::format!("user{i:03}"));
        TrustBridgeContract::register(env.clone(), name.clone(), addr.clone(), Vec::new(env))
            .unwrap();
        out.push_back((name, addr));
    }
    out
}

fn register_n(env: &Env, n: u32) -> Vec<(String, Address)> {
    register_range(env, 0, n)
}

extern crate alloc;

#[test]
fn test_full_walk_with_cursor_none_start_visits_every_record_once() {
    let (env, _admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_n(&env, 25);

        let mut seen: alloc::vec::Vec<String> = alloc::vec::Vec::new();
        let mut cursor = None;
        loop {
            let page =
                TrustBridgeContract::get_registered_paginated(env.clone(), cursor, 10).unwrap();
            for i in 0..page.records.len() {
                let (username, _) = page.records.get(i).unwrap();
                seen.push(username);
            }
            if !page.has_more {
                assert!(page.next_cursor.is_none());
                break;
            }
            cursor = page.next_cursor;
        }

        assert_eq!(seen.len(), 25);
    });
}

#[test]
fn test_empty_registry_page_has_no_next_cursor() {
    let (env, _admin, contract_id) = setup();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let page = TrustBridgeContract::get_registered_paginated(env.clone(), None, 10).unwrap();
        assert_eq!(page.records.len(), 0);
        assert!(page.next_cursor.is_none());
        assert!(!page.has_more);
    });
}

#[test]
fn test_last_page_has_no_next_cursor() {
    let (env, _admin, contract_id) = setup();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_n(&env, 5);
        let page = TrustBridgeContract::get_registered_paginated(env.clone(), None, 10).unwrap();
        assert_eq!(page.records.len(), 5);
        assert!(page.next_cursor.is_none());
        assert!(!page.has_more);
    });
}

/// The core Issue #215 scenario: a cursor issued before a removal must not
/// be usable to silently continue past drifted positions — it must fail
/// loudly instead.
#[test]
fn test_removing_a_record_after_a_cursor_is_issued_invalidates_it() {
    let (env, admin, contract_id) = setup();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_n(&env, 10);

        // First page of 4; there is more after it.
        let page1 = TrustBridgeContract::get_registered_paginated(env.clone(), None, 4).unwrap();
        assert_eq!(page1.records.len(), 4);
        assert!(page1.has_more);
        let stale_cursor = page1.next_cursor.clone().unwrap();

        // Remove a record that sits *before* the stale cursor's offset,
        // shifting every later position back by one.
        TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, "user000")).unwrap();

        // Continuing with the pre-removal cursor must fail loudly rather
        // than silently return a drifted page.
        let result =
            TrustBridgeContract::get_registered_paginated(env.clone(), Some(stale_cursor), 4);
        assert_eq!(result, Err(ContractError::InvalidCursor));

        // The documented recovery — restart from `cursor = None` — must
        // succeed and eventually visit every remaining record exactly once.
        let mut seen: alloc::vec::Vec<String> = alloc::vec::Vec::new();
        let mut cursor = None;
        loop {
            let page =
                TrustBridgeContract::get_registered_paginated(env.clone(), cursor, 4).unwrap();
            for i in 0..page.records.len() {
                let (username, _) = page.records.get(i).unwrap();
                seen.push(username);
            }
            if !page.has_more {
                break;
            }
            cursor = page.next_cursor;
        }
        assert_eq!(seen.len(), 9);
        assert!(!seen.iter().any(|u| *u == s(&env, "user000")));
    });
}

/// A removal that happens *after* the position a cursor already passed
/// still invalidates it under this contract's coarse (whole-index)
/// generation bump — documented behavior, not a bug: any removal shifts the
/// global offset space, so the safe rule is "any removal invalidates every
/// outstanding cursor," not just ones downstream of the removed entry.
#[test]
fn test_removal_anywhere_invalidates_outstanding_cursors_even_if_already_passed() {
    let (env, admin, contract_id) = setup();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_n(&env, 10);

        let page1 = TrustBridgeContract::get_registered_paginated(env.clone(), None, 4).unwrap();
        let stale_cursor = page1.next_cursor.clone().unwrap();

        // Remove a record *after* the cursor's offset this time.
        TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, "user009")).unwrap();

        let result =
            TrustBridgeContract::get_registered_paginated(env.clone(), Some(stale_cursor), 4);
        assert_eq!(result, Err(ContractError::InvalidCursor));
    });
}

#[test]
fn test_cursor_is_interchangeable_between_admin_and_public_paginated() {
    let (env, _admin, contract_id) = setup();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_n(&env, 6);

        let page1 = TrustBridgeContract::get_public_paginated(env.clone(), None, 3).unwrap();
        assert!(page1.has_more);
        let cursor = page1.next_cursor;

        // Resume with the admin-gated variant using the cursor the public
        // variant issued.
        let page2 = TrustBridgeContract::get_registered_paginated(env.clone(), cursor, 3).unwrap();
        assert_eq!(page2.records.len(), 3);
        assert!(!page2.has_more);
    });
}

/// A cursor is opaque by design (Issue #215): this crate deliberately does
/// not expose a way to construct one outside of a contract call, so an
/// "out of range offset with an otherwise-valid generation" case is not
/// independently reachable from outside the contract — any removal that
/// would shrink the registry below a previously-issued offset also bumps
/// the generation, so `test_removing_a_record_after_a_cursor_is_issued_invalidates_it`
/// and `test_removal_anywhere_invalidates_outstanding_cursors_even_if_already_passed`
/// above already exercise the only way a stale cursor can actually arise.
#[test]
fn test_registering_more_after_a_cursor_is_issued_does_not_invalidate_it() {
    let (env, _admin, contract_id) = setup();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_n(&env, 3);

        let page1 = TrustBridgeContract::get_registered_paginated(env.clone(), None, 2).unwrap();
        assert!(page1.has_more);
        let cursor = page1.next_cursor;

        // Appending new registrations only grows the index past the cursor's
        // offset; it never shifts an existing position, so the cursor from
        // before this registration must still be valid.
        register_range(&env, 3, 2);

        let page2 = TrustBridgeContract::get_registered_paginated(env.clone(), cursor, 10).unwrap();
        assert!(!page2.records.is_empty());
    });
}

// ── Soak-style export_cursor decode property (Issue #650) ────────────────────
//
// `fuzz/fuzz_targets/export_cursor.rs` validates decode robustness against
// arbitrary bytes, but it needs `cargo-fuzz` + nightly. This is the
// CI-friendly counterpart: a deterministic soak loop replaying thousands of
// cursor permutations from fixed seeds and asserting an adversarial cursor
// can never panic the export path — a cursor whose embedded generation no
// longer matches, or whose offset is past the registry size, must return
// exactly `Err(ContractError::InvalidCursor)`.
//
// Seeds come from `tests/fuzz/seeds.txt`, the same corpus the in-`lib.rs`
// property suite uses (see `docs/REGISTRY_INVARIANTS.md` § "How the Fuzzing
// Works", invariant I9). Append a seed there when a failure is found so it
// reproduces forever.
//
// Iteration budget: the `CURSOR_SOAK_ITERATIONS` env var sets the per-seed
// loop count (default 512 — fast enough for default CI). Local long soak:
// `make soak-cursor` (`CURSOR_SOAK_ITERATIONS=20000`).
//
// Pinned edge cases (hold on every run, independent of the loop):
// - `limit == 0` falls back to `DEFAULT_PAGE_LIMIT`, never an error.
// - `BytesN<8>` is fixed-width at the type level, so over-long cursor bytes
//   cannot be expressed by a caller at all (the fuzz target mirrors this by
//   rejecting other lengths at its input boundary).
// - A generation bump (any removal) invalidates previously issued cursors.

/// Default soak iterations per seed when `CURSOR_SOAK_ITERATIONS` is unset.
const DEFAULT_SOAK_ITERATIONS: u64 = 512;

/// Env var overriding the per-seed soak iteration count for local long runs.
const SOAK_ITERATIONS_ENV: &str = "CURSOR_SOAK_ITERATIONS";

/// Soak iteration budget: `CURSOR_SOAK_ITERATIONS`, or 512 when unset/empty/invalid.
fn soak_iterations() -> u64 {
    std::env::var(SOAK_ITERATIONS_ENV)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_SOAK_ITERATIONS)
}

/// xorshift64* — the same generator family as the in-`lib.rs` property
/// suite's `Prng` (see `docs/REGISTRY_INVARIANTS.md`); deterministic per seed.
fn xorshift64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

/// Loads `tests/fuzz/seeds.txt` (hex `0x…` or decimal, `#` comments and
/// blank lines ignored), mirroring the corpus loader behind the in-`lib.rs`
/// fuzz suite. Falls back to a pinned seed if the file is unreadable so the
/// soak still runs hermetically.
fn load_fuzz_seeds() -> alloc::vec::Vec<u64> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fuzz/seeds.txt");
    let mut seeds = alloc::vec::Vec::new();
    if let Ok(text) = std::fs::read_to_string(&path) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let value =
                if let Some(hex) = line.strip_prefix("0x").or_else(|| line.strip_prefix("0X")) {
                    u64::from_str_radix(hex, 16).ok()
                } else {
                    line.parse::<u64>().ok()
                };
            if let Some(v) = value {
                seeds.push(v);
            }
        }
    }
    if seeds.is_empty() {
        // Pinned fallback: seeds[0] of tests/fuzz/seeds.txt.
        seeds.push(0xDEAD_BEEF_1234_5678);
    }
    seeds
}

/// White-box cursor forge: bytes `0..4` are the offset, bytes `4..8` are the
/// generation, both big-endian — the layout the contract embeds at issue
/// time. Mirrors the forging in `fuzz/fuzz_targets/export_cursor.rs`; the
/// contract itself deliberately exposes no cursor constructor (see the
/// interchangeability test above), so tests forge bytes exactly like the
/// fuzzer does.
fn forge_cursor(env: &Env, offset: u32, generation: u32) -> BytesN<8> {
    let mut bytes = [0u8; 8];
    bytes[0..4].copy_from_slice(&offset.to_be_bytes());
    bytes[4..8].copy_from_slice(&generation.to_be_bytes());
    BytesN::from_array(env, &bytes)
}

/// Deterministic valid-cursor prologue: every reachable (offset, limit)
/// combination on a 3-record fixture must succeed — including `limit == 0`
/// (falls back to `DEFAULT_PAGE_LIMIT`) and over-cap limits (clamped to
/// `MAX_PAGE_LIMIT`), and `offset == total` (empty last page, not an error).
#[test]
fn test_soak_valid_cursor_prologue() {
    let (env, _admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        for name in ["alice", "bob", "carol"] {
            TrustBridgeContract::register(
                env.clone(),
                s(&env, name),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        }
        let total = 3u32;
        // Mirror storage::DEFAULT_PAGE_LIMIT / storage::MAX_PAGE_LIMIT.
        let default_limit = 20u32;
        let max_limit = 100u32;

        for offset in 0..=total {
            for limit in [0u32, 1, 2, 3, 20, 100, 101, u32::MAX] {
                let cursor = forge_cursor(&env, offset, 0);
                let page =
                    TrustBridgeContract::get_public_paginated(env.clone(), Some(cursor), limit)
                        .expect("in-generation, in-range cursor must decode");
                let effective = if limit == 0 {
                    default_limit
                } else {
                    limit.min(max_limit)
                };
                assert_eq!(
                    page.records.len(),
                    (total - offset).min(effective),
                    "offset {offset} limit {limit}"
                );
                assert_eq!(page.total, total);
                assert_eq!(page.has_more, page.next_cursor.is_some());
            }
        }
    });
}

/// The soak loop proper: replays deterministic cursor permutations from the
/// checked-in seed corpus and asserts adversarial bytes never panic — every
/// outcome is either an `Ok` page with bounded contents or exactly
/// `Err(ContractError::InvalidCursor)`.
#[test]
fn test_soak_export_cursor_decode_never_panics() {
    let (env, _admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // 3-record fixture mirrors fuzz/fuzz_targets/export_cursor.rs, whose
        // oracle is: generation 0 + offset <= 3 ⇒ page, else InvalidCursor.
        for name in ["alice", "bob", "carol"] {
            TrustBridgeContract::register(
                env.clone(),
                s(&env, name),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        }
        let total = 3u32;
        let default_limit = 20u32; // mirrors storage::DEFAULT_PAGE_LIMIT
        let max_limit = 100u32; // mirrors storage::MAX_PAGE_LIMIT

        let seeds = load_fuzz_seeds();
        assert!(seeds.len() >= 4, "seed corpus tests/fuzz/seeds.txt must hold at least 4 seeds");
        let iters = soak_iterations();

        for (si, seed) in seeds.iter().enumerate() {
            let mut rng = *seed ^ 0x9E37_79B9_7F4A_7C15;
            if rng == 0 {
                rng = 0x2545_F491_4F6C_DD1D;
            }
            for i in 0..iters {
                let a = xorshift64(&mut rng);
                let b = xorshift64(&mut rng);
                // Every 4th draw stays near the valid boundary (0..=7) so the
                // offset-past-end edge (`offset > total`) is hit densely;
                // otherwise the full u32 range exercises far-past-end offsets.
                let offset = if i % 4 == 0 {
                    (b & 7) as u32
                } else {
                    (a >> 32) as u32
                };
                let forged_gen = (a & 0xFFFF_FFFF) as u32;
                // Spans 0 (default-limit path), normal, and over-cap limits.
                let limit = ((b >> 32) % 150) as u32;
                let cursor = forge_cursor(&env, offset, forged_gen);
                match TrustBridgeContract::get_public_paginated(
                    env.clone(),
                    Some(cursor),
                    limit,
                ) {
                    Ok(page) => {
                        // Reachable only for the live generation (0 — no
                        // removal has happened yet) with an in-range offset.
                        assert_eq!(
                            forged_gen, 0,
                            "seed {si} iter {i}: an Ok page requires the live generation"
                        );
                        assert!(
                            offset <= total,
                            "seed {si} iter {i}: an Ok page requires an in-range offset"
                        );
                        let effective = if limit == 0 {
                            default_limit
                        } else {
                            limit.min(max_limit)
                        };
                        assert_eq!(
                            page.records.len(),
                            (total - offset).min(effective),
                            "seed {si} iter {i}: bounded page contents"
                        );
                        assert_eq!(page.total, total);
                        assert_eq!(page.has_more, page.next_cursor.is_some());
                    }
                    Err(e) => assert_eq!(
                        e,
                        ContractError::InvalidCursor,
                        "seed {si} iter {i}: adversarial cursor must map to InvalidCursor, never panic"
                    ),
                }
            }
        }
    });
}

/// A generation bump (any removal) invalidates previously valid cursors —
/// both contract-issued `next_cursor` values and the same bytes forged
/// directly. See `docs/REGISTRY_INVARIANTS.md` invariant I9.
#[test]
fn test_soak_generation_bump_invalidates_old_cursors() {
    let (env, admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_n(&env, 5);

        // Pre-removal: the forged cursor and the issued cursor agree.
        let forged = forge_cursor(&env, 2, 0);
        let forged_page =
            TrustBridgeContract::get_public_paginated(env.clone(), Some(forged.clone()), 2)
                .unwrap();
        assert_eq!(forged_page.records.len(), 2);

        let page1 = TrustBridgeContract::get_registered_paginated(env.clone(), None, 2).unwrap();
        assert!(page1.has_more);
        let issued = page1.next_cursor.clone().unwrap();

        // Any removal bumps the index generation: both cursors must now fail
        // loudly instead of silently resuming at a drifted offset.
        TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, "user000")).unwrap();

        assert_eq!(
            TrustBridgeContract::get_public_paginated(env.clone(), Some(issued), 2),
            Err(ContractError::InvalidCursor)
        );
        assert_eq!(
            TrustBridgeContract::get_public_paginated(env.clone(), Some(forged), 2),
            Err(ContractError::InvalidCursor)
        );
    });
}

// ── Cursors from other INDEX_GEN values (Issue #693) ────────────────────────

/// Instance key holding the index generation (`INDEX_GEN_KEY` in
/// `src/storage.rs`, documented in `docs/STORAGE_KEYS.md`).
fn set_index_generation(env: &Env, generation: u32) {
    env.storage()
        .instance()
        .set(&soroban_sdk::symbol_short!("idx_gen"), &generation);
}

/// Happy path: a cursor stamped with the current generation resumes exactly
/// where the issuing page stopped, on the public read path.
#[test]
fn test_public_paginated_accepts_cursor_from_current_generation() {
    let (env, _admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_n(&env, 4);
        set_index_generation(&env, 7);

        let page1 = TrustBridgeContract::get_public_paginated(env.clone(), None, 2).unwrap();
        let issued = page1.next_cursor.clone().unwrap();
        assert_eq!(issued, forge_cursor(&env, 2, 7));

        let page2 =
            TrustBridgeContract::get_public_paginated(env.clone(), Some(issued), 2).unwrap();
        assert_eq!(page2.records.len(), 2);
        assert_eq!(page2.records.get(0).unwrap().0, s(&env, "user002"));
    });
}

/// A cursor from any generation other than the current one — older or
/// newer, adjacent or far — is rejected even when its offset is in range.
#[test]
fn test_public_paginated_rejects_cursor_from_any_other_generation() {
    let (env, _admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_n(&env, 4);
        set_index_generation(&env, 5);

        for generation in [0, 4, 6, 1_000, u32::MAX] {
            assert_eq!(
                TrustBridgeContract::get_public_paginated(
                    env.clone(),
                    Some(forge_cursor(&env, 1, generation)),
                    2,
                ),
                Err(ContractError::InvalidCursor),
                "generation {generation} must not decode against generation 5"
            );
        }
        // The rejection is the only effect: the current generation still works.
        assert!(TrustBridgeContract::get_public_paginated(
            env.clone(),
            Some(forge_cursor(&env, 1, 5)),
            2
        )
        .is_ok());
    });
}

/// The generation must keep changing at `u32::MAX`. A saturating bump would
/// leave a cursor issued at `u32::MAX` valid after a removal shifted the
/// index, silently skipping a record.
#[test]
fn test_generation_wraps_at_u32_max_and_still_invalidates_cursors() {
    let (env, admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_n(&env, 4);
        set_index_generation(&env, u32::MAX);

        let page1 = TrustBridgeContract::get_public_paginated(env.clone(), None, 2).unwrap();
        let issued = page1.next_cursor.clone().unwrap();
        assert_eq!(issued, forge_cursor(&env, 2, u32::MAX));

        TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, "user000")).unwrap();

        assert_eq!(
            TrustBridgeContract::get_public_paginated(env.clone(), Some(issued), 2),
            Err(ContractError::InvalidCursor)
        );
        let restarted = TrustBridgeContract::get_public_paginated(env.clone(), None, 2).unwrap();
        assert_eq!(restarted.next_cursor.unwrap(), forge_cursor(&env, 2, 0));
    });
}

// ── get_pending_reverify (Issue #587) ────────────────────────────────────────

/// Registers and verifies `name`, then re-registers it to a new address so the
/// contract flags it for re-verification. Each call runs in its own frame so
/// the admin auth is not reused.
fn flag_reverify(env: &Env, contract_id: &Address, admin: &Address, name: &str) {
    let register = || {
        env.as_contract(contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                s(env, name),
                Address::generate(env),
                Vec::new(env),
            )
            .unwrap();
        })
    };
    register();
    env.as_contract(contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(env, name)).unwrap();
    });
    register();
}

#[test]
fn test_get_pending_reverify_empty_registry_returns_empty_page() {
    let (env, _admin, contract_id) = setup();
    env.as_contract(&contract_id, || {
        let page = TrustBridgeContract::get_pending_reverify(env.clone(), 0, 10).unwrap();
        assert_eq!(page.len(), 0);
    });
}

#[test]
fn test_get_pending_reverify_two_page_walk_and_clear_on_verify() {
    let (env, admin, contract_id) = setup();
    env.mock_all_auths();
    flag_reverify(&env, &contract_id, &admin, "carol");
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "bob"),
            Address::generate(&env),
            Vec::new(&env),
        )
        .unwrap();
    });
    flag_reverify(&env, &contract_id, &admin, "dave");
    env.as_contract(&contract_id, || {
        let first = TrustBridgeContract::get_pending_reverify(env.clone(), 0, 2).unwrap();
        assert_eq!(first, Vec::from_array(&env, [s(&env, "carol")]));
        let second = TrustBridgeContract::get_pending_reverify(env.clone(), 2, 2).unwrap();
        assert_eq!(second, Vec::from_array(&env, [s(&env, "dave")]));
        let past_end = TrustBridgeContract::get_pending_reverify(env.clone(), 4, 2).unwrap();
        assert_eq!(past_end.len(), 0);
    });

    // Re-verifying clears the flag, so the name drops out of the page.
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, "carol")).unwrap();
    });
    env.as_contract(&contract_id, || {
        let after = TrustBridgeContract::get_pending_reverify(env.clone(), 0, 100).unwrap();
        assert_eq!(after, Vec::from_array(&env, [s(&env, "dave")]));
    });
}

#[test]
fn test_get_pending_reverify_limit_above_max_is_rejected() {
    let (env, _admin, contract_id) = setup();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_pending_reverify(env.clone(), 0, 101),
            Err(ContractError::InvalidBatchSize)
        );
    });
}

#[test]
fn test_get_pending_reverify_requires_initialized() {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_pending_reverify(env.clone(), 0, 10),
            Err(ContractError::NotInitialized)
        );
    });
}
