//! Integration coverage for role expiry (Issues #221 / #428) and the role
//! grant timelock (Issue #220), driven through the public contract client.
//!
//! Run with `cargo test role`.

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, BytesN, Env, String, Vec,
};
use trustbridge_contract::{ContractError, Role, TrustBridgeContract, TrustBridgeContractClient};

const NOW: u64 = 1_000;
const DELAY: u64 = 3_600;

struct Fixture<'a> {
    env: Env,
    client: TrustBridgeContractClient<'a>,
    admin: Address,
    octocat: String,
}

fn setup<'a>() -> Fixture<'a> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(NOW);
    let client = TrustBridgeContractClient::new(&env, &env.register(TrustBridgeContract, ()));
    let admin = Address::generate(&env);
    client.initialize(&admin);
    let octocat = String::from_str(&env, "octocat");
    client.register(&octocat, &Address::generate(&env), &Vec::new(&env));
    Fixture {
        env,
        client,
        admin,
        octocat,
    }
}

fn advance_to(env: &Env, ts: u64) {
    env.ledger().set_timestamp(ts);
}

// ── Role expiry ─────────────────────────────────────────────────────────────

#[test]
fn test_role_expiry_verifier_works_until_expiry_then_role_expired() {
    let f = setup();
    let verifier = Address::generate(&f.env);
    let expires_at = NOW + 100;
    f.client
        .set_role_with_expiry(&verifier, &Role::Verifier, &Some(expires_at));

    assert_eq!(f.client.get_role(&verifier), Some(Role::Verifier));
    assert_eq!(f.client.get_role_expiry(&verifier), Some(expires_at));

    advance_to(&f.env, expires_at - 1);
    f.client.verify(&verifier, &f.octocat);
    f.client.revoke_verification(&f.admin, &f.octocat, &1);

    // Expiry is inclusive: at `expires_at` the grant has lapsed.
    advance_to(&f.env, expires_at);
    assert_eq!(f.client.get_role(&verifier), None);
    assert!(!f.client.has_role(&verifier, &Role::Verifier));
    // The raw timestamp stays readable for audits (lazy expiry).
    assert_eq!(f.client.get_role_expiry(&verifier), Some(expires_at));
    assert_eq!(
        f.client.try_verify(&verifier, &f.octocat),
        Err(Ok(ContractError::RoleExpired))
    );
    assert!(!f.client.get_address(&f.octocat).unwrap().verified);
}

#[test]
fn test_role_expiry_blocks_expired_upgrader_and_revoker() {
    let f = setup();
    let upgrader = Address::generate(&f.env);
    let revoker = Address::generate(&f.env);
    f.client
        .set_role_with_expiry(&upgrader, &Role::Upgrader, &Some(NOW + 10));
    f.client
        .set_role_with_expiry(&revoker, &Role::Revoker, &Some(NOW + 10));
    f.client.verify(&f.admin, &f.octocat);

    advance_to(&f.env, NOW + 10);
    let hash = BytesN::from_array(&f.env, &[7; 32]);
    assert_eq!(
        f.client.try_stage_wasm(&upgrader, &hash),
        Err(Ok(ContractError::RoleExpired))
    );
    assert_eq!(
        f.client.try_revoke_verification(&revoker, &f.octocat, &1),
        Err(Ok(ContractError::RoleExpired))
    );
}

#[test]
fn test_role_expiry_none_never_lapses() {
    let f = setup();
    let verifier = Address::generate(&f.env);
    f.client
        .set_role_with_expiry(&verifier, &Role::Verifier, &None);
    assert_eq!(f.client.get_role_expiry(&verifier), None);

    advance_to(&f.env, u64::MAX);
    assert_eq!(f.client.get_role(&verifier), Some(Role::Verifier));
    f.client.verify(&verifier, &f.octocat);
}

#[test]
fn test_role_expiry_renewed_by_set_role_and_cleared_by_remove_role() {
    let f = setup();
    let verifier = Address::generate(&f.env);
    f.client
        .set_role_with_expiry(&verifier, &Role::Verifier, &Some(NOW + 10));
    advance_to(&f.env, NOW + 20);
    assert_eq!(f.client.get_role(&verifier), None);

    // A plain set_role re-grants without an expiry.
    f.client.set_role(&verifier, &Role::Verifier);
    assert_eq!(f.client.get_role_expiry(&verifier), None);
    assert_eq!(f.client.get_role(&verifier), Some(Role::Verifier));

    f.client
        .set_role_with_expiry(&verifier, &Role::Verifier, &Some(NOW + 100));
    f.client.remove_role(&verifier);
    assert_eq!(f.client.get_role(&verifier), None);
    assert_eq!(f.client.get_role_expiry(&verifier), None);
    assert_eq!(
        f.client.try_verify(&verifier, &f.octocat),
        Err(Ok(ContractError::NotAuthorized))
    );
}

// ── Grant timelock ──────────────────────────────────────────────────────────

#[test]
fn test_role_timelock_grant_is_pending_until_activated() {
    let f = setup();
    let verifier = Address::generate(&f.env);
    f.client.set_role_delay(&DELAY);
    assert_eq!(f.client.get_role_delay(), DELAY);

    f.client.set_role(&verifier, &Role::Verifier);
    let pending = f.client.get_pending_role(&verifier).unwrap();
    assert_eq!(pending.role, Role::Verifier);
    assert_eq!(pending.activate_at, NOW + DELAY);

    // Queued, not held: the key cannot act yet.
    assert_eq!(f.client.get_role(&verifier), None);
    assert_eq!(
        f.client.try_verify(&verifier, &f.octocat),
        Err(Ok(ContractError::NotAuthorized))
    );

    advance_to(&f.env, NOW + DELAY - 1);
    assert_eq!(
        f.client.try_activate_role(&verifier),
        Err(Ok(ContractError::RoleGrantNotReady))
    );

    advance_to(&f.env, NOW + DELAY);
    f.client.activate_role(&verifier);
    assert_eq!(f.client.get_role(&verifier), Some(Role::Verifier));
    assert_eq!(f.client.get_pending_role(&verifier), None);
    f.client.verify(&verifier, &f.octocat);

    assert_eq!(
        f.client.try_activate_role(&verifier),
        Err(Ok(ContractError::NoPendingRoleGrant))
    );
}

#[test]
fn test_role_timelock_cancel_and_remove_role_drop_pending_grant() {
    let f = setup();
    let a = Address::generate(&f.env);
    let b = Address::generate(&f.env);
    f.client.set_role_delay(&DELAY);
    f.client.set_role(&a, &Role::Upgrader);
    f.client.set_role(&b, &Role::Verifier);

    f.client.cancel_role_grant(&a);
    assert_eq!(f.client.get_pending_role(&a), None);
    assert_eq!(
        f.client.try_cancel_role_grant(&a),
        Err(Ok(ContractError::NoPendingRoleGrant))
    );

    // A revoke must not let the queued grant resurrect the role later.
    f.client.remove_role(&b);
    assert_eq!(f.client.get_pending_role(&b), None);

    advance_to(&f.env, NOW + DELAY);
    for target in [&a, &b] {
        assert_eq!(
            f.client.try_activate_role(target),
            Err(Ok(ContractError::NoPendingRoleGrant))
        );
        assert_eq!(f.client.get_role(target), None);
    }
}

#[test]
fn test_role_timelock_zero_delay_restores_instant_grants() {
    let f = setup();
    let verifier = Address::generate(&f.env);
    f.client.set_role_delay(&DELAY);
    f.client.set_role_delay(&0);
    f.client.set_role(&verifier, &Role::Verifier);
    assert_eq!(f.client.get_pending_role(&verifier), None);
    assert_eq!(f.client.get_role(&verifier), Some(Role::Verifier));
}

/// `set_role_with_expiry` must not bypass an armed timelock to mint a new
/// grant, but may still shorten the expiry of a grant already held.
#[test]
fn test_role_timelock_cannot_be_bypassed_by_set_role_with_expiry() {
    let f = setup();
    let held = Address::generate(&f.env);
    let fresh = Address::generate(&f.env);
    f.client.set_role(&held, &Role::Verifier);
    f.client.set_role_delay(&DELAY);

    assert_eq!(
        f.client
            .try_set_role_with_expiry(&fresh, &Role::Verifier, &Some(NOW + 10_000)),
        Err(Ok(ContractError::RoleGrantNotReady))
    );
    assert_eq!(f.client.get_role(&fresh), None);

    // Escalating an existing holder to another role is also a new grant.
    assert_eq!(
        f.client
            .try_set_role_with_expiry(&held, &Role::Upgrader, &None),
        Err(Ok(ContractError::RoleGrantNotReady))
    );

    // Off-boarding an existing holder early still works under the timelock.
    f.client
        .set_role_with_expiry(&held, &Role::Verifier, &Some(NOW + 10));
    advance_to(&f.env, NOW + 10);
    assert_eq!(f.client.get_role(&held), None);
}

#[test]
fn test_role_timelock_admin_endpoints_require_admin_auth() {
    let f = setup();
    let target = Address::generate(&f.env);
    f.env.set_auths(&[]);
    assert!(f.client.try_set_role_delay(&DELAY).is_err());
    assert!(f.client.try_set_role(&target, &Role::Verifier).is_err());
    assert!(f
        .client
        .try_set_role_with_expiry(&target, &Role::Verifier, &None)
        .is_err());
    assert_eq!(f.client.get_role_delay(), 0);
    assert_eq!(f.client.get_role(&target), None);
}

// ── Expired role cleanup idempotency (Issue #752) ───────────────────────────
//
// Cleanup of expired grants is `remove_role` (role grants, any role) and
// `prune_expired_verifiers` (verifier allowlist). Both must be safe to run
// twice: no panic, no state change, and no duplicate events on the re-run.

use soroban_sdk::{testutils::Events as _, Symbol, TryFromVal};

/// `RoleRevokedEvent`s emitted by the contract during the last invocation.
fn role_revoked_events(f: &Fixture) -> usize {
    use soroban_sdk::xdr::{ContractEventBody, ScVal};
    let topic =
        ScVal::try_from_val(&f.env, &Symbol::new(&f.env, "role_revoked_event").to_val()).unwrap();
    f.env
        .events()
        .all()
        .filter_by_contract(&f.client.address)
        .events()
        .iter()
        .filter(|e| {
            let ContractEventBody::V0(v0) = &e.body;
            v0.topics.first() == Some(&topic)
        })
        .count()
}

#[test]
fn test_expired_role_cleanup_is_idempotent_for_admin_verifier_revoker() {
    let f = setup();
    let holders = [
        (Address::generate(&f.env), Role::Admin),
        (Address::generate(&f.env), Role::Verifier),
        (Address::generate(&f.env), Role::Revoker),
    ];
    for (who, role) in &holders {
        f.client.set_role_with_expiry(who, role, &Some(NOW + 10));
    }
    advance_to(&f.env, NOW + 10);

    for (who, role) in &holders {
        // Expired but still in storage (lazy expiry).
        assert!(!f.client.has_role(who, role));
        assert_eq!(f.client.get_role_expiry(who), Some(NOW + 10));

        // First cleanup deletes the grant and emits exactly one event.
        f.client.remove_role(who);
        assert_eq!(
            role_revoked_events(&f),
            1,
            "{role:?}: first cleanup emits one event"
        );
        assert_eq!(f.client.get_role_expiry(who), None);

        // Second cleanup: Ok, no event, no state change.
        f.client.remove_role(who);
        assert_eq!(
            role_revoked_events(&f),
            0,
            "{role:?}: re-run must not emit a duplicate event"
        );
        assert_eq!(f.client.get_role(who), None);
        assert_eq!(f.client.get_role_expiry(who), None);
    }
    // The real admin is unaffected by role-slot cleanup.
    f.client.verify(&f.admin, &f.octocat);
}

#[test]
fn test_remove_role_for_address_without_role_is_silent_no_op() {
    let f = setup();
    let stranger = Address::generate(&f.env);
    f.client.remove_role(&stranger);
    assert_eq!(role_revoked_events(&f), 0);
    assert_eq!(f.client.get_role(&stranger), None);
}

#[test]
fn test_remove_role_still_emits_for_pending_timelocked_grant_once() {
    let f = setup();
    let b = Address::generate(&f.env);
    f.client.set_role_delay(&DELAY);
    f.client.set_role(&b, &Role::Verifier); // queued, not yet active

    f.client.remove_role(&b);
    assert_eq!(role_revoked_events(&f), 1);
    f.client.remove_role(&b);
    assert_eq!(role_revoked_events(&f), 0);
}

#[test]
fn test_prune_expired_verifiers_is_idempotent() {
    let f = setup();
    let (a, b, keep) = (
        Address::generate(&f.env),
        Address::generate(&f.env),
        Address::generate(&f.env),
    );
    f.client.add_verifier(&a, &(NOW + 10));
    f.client.add_verifier(&b, &(NOW + 20));
    f.client.add_verifier(&keep, &0);
    advance_to(&f.env, NOW + 30);

    assert_eq!(f.client.prune_expired_verifiers(), 2);
    assert_eq!(
        f.client.prune_expired_verifiers(),
        0,
        "second prune removes nothing"
    );
    assert!(f.client.is_active_verifier(&keep));
    assert!(!f.client.is_active_verifier(&a));
}

#[test]
fn test_expired_role_cleanup_while_paused_fails_without_partial_state() {
    let f = setup();
    let verifier = Address::generate(&f.env);
    f.client
        .set_role_with_expiry(&verifier, &Role::Verifier, &Some(NOW + 10));
    advance_to(&f.env, NOW + 10);
    f.client.pause(&1);

    assert_eq!(
        f.client.try_remove_role(&verifier),
        Err(Ok(ContractError::Paused))
    );
    assert_eq!(
        f.client.get_role_expiry(&verifier),
        Some(NOW + 10),
        "grant must be untouched"
    );

    f.client.unpause(&1);
    f.client.remove_role(&verifier);
    assert_eq!(role_revoked_events(&f), 1);
}

#[test]
fn test_expired_role_cleanup_requires_admin_auth() {
    let f = setup();
    let verifier = Address::generate(&f.env);
    f.client
        .set_role_with_expiry(&verifier, &Role::Verifier, &Some(NOW + 10));
    advance_to(&f.env, NOW + 10);

    f.env.set_auths(&[]);
    assert!(f.client.try_remove_role(&verifier).is_err());
    assert_eq!(
        f.client.get_role_expiry(&verifier),
        Some(NOW + 10),
        "grant must be untouched"
    );
}

#[test]
fn test_remove_verifier_twice_fails_typed_without_second_event() {
    let f = setup();
    let v = Address::generate(&f.env);
    f.client.add_verifier(&v, &(NOW + 10));
    advance_to(&f.env, NOW + 10);

    f.client.remove_verifier(&v);
    assert_eq!(role_revoked_events(&f), 1);
    assert_eq!(
        f.client.try_remove_verifier(&v),
        Err(Ok(ContractError::VerifierNotAllowlisted))
    );
    assert_eq!(role_revoked_events(&f), 0);
}

#[test]
fn test_set_role_rejects_zero_address() {
    let f = setup();
    let zero = Address::from_str(
        &f.env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    );

    assert_eq!(
        f.client.try_set_role(&zero, &Role::Verifier),
        Err(Ok(ContractError::ZeroAddress))
    );
    assert_eq!(f.client.get_role(&zero), None);
    assert_eq!(f.client.get_pending_role(&zero), None);
}

#[test]
fn test_set_role_timelocked_rejects_zero_address() {
    let f = setup();
    f.client.set_role_delay(&60);
    let zero = Address::from_str(
        &f.env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    );

    assert_eq!(
        f.client.try_set_role(&zero, &Role::Admin),
        Err(Ok(ContractError::ZeroAddress))
    );
    assert_eq!(f.client.get_role(&zero), None);
    assert_eq!(f.client.get_pending_role(&zero), None);
}

#[test]
fn test_set_role_with_expiry_rejects_zero_address() {
    let f = setup();
    let zero = Address::from_str(
        &f.env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    );

    assert_eq!(
        f.client
            .try_set_role_with_expiry(&zero, &Role::Verifier, &Some(NOW + 100)),
        Err(Ok(ContractError::ZeroAddress))
    );
    assert_eq!(f.client.get_role(&zero), None);
    assert_eq!(f.client.get_role_expiry(&zero), None);
}

// ── Pending grant boundaries (Issue #855) ──

#[test]
fn test_pending_grant_activation_boundary() {
    let f = setup();
    let target = Address::generate(&f.env);
    f.client.set_role_delay(&DELAY);
    f.client.set_role(&target, &Role::Verifier);
    let pending = f.client.get_pending_role(&target).unwrap();
    assert_eq!(pending.activate_at, NOW + DELAY);

    advance_to(&f.env, pending.activate_at - 1);
    assert_eq!(
        f.client.try_activate_role(&target),
        Err(Ok(ContractError::RoleGrantNotReady))
    );
    assert_eq!(f.client.get_pending_role(&target), Some(pending.clone()));
    assert_eq!(f.client.get_role(&target), None);

    advance_to(&f.env, pending.activate_at);
    assert_eq!(f.client.get_pending_role(&target), Some(pending));
    assert_eq!(f.client.get_role(&target), None);
    f.client.activate_role(&target);
    assert_eq!(f.client.get_pending_role(&target), None);
    assert_eq!(f.client.get_role(&target), Some(Role::Verifier));
    assert_eq!(f.client.get_role_expiry(&target), None);
    f.client.verify(&target, &f.octocat);
    assert_eq!(
        f.client.try_activate_role(&target),
        Err(Ok(ContractError::NoPendingRoleGrant))
    );
}

#[test]
fn test_pending_grant_remains_queued_after_timelock_until_explicit_action() {
    let f = setup();
    let activate = Address::generate(&f.env);
    let cancel = Address::generate(&f.env);
    f.client.set_role_delay(&DELAY);
    for target in [&activate, &cancel] {
        f.client.set_role(target, &Role::Verifier);
    }

    advance_to(&f.env, NOW + DELAY + 1);
    for target in [&activate, &cancel] {
        assert_eq!(
            f.client.get_pending_role(target).unwrap().activate_at,
            NOW + DELAY
        );
        assert_eq!(f.client.get_role(target), None);
        assert_eq!(
            f.client.try_verify(target, &f.octocat),
            Err(Ok(ContractError::NotAuthorized))
        );
    }

    f.client.cancel_role_grant(&cancel);
    assert_eq!(f.client.get_pending_role(&cancel), None);
    assert_eq!(
        f.client.try_activate_role(&cancel),
        Err(Ok(ContractError::NoPendingRoleGrant))
    );
    assert_eq!(f.client.get_role(&cancel), None);

    f.client.activate_role(&activate);
    assert_eq!(f.client.get_pending_role(&activate), None);
    assert_eq!(f.client.get_role(&activate), Some(Role::Verifier));
}

#[test]
fn test_pending_grant_does_not_extend_expiring_active_role() {
    let f = setup();
    let target = Address::generate(&f.env);
    let expires_at = NOW + DELAY;
    f.client
        .set_role_with_expiry(&target, &Role::Verifier, &Some(expires_at));
    f.client.set_role_delay(&DELAY);
    f.client.set_role(&target, &Role::Verifier);

    advance_to(&f.env, expires_at - 1);
    assert_eq!(f.client.get_role(&target), Some(Role::Verifier));
    assert_eq!(f.client.get_role_expiry(&target), Some(expires_at));

    advance_to(&f.env, expires_at);
    assert_eq!(f.client.get_role(&target), None);
    assert!(!f.client.has_role(&target, &Role::Verifier));
    assert_eq!(
        f.client.try_verify(&target, &f.octocat),
        Err(Ok(ContractError::RoleExpired))
    );
    assert_eq!(
        f.client.get_pending_role(&target).unwrap().activate_at,
        expires_at
    );

    f.client.cancel_role_grant(&target);
    assert_eq!(f.client.get_pending_role(&target), None);
    assert_eq!(f.client.get_role_expiry(&target), Some(expires_at));
    advance_to(&f.env, expires_at + 1);
    assert_eq!(
        f.client.try_activate_role(&target),
        Err(Ok(ContractError::NoPendingRoleGrant))
    );
    assert_eq!(f.client.get_role(&target), None);
    assert!(!f.client.get_address(&f.octocat).unwrap().verified);
}

#[test]
fn test_pending_grant_boundary_preserves_pause_and_admin_auth() {
    let f = setup();
    let target = Address::generate(&f.env);
    f.client.set_role_delay(&DELAY);
    f.client.set_role(&target, &Role::Verifier);
    let pending = f.client.get_pending_role(&target).unwrap();

    advance_to(&f.env, pending.activate_at);
    f.client.pause(&1);
    assert_eq!(
        f.client.try_activate_role(&target),
        Err(Ok(ContractError::Paused))
    );
    assert_eq!(f.client.get_pending_role(&target), Some(pending.clone()));
    assert_eq!(f.client.get_role(&target), None);
    f.client.unpause(&1);

    f.env.set_auths(&[]);
    assert!(f.client.try_activate_role(&target).is_err());
    assert!(f.client.try_cancel_role_grant(&target).is_err());
    assert_eq!(f.client.get_pending_role(&target), Some(pending));
    assert_eq!(f.client.get_role(&target), None);

    f.env.mock_all_auths();
    f.client.activate_role(&target);
    assert_eq!(f.client.get_pending_role(&target), None);
    assert_eq!(f.client.get_role(&target), Some(Role::Verifier));
}
