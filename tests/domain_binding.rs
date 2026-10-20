//! Integration tests for `src/domain.rs` username ↔ deployment binding rules
//! (Issue #659).
//!
//! A username binding is only meaningful inside one deployment: one contract
//! instance on one network. These tests cover the rules that keep a username
//! from being claimed or replayed outside that domain:
//!
//! - every event is stamped with an [`EventDomain`] naming this contract,
//!   this network and this version;
//! - the domain differs across deployments and networks, so a replayed event
//!   or record cannot be mistaken for a local one;
//! - a username bound to one address cannot be taken over by an impersonator,
//!   including via a case variant, without the owner's signature;
//! - reserved and malformed usernames fail with stable errors;
//! - a call on the wrong network or while paused leaves no partial binding.
//!
//! # Validate command
//!
//! ```text
//! cargo test --test domain_binding
//! ```

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke},
    Address, BytesN, Env, IntoVal, String, Vec,
};
use trustbridge_contract::{
    ContractError, EventDomain, TrustBridgeContract, TrustBridgeContractClient,
    EVENT_DOMAIN_VERSION,
};

fn s(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    (env, admin, contract_id)
}

fn register(env: &Env, contract_id: &Address, username: &str, owner: &Address) {
    env.mock_all_auths();
    env.as_contract(contract_id, || {
        TrustBridgeContract::register(env.clone(), s(env, username), owner.clone(), Vec::new(env))
            .unwrap();
    });
}

fn owner_of(env: &Env, contract_id: &Address, username: &str) -> Option<Address> {
    env.as_contract(contract_id, || {
        TrustBridgeContract::get_address(env.clone(), s(env, username)).map(|r| r.stellar_address)
    })
}

// ─── Happy path: the domain names this deployment ───────────────────────────

#[test]
fn test_event_domain_matches_live_deployment() {
    let (env, _admin, contract_id) = setup();

    let (domain, tag, version) = env.as_contract(&contract_id, || {
        let version = TrustBridgeContract::version(env.clone());
        (
            EventDomain::new(&env, version),
            TrustBridgeContract::get_network_tag(env.clone()),
            version,
        )
    });

    assert_eq!(domain.contract_id, contract_id);
    assert_eq!(domain.network_id, env.ledger().network_id());
    assert_eq!(Some(domain.network_id.clone()), tag);
    assert_eq!(domain.contract_version, version);
    assert_eq!(domain.domain_version, EVENT_DOMAIN_VERSION);
}

#[test]
fn test_registration_binds_username_to_owner() {
    let (env, _admin, contract_id) = setup();
    let owner = Address::generate(&env);

    register(&env, &contract_id, "octocat", &owner);

    assert_eq!(owner_of(&env, &contract_id, "octocat"), Some(owner.clone()));
    // Lookups are case-folded: every variant resolves to the same binding.
    assert_eq!(owner_of(&env, &contract_id, "OctoCat"), Some(owner));
}

// ─── Replay across deployments / networks ───────────────────────────────────

#[test]
fn test_event_domain_differs_across_deployments() {
    let (env, _admin, first) = setup();
    let second = env.register(TrustBridgeContract, ());
    env.as_contract(&second, || {
        TrustBridgeContract::initialize(env.clone(), Address::generate(&env)).unwrap();
    });

    let d1 = env.as_contract(&first, || EventDomain::new(&env, (1, 1, 0)));
    let d2 = env.as_contract(&second, || EventDomain::new(&env, (1, 1, 0)));

    assert_eq!(d1.network_id, d2.network_id);
    assert_ne!(d1.contract_id, d2.contract_id);
    assert_ne!(d1, d2, "two deployments must never share an event domain");
}

#[test]
fn test_event_domain_differs_across_networks() {
    let (env, _admin, contract_id) = setup();
    let before = env.as_contract(&contract_id, || EventDomain::new(&env, (1, 1, 0)));

    env.ledger().with_mut(|li| li.network_id = [0xAB; 32]);
    let after = env.as_contract(&contract_id, || EventDomain::new(&env, (1, 1, 0)));

    assert_eq!(after.network_id, BytesN::from_array(&env, &[0xAB; 32]));
    assert_ne!(before, after);
}

#[test]
fn test_register_on_foreign_network_fails_closed_without_binding() {
    let (env, _admin, contract_id) = setup();
    let owner = Address::generate(&env);

    env.ledger().with_mut(|li| li.network_id = [0xCD; 32]);
    env.mock_all_auths();
    let result = env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "octocat"),
            owner.clone(),
            Vec::new(&env),
        )
    });
    assert_eq!(result, Err(ContractError::NetworkMismatch));

    // Back on the home network: nothing was written by the rejected call.
    env.ledger()
        .with_mut(|li| li.network_id = Env::default().ledger().network_id().to_array());
    assert_eq!(owner_of(&env, &contract_id, "octocat"), None);
}

// ─── Impersonation ───────────────────────────────────────────────────────────

#[test]
fn test_case_variant_takeover_requires_owner_signature() {
    let (env, _admin, contract_id) = setup();
    let owner = Address::generate(&env);
    let impostor = Address::generate(&env);
    register(&env, &contract_id, "octocat", &owner);

    let client = TrustBridgeContractClient::new(&env, &contract_id);
    let fallbacks: Vec<Address> = Vec::new(&env);
    let result = client
        .mock_auths(&[MockAuth {
            address: &impostor,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "register",
                args: (s(&env, "OctoCat"), impostor.clone(), fallbacks.clone()).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_register(&s(&env, "OctoCat"), &impostor, &fallbacks);

    assert!(
        result.is_err(),
        "impostor must not rebind without owner auth"
    );
    assert_eq!(owner_of(&env, &contract_id, "octocat"), Some(owner));
}

#[test]
fn test_reserved_username_case_variant_rejected() {
    let (env, _admin, contract_id) = setup();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::add_reserved(env.clone(), s(&env, "stellar")).unwrap();
    });

    let squatter = Address::generate(&env);
    env.mock_all_auths();
    let result = env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), s(&env, "Stellar"), squatter, Vec::new(&env))
    });
    assert_eq!(result, Err(ContractError::UsernameReserved));
    assert_eq!(owner_of(&env, &contract_id, "stellar"), None);
}

// ─── Malformed input / pause ─────────────────────────────────────────────────

#[test]
fn test_malformed_usernames_fail_with_invalid_username() {
    let (env, _admin, contract_id) = setup();
    let owner = Address::generate(&env);
    let too_long = "a".repeat(40);

    for bad in [
        "",
        "-octocat",
        "octocat-",
        "octo--cat",
        "octo cat",
        too_long.as_str(),
    ] {
        env.mock_all_auths();
        let result = env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), s(&env, bad), owner.clone(), Vec::new(&env))
        });
        assert_eq!(result, Err(ContractError::InvalidUsername), "input {bad:?}");
    }
}

#[test]
fn test_register_while_paused_leaves_no_binding() {
    let (env, _admin, contract_id) = setup();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
    });

    let owner = Address::generate(&env);
    env.mock_all_auths();
    let result = env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), s(&env, "octocat"), owner, Vec::new(&env))
    });
    assert_eq!(result, Err(ContractError::Paused));
    assert_eq!(owner_of(&env, &contract_id, "octocat"), None);
}
