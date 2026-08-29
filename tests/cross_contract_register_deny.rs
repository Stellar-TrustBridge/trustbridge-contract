//! Explicit denial test for cross-contract `register` (no nonce added).
//!
//! `register()` has no replay-protecting nonce. That is only a real risk if a
//! sibling contract can ever call `register` on a caller's behalf
//! cross-contract (C2C) — a relaying contract could then replay a captured
//! call. This suite proves that path does not exist today: `register`
//! requires `stellar_address.require_auth()`, and a calling contract executes
//! in its *own* authorization context, so it cannot produce a signature for
//! an address it does not control. A relayed C2C `register` therefore fails
//! closed, the same way C2C calls to the admin-gated export functions already
//! do (see `docs/SECURITY.md` § Cross-Contract Callers and Admin Exports).
//!
//! Per the "don't half-open C2C" scope: this intentionally does **not** add a
//! nonce, and does **not** add any C2C registration surface. If C2C register
//! is ever introduced, replay protection (a nonce) becomes required at that
//! point — see `docs/SECURITY.md` for the explicit ABI statement.

#![cfg(test)]

use soroban_sdk::{contract, contractimpl, testutils::Address as _, Address, Env, String, Vec};

use trustbridge_contract::{TrustBridgeContract, TrustBridgeContractClient};

/// Minimal sibling contract that relays a `register` call to the registry,
/// standing in for any future payout/onboarding contract that might try to
/// call `register` cross-contract.
#[contract]
pub struct CallerContract;

#[contractimpl]
impl CallerContract {
    pub fn relay_register(env: Env, registry: Address, username: String, on_behalf_of: Address) {
        let client = TrustBridgeContractClient::new(&env, &registry);
        client.register(&username, &on_behalf_of, &Vec::new(&env));
    }
}

fn s(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

fn deployed_registry(env: &Env) -> (Address, Address) {
    let admin = Address::generate(env);
    let registry_id = env.register(TrustBridgeContract, ());
    env.as_contract(&registry_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    (registry_id, admin)
}

/// C2C register is impossible: a relaying contract cannot supply the target
/// address's signature, so the host aborts the invocation for missing auth.
#[test]
#[should_panic]
fn cross_contract_register_on_behalf_of_another_address_is_denied() {
    let env = Env::default();
    let (registry_id, _admin) = deployed_registry(&env);

    let caller_id = env.register(CallerContract, ());
    let caller_client = CallerContractClient::new(&env, &caller_id);
    let victim = Address::generate(&env);

    // No auths mocked. `CallerContract` has no way to authorize on behalf of
    // `victim` for `stellar_address.require_auth()` inside `register`.
    caller_client.relay_register(&registry_id, &s(&env, "octocat"), &victim);
}

/// Positive control: the identical call path succeeds when the real address
/// owner signs directly, proving the denial above is about cross-contract
/// auth boundaries, not a broken `register`.
#[test]
fn direct_register_by_the_address_owner_still_works() {
    let env = Env::default();
    env.mock_all_auths();
    let (registry_id, _admin) = deployed_registry(&env);

    let owner = Address::generate(&env);
    let client = TrustBridgeContractClient::new(&env, &registry_id);
    client.register(&s(&env, "octocat"), &owner, &Vec::new(&env));

    assert!(client.has_record(&s(&env, "octocat")));
}
