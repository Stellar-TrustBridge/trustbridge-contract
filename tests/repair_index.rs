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

#![cfg(test)]

use soroban_sdk::{symbol_short, testutils::Address as _, Address, Env, String};

use trustbridge_contract::TrustBridgeContract;

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
        env.storage().instance().set(&symbol_short!("count"), &count);
        env.storage().instance().set(&symbol_short!("vcount"), &verified);
    });
}

#[test]
fn test_repair_index_dry_run_reports_no_drift_on_healthy_registry() {
    let (env, admin, user1, contract_id) = setup();
    let user2 = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), s(&env, "alice"), user1, soroban_sdk::Vec::new(&env))
            .unwrap();
        TrustBridgeContract::register(env.clone(), s(&env, "bob"), user2, soroban_sdk::Vec::new(&env))
            .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, "alice")).unwrap();
    });

    env.mock_all_auths();
    let report = env.as_contract(&contract_id, || {
        TrustBridgeContract::repair_index(env.clone(), false).unwrap()
    });

    assert!(!report.drifted, "a healthy registry must report no drift");
    assert!(!report.applied, "a dry run must never apply, even with no drift");
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
        TrustBridgeContract::register(env.clone(), s(&env, "alice"), user1, soroban_sdk::Vec::new(&env))
            .unwrap();
        TrustBridgeContract::register(env.clone(), s(&env, "bob"), user2, soroban_sdk::Vec::new(&env))
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
    assert!(!report.applied, "dry run (apply = false) must not write");
    assert_eq!(report.stored_count, 99);
    assert_eq!(report.recomputed_count, 2);
    assert_eq!(report.stored_verified, 99);
    assert_eq!(report.recomputed_verified, 1);

    // Confirm nothing was actually written: the corrupted values persist.
    env.mock_all_auths();
    let stats = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_stats(env.clone())
    });
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
        TrustBridgeContract::register(env.clone(), s(&env, "alice"), user1, soroban_sdk::Vec::new(&env))
            .unwrap();
        TrustBridgeContract::register(env.clone(), s(&env, "bob"), user2, soroban_sdk::Vec::new(&env))
            .unwrap();
        TrustBridgeContract::register(env.clone(), s(&env, "carol"), user3, soroban_sdk::Vec::new(&env))
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
    assert!(report.applied);
    assert_eq!(report.recomputed_count, 3);
    assert_eq!(report.recomputed_verified, 2);

    env.mock_all_auths();
    let stats = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_stats(env.clone())
    });
    assert_eq!(stats.total, 3, "count must be corrected on chain");
    assert_eq!(stats.verified, 2, "verified must be corrected on chain");

    // A second apply-true call against the now-healthy registry must be a no-op.
    env.mock_all_auths();
    let second = env.as_contract(&contract_id, || {
        TrustBridgeContract::repair_index(env.clone(), true).unwrap()
    });
    assert!(!second.drifted);
    assert!(!second.applied, "repairing an already-healthy registry must not write");
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
