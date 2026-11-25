//! Tests for the admin `repair_index` operation.
//!
//! `repair_index` recomputes `count` / `verified` from the chunked username
//! index and each entry's stored record, independent of the counters
//! themselves. These tests deliberately drift the on-chain counters by
//! writing directly to instance storage — the only way the counters can
//! realistically drift in this contract, since every public mutation keeps
//! them in lockstep — and then check that a dry run reports the drift
//! without writing, and that `apply = true` corrects it. See
//! `docs/SECURITY.md#index-repair-repair_index`.
//!
//! Interaction note (Issue #857): `repair_index` deliberately touches only
//! `count` / `verified`. These tests also set a `PENDING_REVERIFY_KEY` flag
//! directly in storage and prove it survives dry-run, apply, a rejected
//! unauthenticated call, and repeated no-op applies — operators must not use
//! index repair to hide a Wave address-change re-verification risk.

#![cfg(test)]

use soroban_sdk::{symbol_short, testutils::Address as _, Address, Env, String};

use trustbridge_contract::{ContractError, TrustBridgeContract};

fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let user1 = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });

    (env, admin, user1, contract_id)
}

fn s(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

/// Overwrites the stored `count` / `verified` counters directly, bypassing
/// every public entry point — this is the drifted fixture.
fn corrupt_counters(env: &Env, contract_id: &Address, count: u32, verified: u32) {
    env.as_contract(contract_id, || {
        env.storage()
            .instance()
            .set(&symbol_short!("count"), &count);
        env.storage()
            .instance()
            .set(&symbol_short!("vcount"), &verified);
    });
}

/// Sets a pending re-verify flag exactly as `set_pending_reverify` does,
/// without going through a public entry point that would clear it.
fn set_pending_reverify_flag(env: &Env, contract_id: &Address, username: &str) {
    env.as_contract(contract_id, || {
        env.storage()
            .persistent()
            .set(&(symbol_short!("pend_rev"), s(env, username)), &true);
    });
}

fn pending_reverify_flag(env: &Env, contract_id: &Address, username: &str) -> bool {
    env.as_contract(contract_id, || {
        env.storage()
            .persistent()
            .get::<_, bool>(&(symbol_short!("pend_rev"), s(env, username)))
            .unwrap_or(false)
    })
}

#[test]
fn test_repair_index_preserves_pending_reverify_flag_across_dry_run_and_apply() {
    let (env, _admin, user1, contract_id) = setup();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "alice"),
            user1,
            soroban_sdk::Vec::new(&env),
        )
        .unwrap();
    });
    set_pending_reverify_flag(&env, &contract_id, "alice");
    corrupt_counters(&env, &contract_id, 9, 9);

    env.mock_all_auths();
    let dry = env.as_contract(&contract_id, || {
        TrustBridgeContract::repair_index(env.clone(), false).unwrap()
    });
    assert!(dry.drifted);
    assert!(!dry.applied);
    assert!(pending_reverify_flag(&env, &contract_id, "alice"));

    let applied = env.as_contract(&contract_id, || {
        TrustBridgeContract::repair_index(env.clone(), true).unwrap()
    });
    assert!(applied.applied);
    assert!(pending_reverify_flag(&env, &contract_id, "alice"));
    let stats = env.as_contract(&contract_id, || TrustBridgeContract::get_stats(env.clone()));
    assert_eq!((stats.total, stats.verified), (1, 0));
}

#[test]
fn test_repair_index_rejection_leaves_pending_reverify_flag_intact() {
    let (env, _admin, user1, contract_id) = setup();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "alice"),
            user1,
            soroban_sdk::Vec::new(&env),
        )
        .unwrap();
    });
    set_pending_reverify_flag(&env, &contract_id, "alice");
    corrupt_counters(&env, &contract_id, 7, 7);

    // Failure path 1: a dry run reports drift but must not repair or clear
    // the flag (a deliberately non-applying call).
    env.mock_all_auths();
    let dry = env.as_contract(&contract_id, || {
        TrustBridgeContract::repair_index(env.clone(), false).unwrap()
    });
    assert!(dry.drifted && !dry.applied);
    assert!(pending_reverify_flag(&env, &contract_id, "alice"));
    let stats = env.as_contract(&contract_id, || TrustBridgeContract::get_stats(env.clone()));
    assert_eq!(
        (stats.total, stats.verified),
        (7, 7),
        "dry run must not write"
    );

    // Failure path 2: uninitialized contract fails before any write.
    let fresh = env.register(TrustBridgeContract, ());
    let err = env.as_contract(&fresh, || {
        TrustBridgeContract::repair_index(env.clone(), true)
    });
    assert!(matches!(err, Err(ContractError::NotInitialized)));

    // A later apply still works after the rejection, and repeated no-op
    // applies with the flag set are idempotent.
    env.mock_all_auths();
    let repair = env.as_contract(&contract_id, || {
        TrustBridgeContract::repair_index(env.clone(), true).unwrap()
    });
    assert!(repair.applied);
    for _ in 0..2 {
        let noop = env.as_contract(&contract_id, || {
            TrustBridgeContract::repair_index(env.clone(), true).unwrap()
        });
        assert!(!noop.applied, "no drift must remain a no-op");
        assert!(pending_reverify_flag(&env, &contract_id, "alice"));
    }
}

#[test]
fn test_repair_index_dry_run_reports_no_drift_on_healthy_registry() {
    let (env, admin, user1, contract_id) = setup();
    let user2 = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "alice"),
            user1,
            soroban_sdk::Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "bob"),
            user2,
            soroban_sdk::Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, "alice")).unwrap();
    });

    env.mock_all_auths();
    let report = env.as_contract(&contract_id, || {
        TrustBridgeContract::repair_index(env.clone(), false).unwrap()
    });

    assert!(!report.drifted, "a healthy registry must report no drift");
    assert_eq!(report.stored_count, 2);
    assert_eq!(report.recomputed_count, 2);
    assert_eq!(report.stored_verified, 1);
    assert_eq!(report.recomputed_verified, 1);
}

#[test]
fn test_repair_index_dry_run_detects_drift_without_writing() {
    let (env, admin, user1, contract_id) = setup();
    let user2 = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "alice"),
            user1,
            soroban_sdk::Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "bob"),
            user2,
            soroban_sdk::Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, "alice")).unwrap();
    });

    // Drift the counters directly — real records/chunks still say 2 total, 1 verified.
    corrupt_counters(&env, &contract_id, 99, 99);

    env.mock_all_auths();
    let report = env.as_contract(&contract_id, || {
        TrustBridgeContract::repair_index(env.clone(), false).unwrap()
    });

    assert!(report.drifted);
    assert_eq!(report.stored_count, 99);
    assert_eq!(report.recomputed_count, 2);
    assert_eq!(report.stored_verified, 99);
    assert_eq!(report.recomputed_verified, 1);

    // Confirm nothing was actually written: the corrupted values persist.
    env.mock_all_auths();
    let stats = env.as_contract(&contract_id, || TrustBridgeContract::get_stats(env.clone()));
    assert_eq!(stats.total, 99);
    assert_eq!(stats.verified, 99);
}

#[test]
fn test_repair_index_apply_true_corrects_drifted_counters() {
    let (env, admin, user1, contract_id) = setup();
    let user2 = Address::generate(&env);
    let user3 = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "alice"),
            user1,
            soroban_sdk::Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "bob"),
            user2,
            soroban_sdk::Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "carol"),
            user3,
            soroban_sdk::Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, "alice")).unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, "bob")).unwrap();
    });

    corrupt_counters(&env, &contract_id, 0, 0);

    env.mock_all_auths();
    let report = env.as_contract(&contract_id, || {
        TrustBridgeContract::repair_index(env.clone(), true).unwrap()
    });

    assert!(report.drifted);
    assert_eq!(report.recomputed_count, 3);
    assert_eq!(report.recomputed_verified, 2);

    env.mock_all_auths();
    let stats = env.as_contract(&contract_id, || TrustBridgeContract::get_stats(env.clone()));
    assert_eq!(stats.total, 3, "count must be corrected on chain");
    assert_eq!(stats.verified, 2, "verified must be corrected on chain");

    // A second apply-true call against the now-healthy registry must be a no-op.
    env.mock_all_auths();
    let second = env.as_contract(&contract_id, || {
        TrustBridgeContract::repair_index(env.clone(), true).unwrap()
    });
    assert!(!second.drifted);
}

#[test]
fn test_repair_index_dry_run_never_writes_even_when_only_one_counter_drifts() {
    // Drift only `verified` while `count` happens to already be correct —
    // a dry run must still leave BOTH counters untouched, not just skip
    // writing the one that matches. This guards against an implementation
    // that only short-circuits the write when every counter is already
    // correct, rather than never writing at all in dry-run mode.
    let (env, admin, user1, contract_id) = setup();
    let user2 = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "alice"),
            user1,
            soroban_sdk::Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "bob"),
            user2,
            soroban_sdk::Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, "alice")).unwrap();
    });

    // `count` (2) is correct; only `verified` is drifted (claims 0 verified
    // instead of the real 1).
    corrupt_counters(&env, &contract_id, 2, 0);

    env.mock_all_auths();
    let report = env.as_contract(&contract_id, || {
        TrustBridgeContract::repair_index(env.clone(), false).unwrap()
    });
    assert!(report.drifted);
    assert_eq!(report.recomputed_verified, 1);

    // Both stored counters must be exactly as corrupted — dry-run never
    // writes any field, not even the one it recomputed to the same value.
    env.mock_all_auths();
    let stats = env.as_contract(&contract_id, || TrustBridgeContract::get_stats(env.clone()));
    assert_eq!(stats.total, 2, "count must stay untouched by dry run");
    assert_eq!(
        stats.verified, 0,
        "drifted verified counter must stay untouched by dry run"
    );

    // Run several more dry-run calls in a row: repeated dry runs must be
    // idempotent no-ops, never accumulating partial writes.
    for _ in 0..3 {
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::repair_index(env.clone(), false).unwrap()
        });
    }
    env.mock_all_auths();
    let stats_after = env.as_contract(&contract_id, || TrustBridgeContract::get_stats(env.clone()));
    assert_eq!(stats_after.total, 2);
    assert_eq!(stats_after.verified, 0);
}

#[test]
fn test_repair_index_apply_is_atomic_per_counter_no_partial_update() {
    // Corrupt `count` and `verified` to two independently wrong values and
    // confirm a single apply=true call corrects BOTH together. If apply
    // were not atomic per counter (e.g. wrote `count` then returned early
    // before writing `verified` on some error path), this would observe a
    // half-corrected state: one counter fixed, the other still drifted.
    let (env, admin, user1, contract_id) = setup();
    let user2 = Address::generate(&env);
    let user3 = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "alice"),
            user1,
            soroban_sdk::Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "bob"),
            user2,
            soroban_sdk::Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "carol"),
            user3,
            soroban_sdk::Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, "alice")).unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, "bob")).unwrap();
    });

    // Drift both counters to distinct wrong values in opposite directions
    // (count too low, verified too high) so a single-field fix would be
    // immediately visible as a half-applied state.
    corrupt_counters(&env, &contract_id, 1, 99);

    env.mock_all_auths();
    let report = env.as_contract(&contract_id, || {
        TrustBridgeContract::repair_index(env.clone(), true).unwrap()
    });
    assert!(report.drifted);

    // Both counters must land on their recomputed values in the SAME call —
    // never one corrected while the other remains at its corrupted value.
    env.mock_all_auths();
    let stats = env.as_contract(&contract_id, || TrustBridgeContract::get_stats(env.clone()));
    assert_eq!(stats.total, 3, "count must be corrected");
    assert_eq!(
        stats.verified, 2,
        "verified must be corrected in the same apply call"
    );
    assert_ne!(
        stats.total, 1,
        "count must not be left at its pre-repair corrupted value"
    );
    assert_ne!(
        stats.verified, 99,
        "verified must not be left at its pre-repair corrupted value"
    );
}

#[test]
fn test_repair_index_requires_admin_auth() {
    let (env, _admin, _user1, contract_id) = setup();

    // No auths mocked: a non-admin (or unauthenticated) caller must fail
    // the admin `require_auth()` check inside `repair_index`.
    let result = env.as_contract(&contract_id, || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            TrustBridgeContract::repair_index(env.clone(), false)
        }))
    });
    assert!(
        result.is_err(),
        "repair_index must panic on missing admin auth, matching every other admin-only call"
    );
}
