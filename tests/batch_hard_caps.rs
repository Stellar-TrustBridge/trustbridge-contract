//! Boundary tests for the batch hard caps documented in `src/batch.rs`,
//! `docs/ABI.md`, and `docs/BENCHMARK_BUDGETS.md` (Issue #685).
//!
//! `MAX_WRITE_BATCH` bounds `batch_verify` / `batch_remove`;
//! `BatchConfig::default().max_batch_size` bounds `extend_registry_ttl`.
//! Each cap must accept exactly its ceiling and reject ceiling + 1 and zero
//! with `InvalidBatchSize` before any state is written.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};
use trustbridge_contract::{BatchConfig, ContractError, TrustBridgeContract, MAX_WRITE_BATCH};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    // A full 25-entry `batch_verify` touches ~155 footprint entries / ~77
    // writes, above the default per-invocation limits (100 / 50) the test
    // host enforces. These tests pin the size-cap *behaviour*, which is
    // independent of that network limit (see docs/BENCHMARK_BUDGETS.md).
    env.cost_estimate().disable_resource_limits();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    env.mock_all_auths();
    (env, admin, contract_id)
}

fn names(env: &Env, count: u32) -> Vec<String> {
    let mut out = Vec::new(env);
    for i in 0..count {
        out.push_back(String::from_str(env, &std::format!("cap-user-{i:03}")));
    }
    out
}

fn register_all(env: &Env, contract_id: &Address, usernames: &Vec<String>) {
    for username in usernames.iter() {
        env.as_contract(contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username,
                Address::generate(env),
                Vec::new(env),
            )
            .unwrap();
        });
    }
}

#[test]
fn documented_caps_are_stable() {
    assert_eq!(MAX_WRITE_BATCH, 25);
    assert_eq!(BatchConfig::for_writes().max_batch_size, MAX_WRITE_BATCH);
    assert_eq!(BatchConfig::default().max_batch_size, 100);
}

#[test]
fn batch_verify_accepts_exactly_max_write_batch() {
    let (env, admin, contract_id) = setup();
    let usernames = names(&env, MAX_WRITE_BATCH);
    register_all(&env, &contract_id, &usernames);

    env.as_contract(&contract_id, || {
        let summary =
            TrustBridgeContract::batch_verify(env.clone(), admin.clone(), usernames).unwrap();
        assert_eq!(summary.total, MAX_WRITE_BATCH);
        assert_eq!(summary.successful, MAX_WRITE_BATCH);
        assert_eq!(
            TrustBridgeContract::get_verified_count(env.clone()),
            MAX_WRITE_BATCH
        );
    });
}

#[test]
fn batch_verify_rejects_max_write_batch_plus_one_without_writes() {
    let (env, admin, contract_id) = setup();
    let usernames = names(&env, MAX_WRITE_BATCH + 1);
    register_all(&env, &contract_id, &usernames);

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::batch_verify(env.clone(), admin.clone(), usernames),
            Err(ContractError::InvalidBatchSize)
        );
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 0);
    });
}

#[test]
fn batch_remove_accepts_exactly_max_write_batch() {
    let (env, admin, contract_id) = setup();
    let usernames = names(&env, MAX_WRITE_BATCH);
    register_all(&env, &contract_id, &usernames);

    env.as_contract(&contract_id, || {
        let summary =
            TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames).unwrap();
        assert_eq!(summary.successful, MAX_WRITE_BATCH);
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 0);
    });
}

#[test]
fn batch_remove_rejects_max_write_batch_plus_one_without_writes() {
    let (env, admin, contract_id) = setup();
    let usernames = names(&env, MAX_WRITE_BATCH + 1);
    register_all(&env, &contract_id, &usernames);

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames),
            Err(ContractError::InvalidBatchSize)
        );
        assert_eq!(
            TrustBridgeContract::get_stats(env.clone()).total,
            MAX_WRITE_BATCH + 1
        );
    });
}

#[test]
fn write_batches_reject_empty_input() {
    let (env, admin, contract_id) = setup();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::batch_verify(env.clone(), admin.clone(), Vec::new(&env)),
            Err(ContractError::InvalidBatchSize)
        );
        assert_eq!(
            TrustBridgeContract::batch_remove(env.clone(), admin.clone(), Vec::new(&env)),
            Err(ContractError::InvalidBatchSize)
        );
    });
}

#[test]
fn extend_registry_ttl_rejects_above_default_cap() {
    let (env, _admin, contract_id) = setup();
    let cap = BatchConfig::default().max_batch_size;
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::extend_registry_ttl(env.clone(), names(&env, cap + 1)),
            Err(ContractError::InvalidBatchSize)
        );
        assert_eq!(
            TrustBridgeContract::extend_registry_ttl(env.clone(), Vec::new(&env)),
            Err(ContractError::InvalidBatchSize)
        );
    });
}
