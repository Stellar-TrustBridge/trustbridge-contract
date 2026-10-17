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
