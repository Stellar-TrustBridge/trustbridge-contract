//! Admin-gated oracle signer allowlist rotation (Issue #679).
//!
//! `set_oracle_allowlist` replaces the allowlist, requires admin auth, and
//! emits `OracleAllowlistRotatedEvent`. See `docs/SECURITY.md`.

#![cfg(test)]

use soroban_sdk::testutils::{Address as _, Events as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{vec, Address, BytesN, Env, Event as _, IntoVal};

use trustbridge_contract::{
    ContractError, OracleAllowlistRotatedEvent, TrustBridgeContract, TrustBridgeContractClient,
};

fn setup() -> (Env, Address, TrustBridgeContractClient<'static>) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    let client = TrustBridgeContractClient::new(&env, &contract_id);
    env.mock_all_auths();
    client.initialize(&admin);
    (env, admin, client)
}

fn key(env: &Env, byte: u8) -> BytesN<32> {
    BytesN::from_array(env, &[byte; 32])
}

#[test]
fn test_admin_rotation_replaces_allowlist_and_emits_event() {
    let (env, admin, client) = setup();

    client.set_oracle_allowlist(&vec![&env, key(&env, 1)]);
    let rotated = vec![&env, key(&env, 2), key(&env, 3)];
    client.set_oracle_allowlist(&rotated);
    assert_eq!(client.get_oracle_allowlist(), rotated);

    let expected = OracleAllowlistRotatedEvent {
        rotated_by: admin,
        key_count: 2,
        timestamp: env.ledger().timestamp(),
    };
    // `events().all()` holds only the last top-level invocation's events.
    assert_eq!(
        env.events().all(),
        vec![
            &env,
            (
                client.address.clone(),
                expected.topics(&env),
                expected.data(&env),
            )
        ]
    );
}

#[test]
fn test_empty_rotation_disables_oracle_and_emits_event() {
    let (env, _admin, client) = setup();
    client.set_oracle_allowlist(&vec![&env, key(&env, 1)]);
    client.set_oracle_allowlist(&vec![&env]);
    assert_eq!(client.get_oracle_allowlist().len(), 0);
    assert_eq!(env.events().all().len(), 1);
}

#[test]
fn test_non_admin_cannot_rotate_allowlist() {
    let (env, _admin, client) = setup();
    client.set_oracle_allowlist(&vec![&env, key(&env, 1)]);

    let attacker = Address::generate(&env);
    let new_keys = vec![&env, key(&env, 9)];
    env.mock_auths(&[MockAuth {
        address: &attacker,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "set_oracle_allowlist",
            args: (new_keys.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_set_oracle_allowlist(&new_keys).is_err());
    assert_eq!(client.get_oracle_allowlist(), vec![&env, key(&env, 1)]);
}

#[test]
fn test_rotation_requires_initialization() {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());
    let client = TrustBridgeContractClient::new(&env, &contract_id);
    env.mock_all_auths();
    assert_eq!(
        client.try_set_oracle_allowlist(&vec![&env, key(&env, 1)]),
        Err(Ok(ContractError::NotInitialized))
    );
}
