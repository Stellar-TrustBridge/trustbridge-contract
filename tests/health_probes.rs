//! Integration tests for post-init `get_health_probes` / `get_health` smoke checks.

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, BytesN, Env, String, Vec,
};

use trustbridge_contract::{ContractError, TrustBridgeContract, TrustBridgeContractClient};

const NET_A: [u8; 32] = [0x33; 32];
const NET_B: [u8; 32] = [0x44; 32];

fn setup() -> (Env, Address, Address, TrustBridgeContractClient<'static>) {
    let env = Env::default();
    env.ledger().set_network_id(NET_A);
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    let client = TrustBridgeContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    (env, admin, contract_id, client)
}

#[test]
fn test_get_health_probes_happy_path_after_init() {
    let (env, admin, _cid, client) = setup();

    let snapshot = client.get_health_probes();
    assert_eq!(snapshot, client.get_health());
    assert!(!snapshot.paused);
    assert_eq!(snapshot.total, 0);
    assert_eq!(snapshot.verified, 0);
    assert_eq!(snapshot.cooldown_secs, 0);
    assert_eq!(snapshot.cooldown_remaining_secs, 0);
    assert!(!snapshot.attestation_present);
    assert_eq!(snapshot.version, soroban_sdk::vec![&env, 1u32, 1u32, 0u32]);

    // Register, verify, pause, and attest -> probes reflect live state and still
    // work while paused.
    let user = Address::generate(&env);
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    client.register(&String::from_str(&env, "octocat"), &user, &Vec::new(&env));
    client.verify(&admin, &String::from_str(&env, "octocat"));
    client.attest_upgrade(&BytesN::from_array(&env, &[0xAB; 32]), &2_000u64);
    client.pause(&1u32);

    let paused_snapshot = client.get_health_probes();
    assert_eq!(paused_snapshot, client.get_health());
    assert!(paused_snapshot.paused);
    assert_eq!(paused_snapshot.total, 1);
    assert_eq!(paused_snapshot.verified, 1);
    assert!(paused_snapshot.attestation_present);

    // Expired attestation must report attestation_present == false.
    env.ledger().set_timestamp(2_000);
    let expired_snapshot = client.get_health_probes();
    assert!(!expired_snapshot.attestation_present);
}

#[test]
fn test_get_health_probes_fails_before_initialize() {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());
    let client = TrustBridgeContractClient::new(&env, &contract_id);

    assert_eq!(
        client.try_get_health_probes(),
        Err(Ok(ContractError::NotInitialized))
    );
}

#[test]
fn test_get_health_probes_fails_on_network_mismatch() {
    let (env, _admin, _cid, client) = setup();
    env.ledger().set_network_id(NET_B);

    assert_eq!(
        client.try_get_health_probes(),
        Err(Ok(ContractError::NetworkMismatch))
    );
}
