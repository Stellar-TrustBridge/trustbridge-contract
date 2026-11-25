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

use trustbridge_contract::{
    ContractError, Role, TrustBridgeContract, LAST_UPG_KEY, MAX_UPGRADE_SIGNERS,
};

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

#[test]
fn test_proposal_error_paths_after_cancellation() {
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

        // Admin cancels proposal 0.
        TrustBridgeContract::cancel_upgrade_proposal(env.clone(), admin.clone(), 0).unwrap();

        // Approval on cancelled proposal fails with NoUpgradeProposalPending.
        assert_eq!(
            TrustBridgeContract::approve_upgrade(env.clone(), upgrader.clone(), 0),
            Err(ContractError::NoUpgradeProposalPending)
        );

        // Execution on cancelled proposal fails with NoUpgradeProposalPending.
        assert_eq!(
            TrustBridgeContract::execute_upgrade(env.clone(), admin.clone(), 0),
            Err(ContractError::NoUpgradeProposalPending)
        );

        // Repeat cancel on already-cancelled proposal fails with NoUpgradeProposalPending.
        assert_eq!(
            TrustBridgeContract::cancel_upgrade_proposal(env.clone(), admin.clone(), 0),
            Err(ContractError::NoUpgradeProposalPending)
        );
    });
}

#[test]
fn test_propose_preserves_live_proposal_state_on_already_pending_error() {
    let (env, admin, upgrader, _random, contract_id) = setup();
    env.ledger().set_timestamp(1_000);
    let original_hash = hash(&env, 0xAA);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            original_hash.clone(),
            1_800,
        )
        .unwrap();

        // Upgrader tries to propose while one is pending -> UpgradeProposalAlreadyPending.
        let err = TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            upgrader.clone(),
            hash(&env, 0xBB),
            0,
        );
        assert_eq!(err, Err(ContractError::UpgradeProposalAlreadyPending));

        // Verify the original proposal's state is preserved intact.
        let live = TrustBridgeContract::get_upgrade_proposal(env.clone()).unwrap();
        assert_eq!(live.id, 0);
        assert_eq!(live.wasm_hash, original_hash);
        assert_eq!(live.proposed_by, admin);
        assert_eq!(live.proposed_at, 1_000);
        assert_eq!(live.executable_at, 2_800);
        assert_eq!(live.approvers.len(), 1);
    });
}

// ── Attestation-required flag (Issue #198 / #406) ────────────────────────────

#[test]
fn test_attestation_required_defaults_to_false() {
    let (env, _admin, _upgrader, _random, contract_id) = setup();

    env.as_contract(&contract_id, || {
        assert!(
            !TrustBridgeContract::get_attestation_required(env.clone()),
            "attestation_required must default to false"
        );
    });
}

#[test]
fn test_admin_can_set_attestation_required() {
    let (env, admin, _upgrader, _random, contract_id) = setup();

    // Enable: each require_auth needs a fresh mock_all_auths frame.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_attestation_required(env.clone(), admin.clone(), true).unwrap();
        assert!(TrustBridgeContract::get_attestation_required(env.clone()));
    });

    // Toggle back off.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_attestation_required(env.clone(), admin.clone(), false).unwrap();
        assert!(!TrustBridgeContract::get_attestation_required(env.clone()));
    });
}

#[test]
fn test_non_admin_cannot_set_attestation_required() {
    let (env, _admin, upgrader, random, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::set_attestation_required(env.clone(), upgrader.clone(), true),
            Err(ContractError::NotAuthorized),
            "upgrader must not set attestation_required"
        );
        assert_eq!(
            TrustBridgeContract::set_attestation_required(env.clone(), random.clone(), true),
            Err(ContractError::NotAuthorized),
            "random actor must not set attestation_required"
        );
    });
}

#[test]
fn test_clear_attestation_does_not_disable_required_flag() {
    let (env, admin, _upgrader, _random, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Turn on the gate then clear any pending attestation.
        TrustBridgeContract::set_attestation_required(env.clone(), admin.clone(), true).unwrap();
        TrustBridgeContract::clear_attestation(env.clone()).unwrap();

        // The required flag must still be on — clearing the attestation record
        // is a separate operation from disabling the gate.
        assert!(
            TrustBridgeContract::get_attestation_required(env.clone()),
            "clear_attestation must not disable the attestation_required flag"
        );
    });
}

#[test]
fn test_required_attestation_blocks_multisig_execute_without_attestation() {
    let (env, admin, _upgrader, _random, contract_id) = setup();
    let wasm_hash = hash(&env, 0xA8);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_attestation_required(env.clone(), admin.clone(), true).unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(env.clone(), admin.clone(), wasm_hash, 0)
            .unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::execute_upgrade(env.clone(), admin.clone(), 0),
            Err(ContractError::AttestationRequired),
            "the multisig path must honor the required-attestation gate"
        );
        assert!(
            TrustBridgeContract::get_upgrade_proposal(env.clone()).is_some(),
            "a rejected execution must leave its proposal intact"
        );
    });
}

// ── Threshold above signer set must fail closed (Issue #795) ─────────────────

/// Setting a threshold above `MAX_UPGRADE_SIGNERS` would make the threshold
/// permanently unreachable — no proposal could ever collect enough distinct
/// approvers. The contract must fail closed and reject the call rather than
/// accept a value that freezes all future upgrades.
#[test]
fn test_admin_cannot_set_threshold_above_signer_cap() {
    let (env, admin, _upgrader, _random, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::set_upgrade_threshold(
                env.clone(),
                admin.clone(),
                MAX_UPGRADE_SIGNERS + 1,
            ),
            Err(ContractError::ThresholdExceedsSignerCap),
            "threshold above MAX_UPGRADE_SIGNERS must be rejected"
        );
        // Previous (default) threshold is unchanged after the rejected call.
        assert_eq!(TrustBridgeContract::get_upgrade_threshold(env.clone()), 1);
    });
}

/// A threshold exactly at the hard cap is still reachable and must be
/// accepted — only values strictly above the cap are rejected.
#[test]
fn test_admin_can_set_threshold_exactly_at_signer_cap() {
    let (env, admin, _upgrader, _random, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_upgrade_threshold(env.clone(), admin.clone(), MAX_UPGRADE_SIGNERS)
            .unwrap();
        assert_eq!(
            TrustBridgeContract::get_upgrade_threshold(env.clone()),
            MAX_UPGRADE_SIGNERS
        );
    });
}

/// Recovery: after an over-cap threshold is rejected, the admin can set a
/// reachable threshold (lower it) and resume normal multisig governance —
/// the documented recovery path in `docs/ADMIN_RUNBOOK.md`.
#[test]
fn test_recovery_set_reachable_threshold_after_rejected_attempt() {
    let (env, admin, _upgrader, _random, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::set_upgrade_threshold(
            env.clone(),
            admin.clone(),
            MAX_UPGRADE_SIGNERS + 10,
        )
        .is_err());

        // Admin lowers the threshold to a value within the signer cap.
        TrustBridgeContract::set_upgrade_threshold(env.clone(), admin.clone(), 2).unwrap();
        assert_eq!(TrustBridgeContract::get_upgrade_threshold(env.clone()), 2);
    });
}

// ── Upgrade cooldown and monotonic remaining time assertions (Issue #758) ─────

/// When upgrade cooldown is active, `execute_upgrade` must fail with
/// `ContractError::CooldownActive`, and health snapshot `cooldown_remaining_secs`
/// must decrease monotonically across advancing ledger timestamps until reaching zero.
#[test]
fn test_multisig_upgrade_cooldown_active_and_monotonic_decrease() {
    let (env, admin, upgrader, _random, contract_id) = setup();
    env.ledger().set_timestamp(1_000);

    // Configure 3,600s cooldown and simulate a previous upgrade at timestamp 1,000.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_cooldown(env.clone(), 3_600).unwrap();
        env.storage().instance().set(&LAST_UPG_KEY, &1_000u64);
    });

    // Propose an upgrade with delay 0; threshold default is 1 (so proposer satisfies it).
    // An upgrader also approves to cover multi-party approval semantics.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0xAA),
            0,
        )
        .unwrap();
        TrustBridgeContract::approve_upgrade(env.clone(), upgrader.clone(), 0).unwrap();
    });

    // Check monotonic decrease across timestamps:
    // [timestamp, expected_remaining_secs]
    let timeline: [(u64, u64); 6] = [
        (1_000, 3_600),
        (1_600, 3_000),
        (2_200, 2_400),
        (3_500, 1_100),
        (4_599, 1),
        (4_600, 0),
    ];

    let mut prev_remaining = u64::MAX;

    for (ts, expected_remaining) in timeline {
        env.ledger().set_timestamp(ts);

        env.as_contract(&contract_id, || {
            let health = TrustBridgeContract::get_health(env.clone()).unwrap();
            assert_eq!(
                health.cooldown_remaining_secs, expected_remaining,
                "cooldown_remaining_secs mismatch at timestamp {}",
                ts
            );

            // Assert monotonic decrease: strictly smaller as time advances
            assert!(
                health.cooldown_remaining_secs < prev_remaining,
                "cooldown_remaining_secs must decrease monotonically (was {}, now {})",
                prev_remaining,
                health.cooldown_remaining_secs
            );
            prev_remaining = health.cooldown_remaining_secs;

            // While cooldown is active (> 0), execute_upgrade must fail with CooldownActive
            if expected_remaining > 0 {
                let err = TrustBridgeContract::execute_upgrade(env.clone(), upgrader.clone(), 0);
                assert_eq!(
                    err,
                    Err(ContractError::CooldownActive),
                    "execute_upgrade must fail with CooldownActive while cooldown is running at ts {}",
                    ts
                );

                // Proposal must remain pending and intact
                assert!(
                    TrustBridgeContract::get_upgrade_proposal(env.clone()).is_some(),
                    "proposal must remain live despite failed execute attempt"
                );
            }
        });
    }

    // After cooldown has expired, remaining time stays zero across future timestamps.
    env.ledger().set_timestamp(5_000);
    env.as_contract(&contract_id, || {
        let health = TrustBridgeContract::get_health(env.clone()).unwrap();
        assert_eq!(health.cooldown_remaining_secs, 0);
    });
}

/// When both proposal delay and upgrade cooldown are active:
/// 1. Before proposal delay elapses, `execute_upgrade` fails with `UpgradeProposalDelayActive`.
/// 2. Once proposal delay elapses but cooldown has not, `execute_upgrade` fails with `CooldownActive`.
/// 3. Monotonic decrease of `cooldown_remaining_secs` is maintained across all phases.
#[test]
fn test_multisig_upgrade_delay_precedes_cooldown_error() {
    let (env, admin, _upgrader, _random, contract_id) = setup();
    env.ledger().set_timestamp(1_000);

    // Cooldown = 3,000s (expires at 4,000); proposal delay = 1,000s (executable at 2,000).
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_cooldown(env.clone(), 3_000).unwrap();
        env.storage().instance().set(&LAST_UPG_KEY, &1_000u64);
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0xBB),
            1_000,
        )
        .unwrap();
    });

    // Phase 1: Timestamp 1,500 (delay active: 1,500 < 2,000; cooldown active: 1,500 < 4,000)
    env.ledger().set_timestamp(1_500);
    env.as_contract(&contract_id, || {
        let health = TrustBridgeContract::get_health(env.clone()).unwrap();
        assert_eq!(health.cooldown_remaining_secs, 2_500);

        let err = TrustBridgeContract::execute_upgrade(env.clone(), admin.clone(), 0);
        assert_eq!(
            err,
            Err(ContractError::UpgradeProposalDelayActive),
            "delay check must precede cooldown check"
        );
    });

    // Phase 2: Timestamp 2_500 (delay elapsed: 2_500 >= 2,000; cooldown still active: 2_500 < 4,000)
    env.ledger().set_timestamp(2_500);
    env.as_contract(&contract_id, || {
        let health = TrustBridgeContract::get_health(env.clone()).unwrap();
        assert_eq!(health.cooldown_remaining_secs, 1_500);
        assert!(1_500 < 2_500, "cooldown_remaining_secs decreased monotonically");

        let err = TrustBridgeContract::execute_upgrade(env.clone(), admin.clone(), 0);
        assert_eq!(
            err,
            Err(ContractError::CooldownActive),
            "after delay elapses, cooldown failure must trigger"
        );
    });
}

/// A failed `execute_upgrade` due to `CooldownActive` leaves no partial privileged state.
/// The proposal, approver roster, and last upgrade timestamp remain unchanged.
#[test]
fn test_multisig_upgrade_cooldown_preserves_state_on_failure() {
    let (env, admin, upgrader, _random, contract_id) = setup();
    env.ledger().set_timestamp(1_000);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_cooldown(env.clone(), 5_000).unwrap();
        env.storage().instance().set(&LAST_UPG_KEY, &1_000u64);
        TrustBridgeContract::set_upgrade_threshold(env.clone(), admin.clone(), 2).unwrap();
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0xCC),
            0,
        )
        .unwrap();
        TrustBridgeContract::approve_upgrade(env.clone(), upgrader.clone(), 0).unwrap();
    });

    // Attempt execution during cooldown window at ts 2_000
    env.ledger().set_timestamp(2_000);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let err = TrustBridgeContract::execute_upgrade(env.clone(), upgrader.clone(), 0);
        assert_eq!(err, Err(ContractError::CooldownActive));

        // Verify no partial state was written:
        // 1. Proposal is still live
        let prop = TrustBridgeContract::get_upgrade_proposal(env.clone())
            .expect("proposal must still exist");
        assert_eq!(prop.id, 0);
        assert_eq!(prop.wasm_hash, hash(&env, 0xCC));
        assert_eq!(prop.approvers.len(), 2);

        // 2. last_upgrade timestamp was NOT updated
        let last_upg: u64 = env.storage().instance().get(&LAST_UPG_KEY).unwrap();
        assert_eq!(last_upg, 1_000);

        // 3. Health cooldown remaining is correct
        let health = TrustBridgeContract::get_health(env.clone()).unwrap();
        assert_eq!(health.cooldown_remaining_secs, 4_000);
    });
}
