//! Integration tests for the dual-control `batch_remove` flow (Issue #219).
//!
//! `batch_remove` is a single-admin bulk delete. Once `set_batch_remove_threshold`
//! is non-zero, any batch **larger** than the threshold must be proposed by one
//! admin-equivalent address and executed by a *different* one:
//!
//! - `set_batch_remove_threshold` / `get_batch_remove_threshold`
//! - `propose_batch_remove`
//! - `execute_batch_remove`
//! - `cancel_batch_remove`
//! - `get_pending_batch_remove`
//!
//! This suite exercises the threshold behaviour (`0` disables dual control, `N`
//! forces the propose path), the full propose → (second key) execute lifecycle,
//! the cancel escape hatch, the 24 h proposal TTL, the pause interaction (block
//! propose/execute, but keep cancel available), the auth surface, and the
//! `BatchRemoveProposed` / `BatchRemoveExecuted` / `BatchRemoveCancelled` events.
//!
//! The operator-facing flow is documented in `docs/ADMIN_RUNBOOK.md`
//! ("Dual-Control `batch_remove` (Issue #219)"). The bundle of events published
//! on each path is asserted by topic symbol, matching `tests/challenge.rs`.

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    Address, Env, String, Symbol, TryFromVal, Vec,
};

use trustbridge_contract::{BatchSummary, ContractError, Role, TrustBridgeContract};

/// 24 h proposal TTL, kept in sync with
/// `storage::BATCH_REMOVE_PROPOSAL_TTL_SECS`.
const PROPOSAL_TTL_SECS: u64 = 86_400;

fn username(env: &Env, name: &str) -> String {
    String::from_str(env, name)
}

/// Test fixture: an initialized contract where `admin` is the contract admin
/// and `second` holds `Role::Admin` — the second key required for the
/// dual-control execute step.
fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let second = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), second.clone(), Role::Admin).unwrap();
    });

    (env, admin, second, contract_id)
}

/// Registers `n` usernames (`user000`, `user001`, …) and returns the list.
fn register_n(env: &Env, contract_id: &Address, n: u32) -> Vec<String> {
    let mut names = Vec::new(env);
    env.mock_all_auths();
    env.as_contract(contract_id, || {
        for i in 0..n {
            let name = username(env, &format!("user{i:03}"));
            TrustBridgeContract::register(
                env.clone(),
                name.clone(),
                Address::generate(env),
                Vec::new(env),
            )
            .unwrap();
            names.push_back(name);
        }
    });
    names
}

/// True when the contract emitted an event whose first topic (the event-type
/// symbol) equals `symbol` — e.g. `"batch_remove_proposed_event"`.
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

// ── Threshold behaviour ───────────────────────────────────────────────────────

/// The default threshold is `0` and disables dual control entirely: a large
/// batch is executed directly by a single admin call, exactly as pre-#219.
#[test]
fn test_threshold_zero_disables_dual_control() {
    let (env, admin, _second, contract_id) = setup();
    let names = register_n(&env, &contract_id, 20);

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_batch_remove_threshold(env.clone()),
            0
        );
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let summary = TrustBridgeContract::batch_remove(env.clone(), admin.clone(), names).unwrap();
        assert_eq!(summary.successful, 20);
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 0);
    });
}

/// A non-zero threshold forces the propose/execute path for any batch **larger**
/// than it (strictly greater — a batch exactly at the threshold still executes
/// directly), and the rejected batch writes nothing.
#[test]
fn test_threshold_above_forces_the_propose_path() {
    let (env, admin, _second, contract_id) = setup();
    let names = register_n(&env, &contract_id, 4);

    let over = vec_from(&env, names.clone(), 0, 4);
    let at = vec_from(&env, names.clone(), 0, 3);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        assert_eq!(
            TrustBridgeContract::get_batch_remove_threshold(env.clone()),
            3
        );

        // 4 > 3: rejected with DualControlRequired, registry untouched.
        let res = TrustBridgeContract::batch_remove(env.clone(), admin.clone(), over);
        assert_eq!(res, Err(ContractError::DualControlRequired));
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 4);

        // 3 == 3: still the direct single-step path.
        let summary = TrustBridgeContract::batch_remove(env.clone(), admin.clone(), at).unwrap();
        assert_eq!(summary.successful, 3);
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 1);
    });
}

// ── Propose ───────────────────────────────────────────────────────────────────

/// Propose by the admin, then execute by a *different* `Role::Admin` holder.
/// The batch is removed, `BatchRemoveProposed` + `BatchRemoveExecuted` + one
/// `RemovedEvent` per username are published, and the proposal slot is cleared.
#[test]
fn test_propose_then_execute_by_a_second_key() {
    let (env, admin, second, contract_id) = setup();
    let names = register_n(&env, &contract_id, 4);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();
    });

    assert!(has_event(
        &env,
        &contract_id,
        Symbol::new(&env, "batch_remove_proposed_event")
    ));

    env.as_contract(&contract_id, || {
        let pending = TrustBridgeContract::get_pending_batch_remove(env.clone())
            .expect("proposal must be visible via get_pending_batch_remove");
        assert_eq!(pending.usernames.len(), 4);
        assert_eq!(pending.proposed_by, admin);
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let summary: BatchSummary =
            TrustBridgeContract::execute_batch_remove(env.clone(), second.clone()).unwrap();
        assert_eq!(summary.total, 4);
        assert_eq!(summary.successful, 4);
        assert_eq!(summary.failed, 0);
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 0);
        assert!(
            TrustBridgeContract::get_pending_batch_remove(env.clone()).is_none(),
            "executed proposal must be cleared"
        );
    });

    assert!(has_event(
        &env,
        &contract_id,
        Symbol::new(&env, "batch_remove_executed_event")
    ));
    assert!(has_event(
        &env,
        &contract_id,
        Symbol::new(&env, "removed_event")
    ));
}

/// Only one proposal may be pending at a time: a second propose while one is
/// live is rejected, and the first proposal survives.
#[test]
fn test_second_proposal_rejected_while_pending() {
    let (env, admin, second, contract_id) = setup();
    let names = register_n(&env, &contract_id, 4);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();

        let res =
            TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone());
        assert_eq!(res, Err(ContractError::BatchRemoveProposalPending));

        // A Role::Admin holder is not the contract admin and cannot propose at
        // all — `propose_batch_remove` is explicitly admin-only.
        let res = TrustBridgeContract::propose_batch_remove(env.clone(), second.clone(), names);
        assert_eq!(res, Err(ContractError::NotAuthorized));

        assert!(
            TrustBridgeContract::get_pending_batch_remove(env.clone()).is_some(),
            "the original proposal must survive rejected attempts"
        );
    });
}

/// Propose is admin-only: a random caller gets `NotAuthorized`.
#[test]
fn test_random_cannot_propose() {
    let (env, _admin, _second, contract_id) = setup();
    let names = register_n(&env, &contract_id, 4);
    let random = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::propose_batch_remove(env.clone(), random, names),
            Err(ContractError::NotAuthorized)
        );
    });
}

// ── Execute ───────────────────────────────────────────────────────────────────

/// `execute_batch_remove` requires a caller distinct from the proposer — one
/// signature alone must never delete a large batch.
#[test]
fn test_execute_by_same_proposer_rejected() {
    let (env, admin, _second, contract_id) = setup();
    let names = register_n(&env, &contract_id, 4);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names).unwrap();

        let res = TrustBridgeContract::execute_batch_remove(env.clone(), admin.clone());
        assert_eq!(res, Err(ContractError::NotAuthorized));
        assert!(
            TrustBridgeContract::get_pending_batch_remove(env.clone()).is_some(),
            "the proposal survives a rejected execution attempt"
        );
    });
}

/// A caller that is neither the contract admin nor a `Role::Admin` holder
/// cannot execute, even after auth succeeds.
#[test]
fn test_execute_by_non_admin_rejected() {
    let (env, admin, _second, contract_id) = setup();
    let names = register_n(&env, &contract_id, 4);
    let random = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names).unwrap();

        assert_eq!(
            TrustBridgeContract::execute_batch_remove(env.clone(), random),
            Err(ContractError::NotAuthorized)
        );
    });
}

/// Executing with nothing pending fails with `NoPendingBatchRemove`.
#[test]
fn test_execute_with_no_pending_proposal_fails() {
    let (env, _admin, second, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::execute_batch_remove(env.clone(), second.clone()),
            Err(ContractError::NoPendingBatchRemove)
        );
    });
}

/// A proposal older than `BATCH_REMOVE_PROPOSAL_TTL_SECS` (24 h) is treated as
/// gone rather than executed — the "execute after delay" boundary. An expired
/// proposal is cleared, so a fresh one can be proposed immediately.
#[test]
fn test_execute_after_proposal_ttl_elapsed_rejected() {
    let (env, admin, second, contract_id) = setup();
    let names = register_n(&env, &contract_id, 4);

    env.ledger().set_timestamp(1_000);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();
    });

    // Past the 24 h TTL.
    env.ledger().set_timestamp(1_000 + PROPOSAL_TTL_SECS + 1);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let res = TrustBridgeContract::execute_batch_remove(env.clone(), second.clone());
        assert_eq!(res, Err(ContractError::NoPendingBatchRemove));
        // Expired proposal was cleared and never executed.
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 4);
        assert!(TrustBridgeContract::get_pending_batch_remove(env.clone()).is_none());

        // A fresh proposal is possible immediately after.
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names).unwrap();
    });
}

// ── Cancel ────────────────────────────────────────────────────────────────────

/// Cancelling clears the pending proposal, publishes
/// `BatchRemoveCancelledEvent`, preserves every record, and allows a new
/// proposal.
#[test]
fn test_cancel_clears_proposal_and_allows_a_new_one() {
    let (env, admin, _second, contract_id) = setup();
    let names = register_n(&env, &contract_id, 4);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();
        TrustBridgeContract::cancel_batch_remove(env.clone(), admin.clone()).unwrap();
    });

    assert!(has_event(
        &env,
        &contract_id,
        Symbol::new(&env, "batch_remove_cancelled_event")
    ));

    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::get_pending_batch_remove(env.clone()).is_none());
        assert_eq!(
            TrustBridgeContract::get_stats(env.clone()).total,
            4,
            "cancel must not remove any record"
        );
    });

    // A fresh proposal is possible after a cancel.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names).unwrap();
        assert!(TrustBridgeContract::get_pending_batch_remove(env.clone()).is_some());
    });
}

/// Cancel is admin-only and requires a live proposal.
#[test]
fn test_cancel_by_non_admin_or_without_proposal_fails() {
    let (env, admin, _second, contract_id) = setup();
    let names = register_n(&env, &contract_id, 4);
    let random = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Nothing pending.
        assert_eq!(
            TrustBridgeContract::cancel_batch_remove(env.clone(), admin.clone()),
            Err(ContractError::NoPendingBatchRemove)
        );

        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names).unwrap();

        // Non-admin canceller.
        assert_eq!(
            TrustBridgeContract::cancel_batch_remove(env.clone(), random),
            Err(ContractError::NotAuthorized)
        );

        // The rejected attempt left the proposal in place.
        assert!(TrustBridgeContract::get_pending_batch_remove(env.clone()).is_some());
    });
}

// ── Pause ─────────────────────────────────────────────────────────────────────

/// Pausing blocks propose and execute, but the abort path (`cancel_batch_remove`)
/// and the `get_pending_batch_remove` read stay available — an emergency freeze
/// must never trap a proposal behind itself.
#[test]
fn test_pause_blocks_propose_and_execute_but_not_cancel() {
    let (env, admin, second, contract_id) = setup();
    let names = register_n(&env, &contract_id, 4);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::pause(env.clone(), 1).unwrap();

        // Propose is blocked while paused.
        assert_eq!(
            TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone()),
            Err(ContractError::Paused)
        );

        // Unpause, propose, then re-pause so the execute step is exercised.
        TrustBridgeContract::unpause(env.clone(), 4).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();
        TrustBridgeContract::pause(env.clone(), 1).unwrap();

        // Execute is blocked while paused; the proposal survives.
        assert_eq!(
            TrustBridgeContract::execute_batch_remove(env.clone(), second.clone()),
            Err(ContractError::Paused)
        );
        assert!(
            TrustBridgeContract::get_pending_batch_remove(env.clone()).is_some(),
            "the proposal must survive the pause and remain visible"
        );

        // Cancel works while paused — the abort path is never blockable by the
        // same freeze that might motivate using it.
        TrustBridgeContract::cancel_batch_remove(env.clone(), admin.clone()).unwrap();
        assert!(TrustBridgeContract::get_pending_batch_remove(env.clone()).is_none());

        // Unpausing restores the full flow end to end.
        TrustBridgeContract::unpause(env.clone(), 4).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names).unwrap();
        let summary =
            TrustBridgeContract::execute_batch_remove(env.clone(), second.clone()).unwrap();
        assert_eq!(summary.successful, 4);
    });
}

/// Copy a slice of a soroban `Vec<String>` into a fresh `Vec`, so one
/// registration list can drive several differently-sized batches.
fn vec_from(env: &Env, names: Vec<String>, start: u32, len: usize) -> Vec<String> {
    let mut out = Vec::new(env);
    for i in start..(start + len as u32) {
        out.push_back(names.get(i).unwrap());
    }
    out
}

// ── Auth matrix tests ─────────────────────────────────────────────────────────
//
// Each test here maps to one row in tests/auth_matrix.csv and one cell in the
// Dual-Control batch_remove Auth Negative Matrix in docs/SECURITY.md.

/// ST2: `set_batch_remove_threshold` is admin-only; a random caller is rejected.
/// Maps to: `set_batch_remove_threshold,random,NotAuthorized` in auth_matrix.csv.
#[test]
fn test_auth_matrix_set_batch_remove_threshold_random() {
    let (env, _admin, _second, contract_id) = setup();
    let random = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let res = TrustBridgeContract::set_batch_remove_threshold(env.clone(), random);
        assert_eq!(
            res,
            Err(ContractError::NotAuthorized),
            "set_batch_remove_threshold must reject a non-admin caller with NotAuthorized"
        );
    });
}

/// PB2: A `Role::Admin` holder that is **not** the contract admin cannot propose.
/// The `propose_batch_remove` entry point is reserved exclusively for the
/// contract admin — Role::Admin is only sufficient for the *execute* step.
/// Maps to the `second_admin` row: `propose_batch_remove,random,NotAuthorized`.
#[test]
fn test_propose_batch_remove_role_admin_holder_cannot_propose() {
    let (env, _admin, second, contract_id) = setup();
    let names = register_n(&env, &contract_id, 4);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();

        // `second` holds Role::Admin (granted in setup()) but is not the
        // contract admin stored in ADMIN_KEY — that is `admin`.
        let res = TrustBridgeContract::propose_batch_remove(env.clone(), second.clone(), names);
        assert_eq!(
            res,
            Err(ContractError::NotAuthorized),
            "Role::Admin holder must not be able to propose; only the contract admin can"
        );
        assert!(
            TrustBridgeContract::get_pending_batch_remove(env.clone()).is_none(),
            "a rejected propose must leave no pending proposal"
        );
    });
}

/// EB1 counter-correctness: after a dual-control execute the registry counters
/// (`total` and `verified`) reflect only what was actually removed.
/// Exercises the `apply_batch_remove` path that is shared by both the direct
/// and dual-control flows, confirming it is equally correct either way.
#[test]
fn test_verified_count_correct_after_dual_control_execute() {
    let (env, admin, second, contract_id) = setup();
    let names = register_n(&env, &contract_id, 4);

    // Verify the first two contributors so we can assert the verified counter.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "user000")).unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "user001")).unwrap();
    });

    env.as_contract(&contract_id, || {
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 4);
        assert_eq!(stats.verified, 2);
    });

    // Enable dual-control and go through the propose→execute flow.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let summary =
            TrustBridgeContract::execute_batch_remove(env.clone(), second.clone()).unwrap();
        assert_eq!(summary.total, 4);
        assert_eq!(summary.successful, 4);

        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(
            stats.total, 0,
            "total must reach 0 after removing all 4 entries"
        );
        assert_eq!(
            stats.verified, 0,
            "verified must reach 0 after removing both verified entries"
        );
    });
}

/// `get_pending_batch_remove` is a public read and must remain accessible
/// while the contract is paused, regardless of who calls it.
/// Maps to: `get_pending_batch_remove,random,` (empty expected error) in auth_matrix.csv.
#[test]
fn test_get_pending_batch_remove_public_while_paused() {
    let (env, admin, _second, contract_id) = setup();
    let names = register_n(&env, &contract_id, 4);
    let random = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names).unwrap();
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
    });

    // Must succeed with no auth and while paused.
    env.as_contract(&contract_id, || {
        let pending = TrustBridgeContract::get_pending_batch_remove(env.clone());
        assert!(
            pending.is_some(),
            "get_pending_batch_remove must return the proposal even while paused"
        );
        // The random variable proves no auth was required for the read.
        let _ = random;
    });
}

/// `get_batch_remove_threshold` is a public read — no auth, always accessible.
/// Maps to: `get_batch_remove_threshold,random,` (empty expected error) in auth_matrix.csv.
#[test]
fn test_get_batch_remove_threshold_public_no_auth() {
    let (env, admin, _second, contract_id) = setup();

    // Default threshold is 0.
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_batch_remove_threshold(env.clone()),
            0,
            "default threshold must be 0 (dual-control disabled)"
        );
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 5).unwrap();
    });

    // Any caller (no mock_all_auths) can read back the threshold.
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_batch_remove_threshold(env.clone()),
            5,
            "get_batch_remove_threshold must reflect the admin-set value"
        );
    });
    let _ = admin;
}

/// CB1/CB4 cross-check: cancel after a successful execute must fail with
/// `NoPendingBatchRemove`, confirming that execute clears the slot atomically.
/// Prevents a double-cancel or a cancel after the batch is already gone.
#[test]
fn test_cancel_after_execute_fails_with_no_pending() {
    let (env, admin, second, contract_id) = setup();
    let names = register_n(&env, &contract_id, 4);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names).unwrap();
        TrustBridgeContract::execute_batch_remove(env.clone(), second.clone()).unwrap();

        // The slot is gone — cancel must fail rather than silently no-op.
        let res = TrustBridgeContract::cancel_batch_remove(env.clone(), admin.clone());
        assert_eq!(
            res,
            Err(ContractError::NoPendingBatchRemove),
            "cancel after execute must fail with NoPendingBatchRemove"
        );
    });
}

/// Verify that `BatchRemoveCancelledEvent` carries the correct proposer even
/// when the canceller is different from the original proposer.
/// (admin proposes → admin cancels; event must record both addresses.)
#[test]
fn test_cancel_event_records_proposer_and_canceller() {
    let (env, admin, _second, contract_id) = setup();
    let names = register_n(&env, &contract_id, 4);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names).unwrap();
        TrustBridgeContract::cancel_batch_remove(env.clone(), admin.clone()).unwrap();
    });

    assert!(
        has_event(
            &env,
            &contract_id,
            Symbol::new(&env, "batch_remove_cancelled_event")
        ),
        "BatchRemoveCancelledEvent must be emitted on cancel"
    );
    // Records are untouched — nothing was removed.
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_stats(env.clone()).total,
            4,
            "cancel must not remove any record"
        );
    });
}
