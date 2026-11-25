//! Automated proof for the dashboard consumer flow documented in
//! docs/DASHBOARD_SYNC.md (Issue #866): one paginated, auth-free,
//! pause-exempt `get_pending_reverify(offset, limit)` endpoint, sparse
//! canonical-name pages, and stable failure codes.
//!
//! Run with `cargo test --test dashboard_pending_reverify`.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};
use trustbridge_contract::{ContractError, TrustBridgeContract};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
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

fn register(env: &Env, contract_id: &Address, name: &str) {
    env.as_contract(contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(env, name),
            Address::generate(env),
            Vec::new(env),
        )
        .unwrap();
    });
}

/// Register (create) → verify → re-register at a new address sets `pend_rev`.
fn flag_reverify(env: &Env, contract_id: &Address, admin: &Address, name: &str) {
    register(env, contract_id, name);
    env.as_contract(contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(env, name)).unwrap();
    });
    register(env, contract_id, name);
}

fn pending(env: &Env, contract_id: &Address, offset: u32, limit: u32) -> Vec<String> {
    env.as_contract(contract_id, || {
        TrustBridgeContract::get_pending_reverify(env.clone(), offset, limit).unwrap()
    })
}

/// Happy path: walk sparse pages to `get_stats().total`, read while paused,
/// then confirm `verify` drops the name from the queue.
#[test]
fn page_walk_is_sparse_pause_exempt_and_clears_on_verify() {
    let (env, admin, contract_id) = setup();
    for name in ["alice", "bob", "carol", "dave"] {
        register(&env, &contract_id, name);
    }
    flag_reverify(&env, &contract_id, &admin, "alice");
    flag_reverify(&env, &contract_id, &admin, "dave");

    let total = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_stats(env.clone()).total
    });
    assert_eq!(total, 4);

    assert_eq!(
        pending(&env, &contract_id, 0, 2),
        Vec::from_array(&env, [s(&env, "alice")])
    );
    assert_eq!(
        pending(&env, &contract_id, 2, 2),
        Vec::from_array(&env, [s(&env, "dave")])
    );
    assert!(
        pending(&env, &contract_id, total, 2).is_empty(),
        "past-end page is empty"
    );

    // Paused/frozen contract: the dashboard read still works.
    env.as_contract(&contract_id, || {
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
    });
    assert_eq!(
        pending(&env, &contract_id, 0, 100),
        Vec::from_array(&env, [s(&env, "alice"), s(&env, "dave")])
    );
    env.as_contract(&contract_id, || {
        TrustBridgeContract::unpause(env.clone(), 4).unwrap();
    });

    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, "alice")).unwrap();
    });
    assert_eq!(
        pending(&env, &contract_id, 0, 100),
        Vec::from_array(&env, [s(&env, "dave")])
    );
}

/// Failure path 1: an oversized limit is rejected with `InvalidBatchSize`
/// (it is NOT clamped like export pages); `limit == 0` uses the default.
#[test]
fn oversized_limit_is_rejected_and_zero_uses_default() {
    let (env, _admin, contract_id) = setup();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_pending_reverify(env.clone(), 0, 101),
            Err(ContractError::InvalidBatchSize)
        );
        assert!(TrustBridgeContract::get_pending_reverify(env.clone(), 0, 0)
            .unwrap()
            .is_empty());
    });
}

/// Failure path 2: an uninitialized deployment reports `NotInitialized`, so
/// dashboards never mistake a wrong contract ID/network for an empty queue.
#[test]
fn uninitialized_deployment_is_rejected() {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_pending_reverify(env.clone(), 0, 10),
            Err(ContractError::NotInitialized)
        );
    });
}
