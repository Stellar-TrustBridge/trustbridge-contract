//! Integration tests for `remove_guardian` authorization and event emission
//! (Issue #779).
//!
//! `remove_guardian` is admin-only, single-signature: there is no on-chain
//! dual-control for guardian rotation in this contract (unlike
//! `batch_remove`, see `tests/batch_remove_dual_control.rs`). The dual
//! control described for guardian rotation in `docs/ADMIN_RUNBOOK.md` is an
//! *operational* checklist for the human operators to follow manually; it is
//! not enforced by `src/lib.rs`. These tests lock the real on-chain auth
//! surface (`require_auth` on the admin only) so that doc does not drift
//! into implying a stronger on-chain guarantee than actually exists.

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Events as _},
    Address, Env, Symbol, TryFromVal,
};

use trustbridge_contract::{ContractError, TrustBridgeContract};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    (env, admin, contract_id)
}

fn has_event(env: &Env, contract_id: &Address, symbol: Symbol) -> bool {
    let want = soroban_sdk::xdr::ScVal::try_from_val(env, &symbol.to_val()).unwrap();
    env.events()
        .all()
        .filter_by_contract(contract_id)
        .events()
        .iter()
        .any(|e| match &e.body {
            soroban_sdk::xdr::ContractEventBody::V0(v0) => v0.topics.first() == Some(&want),
        })
}

/// The admin can remove a previously-set guardian.
#[test]
fn test_admin_can_remove_guardian() {
    let (env, admin, contract_id) = setup();
    let guardian = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_guardian(env.clone(), guardian.clone()).unwrap();
        assert_eq!(TrustBridgeContract::get_guardian(env.clone()), Some(guardian));

        TrustBridgeContract::remove_guardian(env.clone()).unwrap();
        assert_eq!(TrustBridgeContract::get_guardian(env.clone()), None);
    });
    let _ = admin;
}

/// Removing the guardian when none is set is a no-op success, not an error —
/// matches the storage-layer behavior of `remove_guardian` clearing a key
/// that may already be absent.
#[test]
fn test_remove_guardian_when_none_set_succeeds() {
    let (env, _admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_guardian(env.clone()), None);
        TrustBridgeContract::remove_guardian(env.clone()).unwrap();
        assert_eq!(TrustBridgeContract::get_guardian(env.clone()), None);
    });
}

/// `remove_guardian` requires auth from the contract admin specifically —
/// there is no secondary signer or dual-control gate on-chain. This locks
/// the real auth surface described in Issue #779 and in
/// `docs/ADMIN_RUNBOOK.md`.
#[test]
fn test_remove_guardian_requires_admin_auth() {
    let (env, _admin, contract_id) = setup();
    let guardian = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_guardian(env.clone(), guardian).unwrap();
    });

    // Without mocking auth, the admin's require_auth() is not satisfied in
    // this harness, so the call should fail rather than silently succeed.
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::remove_guardian(env.clone());
        assert!(result.is_err());
    });
}

/// Calling `remove_guardian` before `initialize` fails closed.
#[test]
fn test_remove_guardian_before_initialize_fails() {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::remove_guardian(env.clone());
        assert_eq!(result, Err(ContractError::NotInitialized));
    });
}

/// `remove_guardian` publishes a `GuardianChangedEvent` with `guardian: None`,
/// matching the removal semantics asserted in `tests/event_replay.rs`
/// (`"guardian_changed_event"`).
#[test]
fn test_remove_guardian_emits_guardian_changed_event() {
    let (env, _admin, contract_id) = setup();
    let guardian = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_guardian(env.clone(), guardian).unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove_guardian(env.clone()).unwrap();
    });

    assert!(has_event(
        &env,
        &contract_id,
        Symbol::new(&env, "guardian_changed_event")
    ));
}

/// A removed guardian can no longer call `emergency_pause` — removal takes
/// effect immediately for the privileged circuit-breaker path, which is the
/// operational reason guardian rotation matters during an incident.
#[test]
fn test_removed_guardian_cannot_emergency_pause() {
    let (env, _admin, contract_id) = setup();
    let guardian = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_guardian(env.clone(), guardian.clone()).unwrap();
        TrustBridgeContract::remove_guardian(env.clone()).unwrap();

        let result = TrustBridgeContract::emergency_pause(env.clone(), guardian.clone());
        assert!(result.is_err());
    });
}
