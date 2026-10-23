//! Integration tests for the timelocked admin transfer flow (Issue #195).
//!
//! Covers the full propose → (delay) → execute lifecycle plus the abort path,
//! the overwrite semantics, the zero-address guard, the pause interplay and
//! the admin-identity rotation, using `env.ledger().set_timestamp` to move
//! the timelock forward without sleeping:
//!
//! - `propose_admin_transfer` — admin-only, fixed identity, overwrites
//! - `cancel_admin_transfer` — admin-only, no-op when nothing pending,
//!   available while paused
//! - `execute_admin_transfer` — only the proposed successor, only after the
//!   delay, only while unpaused
//! - `get_admin_transfer` — read-only, works while paused
//!
//! The operator-facing flow is documented in `docs/ADMIN_RUNBOOK.md`
//! ("Admin transfer flow (Issue #195)"); the security rationale in
//! `docs/SECURITY.md` ("Two-step admin rotation (Issue #195)").
//!
//! Structure note: each auth-bearing call runs in its own `as_contract`
//! block. Recording auth (what `mock_all_auths` enables) allows at most one
//! `require_auth` per address per frame, so bundling two `admin.require_auth`
//! calls into one block trips the host's "frame is already authorized" guard.

#![cfg(test)]

use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal, Symbol, TryFromVal, Val};
use trustbridge_contract::{ContractError, Role, TrustBridgeContract};

/// The zero/burn account strkey — `is_zero_address` must reject it.
const ZERO_ADDRESS_STRKEY: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";

const T0: u64 = 1_000;
const DELAY: u64 = 60;

/// Test fixture: an initialized contract where `admin` is the contract admin
/// (ADMIN_KEY and `Role::Admin`) and `successor` / `random` hold no role.
/// The ledger timestamp starts at [`T0`].
fn setup() -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    env.ledger().set_timestamp(T0);
    let admin = Address::generate(&env);
    let successor = Address::generate(&env);
    let random = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    env.mock_all_auths();

    (env, admin, successor, random, contract_id)
}

/// Invokes `func` with only `signer` authorized for that exact invocation and
/// returns `true` when the host auth layer rejected the call (i.e. `signer`
/// does not satisfy the entrypoint's `require_auth`).
///
/// Runs *outside* any `as_contract` block — `try_invoke_contract` creates its
/// own invocation frame.
fn host_rejects(
    env: &Env,
    contract_id: &Address,
    signer: &Address,
    func: &str,
    args: soroban_sdk::Vec<Val>,
) -> bool {
    let invoke = MockAuthInvoke {
        contract: contract_id,
        fn_name: func,
        args: args.clone(),
        sub_invokes: &[],
    };
    env.mock_auths(&[MockAuth {
        address: signer,
        invoke: &invoke,
    }]);
    let res =
        env.try_invoke_contract::<Val, ContractError>(contract_id, &Symbol::new(env, func), args);
    let rejected = matches!(res, Err(Err(_)));
    env.mock_all_auths();
    rejected
}

// ── Propose ──────────────────────────────────────────────────────────────────

#[test]
fn test_propose_stores_proposal_and_getter_round_trips() {
    let (env, admin, successor, _random, contract_id) = setup();

    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_admin_transfer(env.clone()), None);
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_admin_transfer(env.clone(), successor.clone(), DELAY).unwrap();
    });
    env.as_contract(&contract_id, || {
        let proposal = TrustBridgeContract::get_admin_transfer(env.clone())
            .expect("pending proposal must be readable");
        assert_eq!(proposal.new_admin, successor);
        assert_eq!(proposal.proposed_by, admin);
        assert_eq!(proposal.proposed_at, T0);
        assert_eq!(proposal.executable_at, T0 + DELAY);
    });
}

#[test]
fn test_propose_emits_admin_transfer_proposed_event() {
    let (env, _admin, successor, _random, contract_id) = setup();

    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_admin_transfer(env.clone(), successor.clone(), DELAY).unwrap();
    });

    let topic = Symbol::new(&env, "admin_transfer_proposed_event");
    let target = soroban_sdk::xdr::ScVal::try_from_val(&env, &topic.to_val()).unwrap();
    let found = env
        .events()
        .all()
        .filter_by_contract(&contract_id)
        .events()
        .iter()
        .any(|event| match &event.body {
            soroban_sdk::xdr::ContractEventBody::V0(v0) => v0.topics.first() == Some(&target),
        });
    assert!(found, "AdminTransferProposedEvent must be published");
}

#[test]
fn test_non_admin_cannot_propose() {
    let (env, _admin, successor, random, contract_id) = setup();

    // Only `random` is authorized for this invocation; the contract demands
    // the stored admin, so the host auth layer must reject it.
    let args = soroban_sdk::vec![&env, successor.into_val(&env), DELAY.into_val(&env)];
    assert!(
        host_rejects(&env, &contract_id, &random, "propose_admin_transfer", args),
        "a non-admin signer must be rejected by the host auth layer"
    );
    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_admin_transfer(env.clone()), None);
    });
}

#[test]
fn test_propose_rejects_zero_address() {
    let (env, _admin, _successor, _random, contract_id) = setup();

    env.as_contract(&contract_id, || {
        let zero = Address::from_string(&soroban_sdk::String::from_str(&env, ZERO_ADDRESS_STRKEY));
        assert_eq!(
            TrustBridgeContract::propose_admin_transfer(env.clone(), zero, DELAY),
            Err(ContractError::ZeroAddress)
        );
    });
    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_admin_transfer(env.clone()), None);
    });
}

#[test]
fn test_propose_overwrites_pending_proposal() {
    let (env, _admin, successor, random, contract_id) = setup();

    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_admin_transfer(env.clone(), successor.clone(), DELAY).unwrap();
    });
    // The admin corrects the target and shortens the delay during the
    // window — documented overwrite semantics.
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_admin_transfer(env.clone(), random.clone(), 0).unwrap();
    });
    env.as_contract(&contract_id, || {
        let proposal = TrustBridgeContract::get_admin_transfer(env.clone()).unwrap();
        assert_eq!(proposal.new_admin, random, "second proposal wins");
        assert_eq!(proposal.executable_at, T0, "delay is recomputed from now");
    });
}

// ── Execute ──────────────────────────────────────────────────────────────────

#[test]
fn test_execute_before_delay_fails() {
    let (env, _admin, successor, _random, contract_id) = setup();

    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_admin_transfer(env.clone(), successor.clone(), DELAY).unwrap();
    });

    // One second short of the timelock.
    env.ledger().set_timestamp(T0 + DELAY - 1);
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::execute_admin_transfer(env.clone(), successor.clone()),
            Err(ContractError::AdminTransferDelayActive)
        );
    });

    // Exactly at executable_at the transfer goes through.
    env.ledger().set_timestamp(T0 + DELAY);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::execute_admin_transfer(env.clone(), successor.clone()).unwrap();
    });
}

#[test]
fn test_execute_with_no_pending_proposal_fails() {
    let (env, _admin, successor, _random, contract_id) = setup();

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::execute_admin_transfer(env.clone(), successor.clone()),
            Err(ContractError::NoPendingAdminTransfer)
        );
    });
}

#[test]
fn test_execute_rejects_outgoing_admin_and_strangers() {
    let (env, admin, successor, random, contract_id) = setup();

    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_admin_transfer(env.clone(), successor.clone(), DELAY).unwrap();
    });
    env.ledger().set_timestamp(T0 + DELAY);

    // The outgoing admin gets the actionable state, not a generic denial.
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::execute_admin_transfer(env.clone(), admin),
            Err(ContractError::AdminTransferPending)
        );
    });
    // Anyone else is simply not authorized.
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::execute_admin_transfer(env.clone(), random),
            Err(ContractError::NotAuthorized)
        );
    });
    // The proposal survives both failed attempts.
    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::get_admin_transfer(env.clone()).is_some());
    });
}

#[test]
fn test_execute_rotates_admin_identity_and_roles() {
    let (env, admin, successor, _random, contract_id) = setup();

    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::has_role(
            env.clone(),
            admin.clone(),
            Role::Admin
        ));
        assert!(!TrustBridgeContract::has_role(
            env.clone(),
            successor.clone(),
            Role::Admin
        ));
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_admin_transfer(env.clone(), successor.clone(), DELAY).unwrap();
    });
    env.ledger().set_timestamp(T0 + DELAY);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::execute_admin_transfer(env.clone(), successor.clone()).unwrap();
    });

    // RBAC bookkeeping follows the key: no window with two live admins.
    env.as_contract(&contract_id, || {
        assert!(
            !TrustBridgeContract::has_role(env.clone(), admin.clone(), Role::Admin),
            "old admin must lose Role::Admin"
        );
        assert!(
            TrustBridgeContract::has_role(env.clone(), successor.clone(), Role::Admin),
            "new admin must receive Role::Admin"
        );
    });

    // The stored admin identity (ADMIN_KEY) rotated with it: only the
    // successor can now satisfy propose's fixed-identity require_auth.
    let propose_args = || {
        soroban_sdk::vec![
            &env,
            Address::generate(&env).into_val(&env),
            DELAY.into_val(&env)
        ]
    };
    assert!(
        host_rejects(
            &env,
            &contract_id,
            &admin,
            "propose_admin_transfer",
            propose_args()
        ),
        "the outgoing admin must no longer authenticate as admin"
    );
    assert!(
        !host_rejects(
            &env,
            &contract_id,
            &successor,
            "propose_admin_transfer",
            propose_args()
        ),
        "the successor must now authenticate as admin"
    );
}

// ── Cancel ───────────────────────────────────────────────────────────────────

#[test]
fn test_cancel_releases_the_proposal() {
    let (env, _admin, successor, _random, contract_id) = setup();

    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_admin_transfer(env.clone(), successor.clone(), DELAY).unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::cancel_admin_transfer(env.clone()).unwrap();
    });
    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_admin_transfer(env.clone()), None);
    });
    env.ledger().set_timestamp(T0 + DELAY);
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::execute_admin_transfer(env.clone(), successor.clone()),
            Err(ContractError::NoPendingAdminTransfer)
        );
    });
}

#[test]
fn test_cancel_without_pending_proposal_is_a_noop() {
    let (env, _admin, _successor, _random, contract_id) = setup();

    env.as_contract(&contract_id, || {
        TrustBridgeContract::cancel_admin_transfer(env.clone()).unwrap();
    });
    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_admin_transfer(env.clone()), None);
    });
}

#[test]
fn test_non_admin_cannot_cancel() {
    let (env, _admin, successor, random, contract_id) = setup();

    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_admin_transfer(env.clone(), successor.clone(), DELAY).unwrap();
    });
    assert!(
        host_rejects(
            &env,
            &contract_id,
            &random,
            "cancel_admin_transfer",
            soroban_sdk::vec![&env]
        ),
        "a non-admin signer must be rejected by the host auth layer"
    );
    env.as_contract(&contract_id, || {
        assert!(
            TrustBridgeContract::get_admin_transfer(env.clone()).is_some(),
            "a failed cancel must leave the proposal pending"
        );
    });
}

// ── Pause interplay ──────────────────────────────────────────────────────────

#[test]
fn test_pause_blocks_propose_and_execute_but_not_cancel() {
    let (env, _admin, successor, _random, contract_id) = setup();

    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_admin_transfer(env.clone(), successor.clone(), 0).unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
    });

    // A pause is an emergency brake on the forward path...
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::propose_admin_transfer(env.clone(), successor.clone(), 0),
            Err(ContractError::Paused)
        );
    });
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::execute_admin_transfer(env.clone(), successor.clone()),
            Err(ContractError::Paused)
        );
    });

    // ...but the abort path stays open, and monitoring keeps working:
    // the getter is read-only and unaffected by the pause.
    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::get_admin_transfer(env.clone()).is_some());
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::cancel_admin_transfer(env.clone()).unwrap();
    });
    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_admin_transfer(env.clone()), None);
    });
}

#[test]
fn test_pause_keeps_the_delay_timer_running() {
    let (env, _admin, successor, _random, contract_id) = setup();

    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_admin_transfer(env.clone(), successor.clone(), DELAY).unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
    });

    // The proposal storage is untouched by the pause; once unpaused the
    // original executable_at still governs.
    env.as_contract(&contract_id, || {
        let proposal = TrustBridgeContract::get_admin_transfer(env.clone()).unwrap();
        assert_eq!(proposal.executable_at, T0 + DELAY);
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::unpause(env.clone(), 1).unwrap();
    });
    env.ledger().set_timestamp(T0 + DELAY);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::execute_admin_transfer(env.clone(), successor.clone()).unwrap();
    });
}
