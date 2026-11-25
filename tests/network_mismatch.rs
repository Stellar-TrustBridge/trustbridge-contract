//! Network-id mismatch tests (Issue #231 / #401).
//!
//! Ensures that attestation-required setters (`set_attestation_required`) and
//! upgrade attestation operations (`attest_upgrade`, `clear_attestation`,
//! `adopt_network_tag`) honor the instance's recorded `network_id` and fail
//! closed with `ContractError::NetworkMismatch` (code 30) when invoked on a
//! different network.

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, BytesN, Env,
};

use trustbridge_contract::{ContractError, TrustBridgeContract, TrustBridgeContractClient};

const NET_A: [u8; 32] = [0x11; 32];
const NET_B: [u8; 32] = [0x22; 32];

fn setup_on_network(network_id: [u8; 32]) -> (Env, Address, Address) {
    let env = Env::default();
    env.ledger().set_network_id(network_id);
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    (env, admin, contract_id)
}

fn wasm_hash(env: &Env, byte: u8) -> BytesN<32> {
    BytesN::from_array(env, &[byte; 32])
}

// ── Happy path ───────────────────────────────────────────────────────────────

#[test]
fn test_set_attestation_required_succeeds_on_matching_network() {
    let (env, _admin, contract_id) = setup_on_network(NET_A);
    let client = TrustBridgeContractClient::new(&env, &contract_id);

    assert!(!client.is_attestation_required());
    assert_eq!(
        client.get_network_tag(),
        Some(BytesN::from_array(&env, &NET_A))
    );

    env.mock_all_auths();
    client.set_attestation_required(&true);
    assert!(client.is_attestation_required());

    // Preserves pause-exempt behavior so operators can tighten upgrade controls
    // during an incident pause.
    client.pause(&1u32);
    client.set_attestation_required(&false);
    assert!(!client.is_attestation_required());
    client.set_attestation_required(&true);
    assert!(client.is_attestation_required());
}

#[test]
fn test_attest_and_clear_succeed_on_matching_network() {
    let (env, _admin, contract_id) = setup_on_network(NET_A);
    let client = TrustBridgeContractClient::new(&env, &contract_id);
    env.ledger().set_timestamp(1_000);
    let hash = wasm_hash(&env, 0xAA);

    env.mock_all_auths();
    client.attest_upgrade(&hash, &2_000u64);
    assert_eq!(client.get_attestation().unwrap().wasm_hash, hash);

    client.clear_attestation();
    assert!(client.get_attestation().is_none());

    // Re-adopting the same network tag is an idempotent no-op.
    client.adopt_network_tag();
}

// ── Failure path 1: cross-network mismatch ───────────────────────────────────

#[test]
fn test_set_attestation_required_rejects_mismatched_network_without_mutating() {
    let (env, _admin, contract_id) = setup_on_network(NET_A);

    // Default is false on NET_A. Switching to NET_B must fail closed and
    // must not enable or toggle the flag.
    env.ledger().set_network_id(NET_B);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::set_attestation_required(env.clone(), true),
            Err(ContractError::NetworkMismatch)
        );
        assert!(
            !TrustBridgeContract::is_attestation_required(env.clone()),
            "failed cross-network set_attestation_required(true) must not mutate state"
        );
    });

    // Enable on NET_A, then verify NET_B cannot weaken/disable it.
    env.ledger().set_network_id(NET_A);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_attestation_required(env.clone(), true).unwrap();
        assert!(TrustBridgeContract::is_attestation_required(env.clone()));
    });

    env.ledger().set_network_id(NET_B);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::set_attestation_required(env.clone(), false),
            Err(ContractError::NetworkMismatch)
        );
        assert!(
            TrustBridgeContract::is_attestation_required(env.clone()),
            "cross-network caller must not be able to disable attestation_required"
        );
    });
}

#[test]
fn test_attest_upgrade_and_clear_attestation_reject_mismatched_network() {
    let (env, _admin, contract_id) = setup_on_network(NET_A);
    env.ledger().set_timestamp(1_000);
    let hash_a = wasm_hash(&env, 0x0A);
    let hash_b = wasm_hash(&env, 0x0B);

    // Publish an attestation on NET_A first.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::attest_upgrade(env.clone(), hash_a.clone(), 5_000).unwrap();
    });

    // Switch to NET_B: neither attest_upgrade, clear_attestation, nor
    // adopt_network_tag may mutate state.
    env.ledger().set_network_id(NET_B);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::attest_upgrade(env.clone(), hash_b, 5_000),
            Err(ContractError::NetworkMismatch)
        );
        assert_eq!(
            TrustBridgeContract::clear_attestation(env.clone()),
            Err(ContractError::NetworkMismatch)
        );
        assert_eq!(
            TrustBridgeContract::adopt_network_tag(env.clone()),
            Err(ContractError::NetworkMismatch)
        );
        assert_eq!(
            TrustBridgeContract::get_attestation(env.clone())
                .expect("NET_A attestation must remain intact")
                .wasm_hash,
            hash_a
        );
        assert_eq!(
            TrustBridgeContract::get_network_tag(env.clone()),
            Some(BytesN::from_array(&env, &NET_A))
        );
    });
}

// ── Failure path 2: uninitialized contract ───────────────────────────────────

#[test]
fn test_attestation_setters_reject_uninitialized_contract() {
    let env = Env::default();
    env.ledger().set_network_id(NET_A);
    let contract_id = env.register(TrustBridgeContract, ());
    let hash = wasm_hash(&env, 0xCC);
    env.ledger().set_timestamp(1_000);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::set_attestation_required(env.clone(), true),
            Err(ContractError::NotInitialized)
        );
        assert_eq!(
            TrustBridgeContract::attest_upgrade(env.clone(), hash, 2_000),
            Err(ContractError::NotInitialized)
        );
        assert_eq!(
            TrustBridgeContract::clear_attestation(env.clone()),
            Err(ContractError::NotInitialized)
        );
        assert!(!TrustBridgeContract::is_attestation_required(env.clone()));
    });
}

// ── Failure path 3: unauthorized caller & required-mode upgrade gate ─────────

#[test]
fn test_set_attestation_required_rejects_missing_admin_auth() {
    let (env, _admin, contract_id) = setup_on_network(NET_A);
    let client = TrustBridgeContractClient::new(&env, &contract_id);

    // Clear all mocked auths so admin signature is absent.
    env.set_auths(&[]);
    let res = client.try_set_attestation_required(&true);
    assert!(
        res.is_err(),
        "set_attestation_required without admin auth must fail"
    );

    assert!(
        !client.is_attestation_required(),
        "unauthorized call must leave attestation_required unchanged"
    );
}

#[test]
fn test_upgrade_without_attestation_fails_when_required() {
    let (env, _admin, contract_id) = setup_on_network(NET_A);
    let client = TrustBridgeContractClient::new(&env, &contract_id);
    let hash = wasm_hash(&env, 0xDD);

    env.mock_all_auths();
    client.set_attestation_required(&true);

    let res = client.try_upgrade(&hash);
    assert_eq!(res, Err(Ok(ContractError::AttestationRequired)));
}
