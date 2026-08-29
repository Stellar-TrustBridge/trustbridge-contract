//! Standalone verification for Issue #194 (case-fold GitHub usernames at the
//! storage-key layer).
//!
//! Kept as its own integration test file, independent of `tests/integration.rs`,
//! so it can be built and run on its own via
//! `cargo test --test username_case_fold` regardless of the state of that
//! other file.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

use trustbridge_contract::TrustBridgeContract;

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

#[test]
fn test_username_case_variant_looks_up_the_same_record() {
    let (env, _admin, contract_id) = setup();
    let alice = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), s(&env, "Alice"), alice.clone(), Vec::new(&env))
            .unwrap();

        // A case variant lookup must hit the exact same record.
        let via_lower = TrustBridgeContract::get_address(env.clone(), s(&env, "alice")).unwrap();
        let via_upper = TrustBridgeContract::get_address(env.clone(), s(&env, "ALICE")).unwrap();
        let via_mixed = TrustBridgeContract::get_address(env.clone(), s(&env, "aLiCe")).unwrap();

        assert_eq!(via_lower.stellar_address, alice);
        assert_eq!(via_upper.stellar_address, alice);
        assert_eq!(via_mixed.stellar_address, alice);

        // Only one registration exists, not one per case variant.
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 1);

        assert!(TrustBridgeContract::has_record(env.clone(), s(&env, "alice")));
        assert!(TrustBridgeContract::has_record(env.clone(), s(&env, "ALICE")));
    });
}

#[test]
fn test_registering_a_case_variant_updates_the_existing_record_not_a_new_one() {
    let (env, _admin, contract_id) = setup();
    let alice = Address::generate(&env);

    // `mock_all_auths` authorizes every signature the invocation asks for,
    // including the existing owner's — this test's job is to prove the
    // second call lands on the *same* storage key (so a real deployment's
    // `old.stellar_address.require_auth()` guard is even reached) rather than
    // silently creating a second, independent registration for the same
    // GitHub login. The auth-rejection path itself is exercised by the
    // broader `remove`/`register` negative-auth suite.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), s(&env, "alice"), alice.clone(), Vec::new(&env))
            .unwrap();
    });

    let new_owner = Address::generate(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "ALICE"),
            new_owner.clone(),
            Vec::new(&env),
        )
        .unwrap();

        // Still one registration overall: the case variant updated the
        // existing entry rather than creating a second one.
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 1);

        let record = TrustBridgeContract::get_address(env.clone(), s(&env, "alice")).unwrap();
        assert_eq!(record.stellar_address, new_owner);
    });
}

#[test]
fn test_export_and_index_show_canonical_username_form() {
    let (env, _admin, contract_id) = setup();
    let alice = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), s(&env, "Alice"), alice.clone(), Vec::new(&env))
            .unwrap();

        let page = TrustBridgeContract::get_registered_paginated(env.clone(), 0, 10).unwrap();
        assert_eq!(page.records.len(), 1);
        let (stored_username, _) = page.records.get(0).unwrap();
        assert_eq!(stored_username, s(&env, "alice"));
    });
}

#[test]
fn test_remove_by_case_variant_removes_the_canonical_record() {
    let (env, admin, contract_id) = setup();
    let alice = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), s(&env, "alice"), alice.clone(), Vec::new(&env))
            .unwrap();

        // Remove using a different case than the one used to register.
        TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, "ALICE")).unwrap();

        assert!(!TrustBridgeContract::has_record(env.clone(), s(&env, "alice")));
        assert!(!TrustBridgeContract::has_record(env.clone(), s(&env, "Alice")));
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 0);
    });
}
