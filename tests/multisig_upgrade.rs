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

/// Uploads a minimal valid contract WASM so a successful `execute_upgrade` has
/// a real binary to swap in (same approach as `tests/auth_matrix.rs`).
///
/// The swap replaces this contract with a function-less module, so no further
/// contract entry points can be called afterwards in the same test; use the
/// `wasm-test` feature with the pre-built full WASM when post-execute calls
/// are needed.
#[allow(deprecated)] // `protocol_version` is the only way to match the host.
fn uploaded_wasm(env: &Env) -> BytesN<32> {
    use soroban_sdk::xdr::{Limits, ScEnvMetaEntry, ScEnvMetaEntryInterfaceVersion, WriteXdr};
    let meta = ScEnvMetaEntry::ScEnvMetaKindInterfaceVersion(ScEnvMetaEntryInterfaceVersion {
        protocol: env.ledger().protocol_version(),
        pre_release: 0,
    })
    .to_xdr(Limits::none())
    .unwrap();
    let name = b"contractenvmetav0";
    let mut section = std::vec![name.len() as u8];
    section.extend_from_slice(name);
    section.extend_from_slice(&meta);
    let mut wasm = std::vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x00];
    assert!(
        section.len() < 0x80,
        "custom section must fit a 1-byte LEB128 size"
    );
    wasm.push(section.len() as u8);
    wasm.extend_from_slice(&section);
    env.deployer()
        .upload_contract_wasm(soroban_sdk::Bytes::from_slice(env, &wasm))
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
    // One auth-bearing call per frame (recording-auth allows a single
    // `require_auth` per address per `as_contract` block).
    env.as_contract(&contract_id, || {
        // Nothing pending yet.
        assert_eq!(
            TrustBridgeContract::approve_upgrade(env.clone(), upgrader.clone(), 0),
            Err(ContractError::NoUpgradeProposalPending)
        );
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x01),
            0,
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
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
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x01),
            0,
        )
        .unwrap();
    });

    // Delay (0) has elapsed, but only the proposer has approved.
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
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x01),
            0,
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
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
    // One auth-bearing call per frame (recording-auth allows a single
    // `require_auth` per address per `as_contract` block).
    env.as_contract(&contract_id, || {
        // Nothing pending.
        assert_eq!(
            TrustBridgeContract::cancel_upgrade_proposal(env.clone(), admin.clone(), 0),
            Err(ContractError::NoUpgradeProposalPending)
        );
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x01),
            0,
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
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
    // One auth-bearing call per frame (recording-auth allows a single
    // `require_auth` per address per `as_contract` block).
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x01),
            0,
        )
        .unwrap();
    });

    // Admin cancels proposal 0.
    env.as_contract(&contract_id, || {
        TrustBridgeContract::cancel_upgrade_proposal(env.clone(), admin.clone(), 0).unwrap();
    });

    env.as_contract(&contract_id, || {
        // Approval on cancelled proposal fails with NoUpgradeProposalPending.
        assert_eq!(
            TrustBridgeContract::approve_upgrade(env.clone(), upgrader.clone(), 0),
            Err(ContractError::NoUpgradeProposalPending)
        );
    });

    env.as_contract(&contract_id, || {
        // Execution on cancelled proposal fails with NoUpgradeProposalPending.
        assert_eq!(
            TrustBridgeContract::execute_upgrade(env.clone(), admin.clone(), 0),
            Err(ContractError::NoUpgradeProposalPending)
        );
    });

    env.as_contract(&contract_id, || {
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

// ── Mid-flight attestation mutations (Issue #592) ────────────────────────────
//
// `execute_upgrade` reads `attestation_required` and the attestation record at
// *execution* time, so between propose and execute the policy must be
// tamper-proof: the gate cannot be switched off mid-flight, and clearing the
// attestation record fails closed instead of opening the gate.  The lock
// releases when the proposal executes or is cancelled.  Semantics are
// documented in `docs/SECURITY.md` under the M-of-N multisig upgrade section.

/// The core of Issue #592: a live proposal cannot have its attestation gate
/// disabled underneath it, execute still honours the gate it was proposed
/// under, and the lock releases with execution.
#[test]
fn test_cannot_disable_attestation_gate_while_proposal_pending() {
    let (env, admin, upgrader, _random, contract_id) = setup();
    env.ledger().set_timestamp(1_000);

    // A real uploadable WASM, so execute can complete the swap.
    let wasm_hash = uploaded_wasm(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_attestation_required(env.clone(), admin.clone(), true).unwrap();
    });
    env.as_contract(&contract_id, || {
        // The attestation the execute gate will require.
        TrustBridgeContract::attest_upgrade(env.clone(), wasm_hash.clone(), 10_000).unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_upgrade_threshold(env.clone(), admin.clone(), 2).unwrap();
    });
    env.as_contract(&contract_id, || {
        // The proposer counts as approval #1.
        TrustBridgeContract::propose_multisig_upgrade(env.clone(), admin.clone(), wasm_hash, 0)
            .unwrap();
    });

    // The core assertion: the gate cannot be switched off mid-flight.
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::set_attestation_required(env.clone(), admin.clone(), false),
            Err(ContractError::UpgradeProposalAlreadyPending),
            "disabling the attestation gate must be refused while an upgrade proposal is live"
        );
    });
    env.as_contract(&contract_id, || {
        assert!(
            TrustBridgeContract::get_attestation_required(env.clone()),
            "the refused call must leave the flag on"
        );
    });

    // A second distinct approver meets threshold 2; execute proceeds *under*
    // the gate — attestation present, flag still required.
    env.as_contract(&contract_id, || {
        TrustBridgeContract::approve_upgrade(env.clone(), upgrader.clone(), 0).unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::execute_upgrade(env.clone(), upgrader.clone(), 0)
            .expect("execute must succeed under the gate it was proposed with");
    });

    // The executed upgrade consumed the proposal, so the lock is gone.
    env.as_contract(&contract_id, || {
        assert!(
            TrustBridgeContract::get_upgrade_proposal(env.clone()).is_none(),
            "an executed upgrade must clear the pending proposal"
        );
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_attestation_required(env.clone(), admin.clone(), false).unwrap();
        assert!(
            !TrustBridgeContract::get_attestation_required(env.clone()),
            "the lock must not outlive the proposal"
        );
    });
}

/// `clear_attestation` stays available mid-flight (it is the documented
/// escape hatch) but can never open the gate: the flag is untouched, so
/// execute fails closed with `AttestationRequired`.
#[test]
fn test_clear_attestation_mid_flight_fails_closed_at_execute() {
    let (env, admin, _upgrader, _random, contract_id) = setup();
    env.ledger().set_timestamp(1_000);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_attestation_required(env.clone(), admin.clone(), true).unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::attest_upgrade(env.clone(), hash(&env, 0x02), 10_000).unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x02),
            0,
        )
        .unwrap();
    });

    // Allowed mid-flight — the escape hatch for an attestation published in error.
    env.as_contract(&contract_id, || {
        TrustBridgeContract::clear_attestation(env.clone()).unwrap();
    });
    env.as_contract(&contract_id, || {
        assert!(
            TrustBridgeContract::get_attestation_required(env.clone()),
            "clearing the record must not disable the gate"
        );
    });

    // Execute must fail closed rather than proceed unattested.
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::execute_upgrade(env.clone(), admin.clone(), 0),
            Err(ContractError::AttestationRequired),
            "a cleared attestation must not let execute bypass the required gate"
        );
    });
}

/// Cancelling the proposal releases the gate lock.
#[test]
fn test_cancel_releases_attestation_gate_lock() {
    let (env, admin, _upgrader, _random, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_attestation_required(env.clone(), admin.clone(), true).unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x03),
            0,
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::set_attestation_required(env.clone(), admin.clone(), false),
            Err(ContractError::UpgradeProposalAlreadyPending),
            "the gate must stay locked while the proposal is live"
        );
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::cancel_upgrade_proposal(env.clone(), admin.clone(), 0).unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_attestation_required(env.clone(), admin.clone(), false).unwrap();
        assert!(
            !TrustBridgeContract::get_attestation_required(env.clone()),
            "cancelling must release the lock"
        );
    });
}

/// Tightening mid-flight is deliberately allowed (it only strengthens) and
/// applies to the pending proposal at execute time.
#[test]
fn test_enabling_attestation_mid_flight_applies_at_execute() {
    let (env, admin, _upgrader, _random, contract_id) = setup();
    env.ledger().set_timestamp(1_000);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            hash(&env, 0x04),
            0,
        )
        .unwrap();
    });

    // Strengthening is allowed while the proposal is live.
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_attestation_required(env.clone(), admin.clone(), true).unwrap();
    });

    // And the tightened gate applies to the live proposal: no attestation
    // exists, so execute fails closed.
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::execute_upgrade(env.clone(), admin.clone(), 0),
            Err(ContractError::AttestationRequired),
            "an enabled gate must apply to the proposal already in flight"
        );
    });
}

/// Optional mode (flag off, the default) behaves differently by design:
/// there is no gate to bypass, so clearing (or never attesting) does not
/// block execute.
#[test]
fn test_optional_mode_clear_mid_flight_does_not_block_execute() {
    let (env, admin, _upgrader, _random, contract_id) = setup();
    env.ledger().set_timestamp(1_000);

    // A real uploadable WASM, so execute can complete the swap.
    let wasm_hash = uploaded_wasm(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert!(
            !TrustBridgeContract::get_attestation_required(env.clone()),
            "optional mode is the default"
        );
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_multisig_upgrade(env.clone(), admin.clone(), wasm_hash, 0)
            .unwrap();
    });
    env.as_contract(&contract_id, || {
        // Nothing to clear, and the gate is off either way.
        TrustBridgeContract::clear_attestation(env.clone()).unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::execute_upgrade(env.clone(), admin.clone(), 0)
            .expect("optional mode must not require an attestation at execute");
    });
}
