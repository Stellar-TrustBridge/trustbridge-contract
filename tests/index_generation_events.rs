//! `IndexGenerationBumpedEvent` emission for dashboard consumers (Issue #688).
//!
//! Every index generation bump must publish an event so dashboards can
//! restart paginated syncs; rejected removals must not bump or emit.

#![cfg(test)]

use soroban_sdk::{
    testutils::Address as _, testutils::Events as _, Address, Env, String, Symbol, TryFromVal, Vec,
};
use trustbridge_contract::{ContractError, TrustBridgeContract};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    env.mock_all_auths();
    (env, admin, contract_id)
}

fn s(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

/// Number of `index_generation_bumped_event`s in the current event buffer.
fn bump_events(env: &Env, contract_id: &Address) -> usize {
    let symbol = Symbol::new(env, "index_generation_bumped_event");
    let want = soroban_sdk::xdr::ScVal::try_from_val(env, &symbol.to_val()).unwrap();
    env.events()
        .all()
        .filter_by_contract(contract_id)
        .events()
        .iter()
        .filter(|e| match &e.body {
            soroban_sdk::xdr::ContractEventBody::V0(v0) => v0.topics.first() == Some(&want),
        })
        .count()
}

#[test]
fn remove_bumps_generation_and_emits_event() {
    let (env, admin, contract_id) = setup();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "alice"),
            Address::generate(&env),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, "alice")).unwrap();

        assert_eq!(bump_events(&env, &contract_id), 1);
    });
}

#[test]
fn rejected_remove_does_not_bump_or_emit() {
    let (env, admin, contract_id) = setup();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, "ghost")),
            Err(ContractError::NotRegistered)
        );
        assert_eq!(bump_events(&env, &contract_id), 0);
    });
}
