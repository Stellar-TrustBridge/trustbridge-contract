//! End-to-end tests for the squatter-challenge lifecycle (Issue #214).
//!
//! `start_challenge` / `cancel_challenge` / `complete_challenge` /
//! `get_challenge` shipped with events but no dedicated test file. This suite
//! exercises the full host-level lifecycle, the admin-only auth surface, every
//! error named in the ABI (`ChallengeAlreadyActive`, `ChallengeNotResolvable`,
//! `NoChallengeActive`, `NotRegistered`, `Paused`), and the registry count
//! invariants the challenge flow must preserve (see
//! `docs/REGISTRY_INVARIANTS.md` -> *Challenge-period operations*).

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    Address, Env, String, Symbol, TryFromVal, Vec,
};

use trustbridge_contract::{ChallengeRecord, ContractError, TrustBridgeContract};

/// 48h challenge delay, kept in sync with `storage::DEFAULT_CHALLENGE_DELAY_SECS`.
const DEFAULT_CHALLENGE_DELAY_SECS: u64 = 172_800;

fn username(env: &Env, name: &str) -> String {
    String::from_str(env, name)
}

fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });

    (env, admin, user, contract_id)
}

fn register(env: &Env, contract_id: &Address, user: &Address, name: &str) {
    env.mock_all_auths();
    env.as_contract(contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(env, name),
            user.clone(),
            Vec::new(env),
        )
        .unwrap();
    });
}

fn start_challenge(env: &Env, contract_id: &Address, admin: &Address, name: &str) {
    env.mock_all_auths();
    env.as_contract(contract_id, || {
        TrustBridgeContract::start_challenge(env.clone(), admin.clone(), username(env, name))
            .unwrap();
    });
}

/// True when the contract emitted an event whose first topic (the event-type
/// symbol) equals `symbol` — e.g. `"challenge_started_event"`.
fn has_event(env: &Env, contract_id: &Address, symbol: Symbol) -> bool {
    for (source, topics, _data) in env.events().all() {
        if source == *contract_id {
            if let Some(topic) = topics.get(0) {
                if Symbol::try_from_val(env, &topic).unwrap() == symbol {
                    return true;
                }
            }
        }
    }
    false
}

// ── Happy path ────────────────────────────────────────────────────────────────

/// register → start_challenge → register/rename blocked → complete after the
/// delay → record gone, challenge cleared, counters decremented.
#[test]
fn test_challenge_happy_path_lifecycle() {
    let (env, admin, user, contract_id) = setup();
    register(&env, &contract_id, &user, "octocat");

    // No challenge before `start_challenge`.
    env.as_contract(&contract_id, || {
        assert!(
            TrustBridgeContract::get_challenge(env.clone(), username(&env, "octocat")).is_none()
        );
    });

    start_challenge(&env, &contract_id, &admin, "octocat");

    let started = env.ledger().timestamp();
    env.as_contract(&contract_id, || {
        let challenge: ChallengeRecord =
            TrustBridgeContract::get_challenge(env.clone(), username(&env, "octocat"))
                .expect("challenge must be visible via get_challenge");
        assert_eq!(challenge.challenged_by, admin);
        assert_eq!(challenge.started_at, started);
        assert_eq!(
            challenge.resolve_after,
            started.saturating_add(DEFAULT_CHALLENGE_DELAY_SECS)
        );
    });

    // Re-registration is blocked while the challenge is active.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            ),
            Err(ContractError::ChallengeActive)
        );
    });

    // Rename is blocked while a challenge is open on either name.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::rename(
                env.clone(),
                user.clone(),
                username(&env, "octocat"),
                username(&env, "octocat2"),
            ),
            Err(ContractError::ChallengeActive)
        );
    });

    // complete_challenge before the resolve window elapses is not resolvable.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::complete_challenge(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
            ),
            Err(ContractError::ChallengeNotResolvable)
        );
    });

    // The rejected completion moved nothing.
    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).is_some());
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 1);
    });

    // Advance the ledger past `resolve_after` and complete.
    env.ledger()
        .set_timestamp(started.saturating_add(DEFAULT_CHALLENGE_DELAY_SECS + 1));

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::complete_challenge(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
        )
        .unwrap();
    });

    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).is_none());
        assert!(
            TrustBridgeContract::get_challenge(env.clone(), username(&env, "octocat")).is_none()
        );
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 0);
        assert_eq!(stats.verified, 0);
    });

    // Both the removal and the challenge-completion events were published.
    assert!(has_event(
        &env,
        &contract_id,
        Symbol::new(&env, "challenge_started_event")
    ));
    assert!(has_event(
        &env,
        &contract_id,
        Symbol::new(&env, "removed_event")
    ));
    assert!(has_event(
        &env,
        &contract_id,
        Symbol::new(&env, "challenge_completed_event")
    ));
}

/// A completed challenge decrements `total` and `verified` exactly like a
/// `remove` of the same record (Issue #214 count invariants).
#[test]
fn test_complete_challenge_decrements_total_and_verified() {
    let (env, admin, user, contract_id) = setup();
    register(&env, &contract_id, &user, "octocat");

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });

    env.as_contract(&contract_id, || {
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 1);
        assert_eq!(stats.verified, 1);
    });

    let started = env.ledger().timestamp();
    start_challenge(&env, &contract_id, &admin, "octocat");
    env.ledger()
        .set_timestamp(started.saturating_add(DEFAULT_CHALLENGE_DELAY_SECS + 1));

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::complete_challenge(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
        )
        .unwrap();
    });

    env.as_contract(&contract_id, || {
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 0, "total must decrement on completion");
        assert_eq!(
            stats.verified, 0,
            "verified count must decrement when completing a challenge on a verified record"
        );
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 0);
    });
}

// ── Cancel path ───────────────────────────────────────────────────────────────

/// Admin cancels a challenge; the record survives and the username is free for
/// re-registration again.
#[test]
fn test_cancel_challenge_frees_username() {
    let (env, admin, user, contract_id) = setup();
    register(&env, &contract_id, &user, "octocat");
    start_challenge(&env, &contract_id, &admin, "octocat");

    assert!(has_event(
        &env,
        &contract_id,
        Symbol::new(&env, "challenge_started_event")
    ));

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::cancel_challenge(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
        )
        .unwrap();
    });

    env.as_contract(&contract_id, || {
        assert!(
            TrustBridgeContract::get_challenge(env.clone(), username(&env, "octocat")).is_none()
        );
        // The record itself is preserved by a cancel.
        assert!(TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).is_some());
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 1);
    });

    assert!(has_event(
        &env,
        &contract_id,
        Symbol::new(&env, "challenge_cancelled_event")
    ));

    // The name is registrable again after the cancel. A *different* address
    // taking the name over would normally need the old holder's signature
    // (`stellar_address.require_auth` on the existing record's address) — the
    // point here is simply that the challenge lock itself is gone.
    let newcomer = Address::generate(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            newcomer.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 1);
    });
}

/// The registrant can self-remove during the window; that clears the challenge
/// atomically, so a later complete_challenge finds nothing to complete.
#[test]
fn test_registrant_self_remove_clears_challenge() {
    let (env, admin, user, contract_id) = setup();
    register(&env, &contract_id, &user, "octocat");
    start_challenge(&env, &contract_id, &admin, "octocat");

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat")).unwrap();
    });

    env.as_contract(&contract_id, || {
        assert!(
            TrustBridgeContract::get_challenge(env.clone(), username(&env, "octocat")).is_none()
        );
        assert!(TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).is_none());
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 0);
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::complete_challenge(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
            ),
            Err(ContractError::NoChallengeActive)
        );
    });
}

// ── Failure surface ───────────────────────────────────────────────────────────

#[test]
fn test_start_challenge_twice_returns_challenge_already_active() {
    let (env, admin, user, contract_id) = setup();
    register(&env, &contract_id, &user, "octocat");
    start_challenge(&env, &contract_id, &admin, "octocat");

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::start_challenge(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
            ),
            Err(ContractError::ChallengeAlreadyActive)
        );
    });
}

#[test]
fn test_cancel_challenge_without_active_challenge_returns_no_challenge_active() {
    let (env, admin, user, contract_id) = setup();
    register(&env, &contract_id, &user, "octocat");

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::cancel_challenge(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
            ),
            Err(ContractError::NoChallengeActive)
        );
    });
}

#[test]
fn test_complete_challenge_without_active_challenge_returns_no_challenge_active() {
    let (env, admin, user, contract_id) = setup();
    register(&env, &contract_id, &user, "octocat");

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::complete_challenge(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
            ),
            Err(ContractError::NoChallengeActive)
        );
    });
}

#[test]
fn test_start_challenge_on_unregistered_username_returns_not_registered() {
    let (env, admin, _user, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::start_challenge(
                env.clone(),
                admin.clone(),
                username(&env, "ghost")
            ),
            Err(ContractError::NotRegistered)
        );
    });
}

// ── Auth ──────────────────────────────────────────────────────────────────────

/// All three mutating challenge entry points are admin-only; a non-admin gets
/// `NotAuthorized` regardless of the underlying challenge state.
#[test]
fn test_challenge_ops_are_admin_only() {
    let (env, admin, user, contract_id) = setup();
    register(&env, &contract_id, &user, "octocat");

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::start_challenge(
                env.clone(),
                user.clone(),
                username(&env, "octocat"),
            ),
            Err(ContractError::NotAuthorized)
        );
        assert_eq!(
            TrustBridgeContract::cancel_challenge(
                env.clone(),
                user.clone(),
                username(&env, "octocat"),
            ),
            Err(ContractError::NotAuthorized)
        );
        assert_eq!(
            TrustBridgeContract::complete_challenge(
                env.clone(),
                user.clone(),
                username(&env, "octocat"),
            ),
            Err(ContractError::NotAuthorized)
        );
    });

    // The rejected calls moved nothing.
    env.as_contract(&contract_id, || {
        assert!(
            TrustBridgeContract::get_challenge(env.clone(), username(&env, "octocat")).is_none()
        );
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 1);
    });

    // `get_challenge` is a public read — a non-admin can see the challenge.
    start_challenge(&env, &contract_id, &admin, "octocat");
    env.as_contract(&contract_id, || {
        assert!(
            TrustBridgeContract::get_challenge(env.clone(), username(&env, "octocat")).is_some()
        );
    });
}

// ── Pause ─────────────────────────────────────────────────────────────────────

/// All three challenge mutators are blocked while the contract is paused.
#[test]
fn test_challenge_ops_blocked_while_paused() {
    let (env, admin, user, contract_id) = setup();
    register(&env, &contract_id, &user, "octocat");

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::start_challenge(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
            ),
            Err(ContractError::Paused)
        );
        assert_eq!(
            TrustBridgeContract::cancel_challenge(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
            ),
            Err(ContractError::Paused)
        );
        assert_eq!(
            TrustBridgeContract::complete_challenge(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
            ),
            Err(ContractError::Paused)
        );
    });
}
