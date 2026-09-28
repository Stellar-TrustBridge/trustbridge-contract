//! Integration tests for the M-of-N multisig upgrade flow (Issue #301).
//!
//! Covers the full propose → approve → (delay + threshold) → execute lifecycle
//! plus the auth matrix (admin vs Upgrader vs random actor) for every entry
//! point exposed by `TrustBridgeContract`:
//!
//! - `set_upgrade_threshold` / `get_upgrade_threshold`
//! - `propose_multisig_upgrade`
//! - `approve_upgrade`
//! - `execute_upgrade`
//! - `cancel_upgrade_proposal`
//! - `get_upgrade_proposal`
//!
//! The operator-facing flow is documented in `docs/ADMIN_RUNBOOK.md`
//! ("Multisig upgrade flow (Issue #301)").

#![cfg(test)]

use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, BytesN, Env};

use trustbridge_contract::{ContractError, Role, TrustBridgeContract};

/// Test fixture: an initialized contract where `admin` is the contract admin,
/// `upgrader` holds `Role::Upgrader`, and `random` holds no role at all.
fn setup() -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let upgrader = Address::generate(&env);
    let random = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), upgrader.clone(), Role::Upgrader).unwrap();
    });

    (env, admin, upgrader, random, contract_id)
}

/// Helper: a 32-byte WASM hash filled with `byte` (a stand-in for a real hash).
fn hash(env: &Env, byte: u8) -> BytesN<32> {
    BytesN::from_array(env, &[byte; 32])
}

// ── Threshold configuration ───────────────────────────────────────────────────

#[test]
fn test_default_threshold_is_one() {
    let (env, _admin, _upgrader, _random, contract_id) = setup();

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_upgrade_threshold(env.clone()),
            1,
            "default threshold must retain single-admin behaviour"
        );
    });
}

#[test]
fn test_admin_can_set_upgrade_threshold() {
    let (env, admin, _upgrader, _random, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_upgrade_threshold(env.clone(), admin.clone(), 3).unwrap();
    });

    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_upgrade_threshold(env.clone()), 3);
    });
}

#[test]
fn test_upgrader_cannot_set_upgrade_threshold() {
    let (env, _admin, upgrader, _random, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::set_upgrade_threshold(env.clone(), upgrader.clone(), 2),
            Err(ContractError::NotAuthorized),
            "threshold configuration is admin-only"
        );
    });
}

#[test]
fn test_random_cannot_set_upgrade_threshold() {
    let (env, _admin, _upgrader, random, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::set_upgrade_threshold(env.clone(), random.clone(), 2),
            Err(ContractError::NotAuthorized)
        );
    });
}

// ── Proposing ─────────────────────────────────────────────────────────────────

#[test]
fn test_admin_and_upgrader_can_propose() {
    let (env, admin, upgrader, _random, contract_id) = setup();
    env.ledger().set_timestamp(1_000);
    let wasm = hash(&env, 0xAA);

    // Admin proposes.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            wasm.clone(),
            3_600,
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
        let proposal = TrustBridgeContract::get_upgrade_proposal(env.clone())
            .expect("proposal must exist after admin proposes");
        assert_eq!(proposal.id, 0);
        assert_eq!(proposal.wasm_hash, wasm);
        assert_eq!(proposal.proposed_by, admin);
        assert_eq!(proposal.proposed_at, 1_000);
        assert_eq!(proposal.executable_at, 4_600);
        assert_eq!(
            proposal.approvers.len(),
            1,
            "the proposer counts as the first approval"
        );
    });

    // Upgrader proposes after admin cancels.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::cancel_upgrade_proposal(env.clone(), admin.clone(), 0).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(env.clone(), upgrader.clone(), wasm, 0)
            .unwrap();
    });
    env.as_contract(&contract_id, || {
        let proposal = TrustBridgeContract::get_upgrade_proposal(env.clone()).unwrap();
        assert_eq!(proposal.id, 1, "proposal ids must be monotonic");
        assert_eq!(proposal.proposed_by, upgrader);
    });
}

#[test]
fn test_random_cannot_propose() {
    let (env, _admin, _upgrader, random, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::propose_multisig_upgrade(
                env.clone(),
                random.clone(),
                hash(&env, 0x01),
                0
            ),
            Err(ContractError::NotAuthorized)
        );
    });
}

#[test]
fn test_propose_fails_when_a_proposal_is_already_pending() {
    let (env, admin, upgrader, _random, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x01),
            0,
        )
        .unwrap();
        assert_eq!(
            TrustBridgeContract::propose_multisig_upgrade(
                env.clone(),
                upgrader.clone(),
                hash(&env, 0x02),
                0
            ),
            Err(ContractError::UpgradeProposalAlreadyPending),
            "only one proposal may be live at a time"
        );
    });
}

// ── Approving ─────────────────────────────────────────────────────────────────

#[test]
fn test_approvals_record_distinct_signers() {
    let (env, admin, upgrader, random, contract_id) = setup();
    env.ledger().set_timestamp(1_000);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x01),
            0,
        )
        .unwrap();
    });

    // Upgrader approves — the second distinct signer.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::approve_upgrade(env.clone(), upgrader.clone(), 0).unwrap();
    });
    env.as_contract(&contract_id, || {
        let proposal = TrustBridgeContract::get_upgrade_proposal(env.clone()).unwrap();
        assert_eq!(proposal.approvers.len(), 2);
        assert_eq!(proposal.approvers.get(0).unwrap(), admin);
        assert_eq!(proposal.approvers.get(1).unwrap(), upgrader);
    });

    // Random has no role: approval is rejected after auth.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::approve_upgrade(env.clone(), random.clone(), 0),
            Err(ContractError::NotAuthorized)
        );
    });

    // Duplicate approval by the proposer is rejected.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::approve_upgrade(env.clone(), admin.clone(), 0),
            Err(ContractError::UpgradeProposalAlreadyApproved)
        );
    });
}

#[test]
fn test_approve_upgrade_with_no_or_wrong_proposal_id_fails() {
    let (env, admin, upgrader, _random, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Nothing pending yet.
        assert_eq!(
            TrustBridgeContract::approve_upgrade(env.clone(), upgrader.clone(), 0),
            Err(ContractError::NoUpgradeProposalPending)
        );

        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x01),
            0,
        )
        .unwrap();
        // Wrong id for the live proposal.
        assert_eq!(
            TrustBridgeContract::approve_upgrade(env.clone(), upgrader.clone(), 99),
            Err(ContractError::NoUpgradeProposalPending)
        );
    });
}

// ── Executability: delay and threshold ────────────────────────────────────────

#[test]
fn test_execute_fails_before_delay_elapses() {
    let (env, admin, upgrader, _random, contract_id) = setup();
    env.ledger().set_timestamp(1_000);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x01),
            3_600,
        )
        .unwrap();

        // executable_at = 4_600; still inside the delay window.
        assert_eq!(
            TrustBridgeContract::execute_upgrade(env.clone(), upgrader.clone(), 0),
            Err(ContractError::UpgradeProposalDelayActive)
        );
    });
}

#[test]
fn test_execute_fails_with_insufficient_approvals() {
    let (env, admin, _upgrader, _random, contract_id) = setup();
    env.ledger().set_timestamp(1_000);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_upgrade_threshold(env.clone(), admin.clone(), 2).unwrap();
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x01),
            0,
        )
        .unwrap();
    });

    // Delay (0) has elapsed, but only the proposer has approved.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::execute_upgrade(env.clone(), admin.clone(), 0),
            Err(ContractError::UpgradeProposalInsufficientApprovals)
        );
    });
}

#[test]
fn test_execute_fails_with_no_live_proposal() {
    let (env, admin, upgrader, _random, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::execute_upgrade(env.clone(), upgrader.clone(), 0),
            Err(ContractError::NoUpgradeProposalPending)
        );

        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x01),
            0,
        )
        .unwrap();
        // Proposed but executed/cancelled under a different id than requested.
        assert_eq!(
            TrustBridgeContract::execute_upgrade(env.clone(), admin.clone(), 42),
            Err(ContractError::NoUpgradeProposalPending)
        );
    });
}

#[test]
fn test_random_cannot_execute_even_when_conditions_are_met() {
    let (env, admin, _upgrader, random, contract_id) = setup();
    env.ledger().set_timestamp(1_000);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x01),
            0,
        )
        .unwrap();
    });
    // Delay elapsed, threshold (default 1) met — but the caller has no role.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::execute_upgrade(env.clone(), random.clone(), 0),
            Err(ContractError::NotAuthorized)
        );
    });
}

// ── Successful execution (requires a pre-built WASM to actually swap) ─────────

#[test]
#[cfg(feature = "wasm-test")]
fn test_successful_execute_after_threshold_and_delay() {
    let (env, admin, upgrader, _random, contract_id) = setup();
    env.ledger().set_timestamp(1_000);

    let wasm_bytes = soroban_sdk::Bytes::from_slice(
        &env,
        include_bytes!("../target/wasm32v1-none/release/trustbridge_contract.wasm"),
    );
    let new_wasm_hash = env.deployer().upload_contract_wasm(wasm_bytes);

    // 2-of-2: admin proposes, upgrader approves, then any admin/Upgrader may
    // execute once the 3_600s delay has elapsed.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_upgrade_threshold(env.clone(), admin.clone(), 2).unwrap();
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            new_wasm_hash.clone(),
            3_600,
        )
        .unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::approve_upgrade(env.clone(), upgrader.clone(), 0).unwrap();
    });

    // Still inside the delay window after the second approval.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::execute_upgrade(env.clone(), upgrader.clone(), 0),
            Err(ContractError::UpgradeProposalDelayActive)
        );
    });

    // Advance past executable_at (1_000 + 3_600) and execute.
    env.ledger().set_timestamp(4_600);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::execute_upgrade(env.clone(), upgrader.clone(), 0).unwrap();
    });

    env.as_contract(&contract_id, || {
        // Proposal consumed.
        assert!(
            TrustBridgeContract::get_upgrade_proposal(env.clone()).is_none(),
            "executed proposal must be cleared"
        );
        // Provenance records the swap exactly like the single-admin path.
        let prov = TrustBridgeContract::get_provenance(env.clone())
            .expect("provenance must exist after multisig execute");
        assert_eq!(prov.wasm_hash, new_wasm_hash);
        assert_eq!(prov.upgraded_by, upgrader);
        assert_eq!(prov.upgraded_at, 4_600);
        assert!(!prov.attested, "multisig path records attested = false");
    });
}

// ── Cancellation ──────────────────────────────────────────────────────────────

#[test]
fn test_cancel_removes_proposal_and_allows_a_new_one() {
    let (env, admin, upgrader, _random, contract_id) = setup();
    env.ledger().set_timestamp(1_000);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x01),
            0,
        )
        .unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::cancel_upgrade_proposal(env.clone(), admin.clone(), 0).unwrap();
    });

    env.as_contract(&contract_id, || {
        assert!(
            TrustBridgeContract::get_upgrade_proposal(env.clone()).is_none(),
            "cancelled proposal must be cleared"
        );
    });

    // A new proposal can be created after cancellation.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            upgrader.clone(),
            hash(&env, 0x02),
            0,
        )
        .unwrap();
    });
}

#[test]
fn test_only_admin_can_cancel() {
    let (env, admin, upgrader, random, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x01),
            0,
        )
        .unwrap();
    });

    // Upgrader holds a privileged role but cancellation is admin-only.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::cancel_upgrade_proposal(env.clone(), upgrader.clone(), 0),
            Err(ContractError::NotAuthorized)
        );
    });

    // Random cannot cancel either.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::cancel_upgrade_proposal(env.clone(), random.clone(), 0),
            Err(ContractError::NotAuthorized)
        );
    });

    // The proposal survives the rejected attempts.
    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::get_upgrade_proposal(env.clone()).is_some());
    });
}

#[test]
fn test_cancel_with_no_or_wrong_proposal_id_fails() {
    let (env, admin, _upgrader, _random, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Nothing pending.
        assert_eq!(
            TrustBridgeContract::cancel_upgrade_proposal(env.clone(), admin.clone(), 0),
            Err(ContractError::NoUpgradeProposalPending)
        );

        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x01),
            0,
        )
        .unwrap();
        // Wrong id for the live proposal.
        assert_eq!(
            TrustBridgeContract::cancel_upgrade_proposal(env.clone(), admin.clone(), 99),
            Err(ContractError::NoUpgradeProposalPending)
        );
    });
}

// ── Not-initialized guard ─────────────────────────────────────────────────────

#[test]
fn test_multisig_functions_require_initialization() {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());
    let admin = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::set_upgrade_threshold(env.clone(), admin.clone(), 2),
            Err(ContractError::NotInitialized)
        );
        assert_eq!(
            TrustBridgeContract::propose_multisig_upgrade(
                env.clone(),
                admin.clone(),
                hash(&env, 0x01),
                0
            ),
            Err(ContractError::NotInitialized)
        );
        assert_eq!(
            TrustBridgeContract::approve_upgrade(env.clone(), admin.clone(), 0),
            Err(ContractError::NotInitialized)
        );
        assert_eq!(
            TrustBridgeContract::execute_upgrade(env.clone(), admin.clone(), 0),
            Err(ContractError::NotInitialized)
        );
        assert_eq!(
            TrustBridgeContract::cancel_upgrade_proposal(env.clone(), admin.clone(), 0),
            Err(ContractError::NotInitialized)
        );
    });
}
