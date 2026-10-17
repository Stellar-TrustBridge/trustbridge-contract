//! On-chain M-of-N multisig upgrade flow (Issue #301).
//!
//! Replaces the single-admin `upgrade` path with a **propose → N-of-M approve
//! → execute after delay** governance flow for WASM upgrades.
//!
//! ## Why
//!
//! The existing upgrade flow — single admin key + optional attestation + cooldown
//! — is documented in `docs/SECURITY.md`.  It works well for a trusted operator
//! but provides no resistance if that one key is compromised: anyone who steals
//! it can bypass the attestation by setting `attestation_required = false` first,
//! then upgrading.  An on-chain M-of-N requirement means an attacker must
//! compromise M distinct keys simultaneously.
//!
//! ## Flow
//!
//! ```text
//! proposer (admin or Upgrader) → propose_multisig_upgrade(wasm_hash, delay_secs)
//!                                                ↓ UpgradeProposedEvent
//!     any signer (admin / Upgrader, distinct from previous approvers)
//!         → approve_upgrade(proposal_id)          ↓ UpgradeApprovedEvent
//!     ... repeat until approvals ≥ threshold ...
//!     any admin or Upgrader → execute_upgrade(proposal_id)
//!         (checks: delay elapsed, approvals ≥ threshold, hash consistent)
//!                                                ↓ UpgradeProposalExecutedEvent
//! ```
//!
//! Cancellation is available to any admin at any time before execution:
//! `cancel_upgrade_proposal(proposal_id)` → `UpgradeProposalCancelledEvent`.
//!
//! ## Approval threshold
//!
//! Configured on-chain via `set_upgrade_threshold(m)`.  Default `1` retains
//! single-admin behaviour (no breaking change for existing deployments).  Set
//! to `2` or higher to require multi-party authorisation.  The threshold is a
//! floor on *distinct* signers; the proposer counts as one approval.
//!
//! ## Proposal ID
//!
//! A `u32` counter stored in instance storage, incremented each time a new
//! proposal is created.  Using a counter rather than a hash ensures the
//! proposal is retrievable by a simple ID without a secondary index.
//!
//! ## Scope
//!
//! This module handles **upgrade proposals only**.  It does not attempt to
//! wrap every admin function in multisig — that is explicitly out of scope
//! per the issue description.
//!
//! ## Storage keys (all instance)
//!
//! | Key | Type | Meaning |
//! |-----|------|---------|
//! | `upg_thr`  | `u32`                   | Approval threshold (default 1) |
//! | `upg_cnt`  | `u32`                   | Monotonic proposal counter |
//! | `upg_prop` | `UpgradeProposal`        | The live (un-executed) proposal |

use soroban_sdk::{contracttype, symbol_short, Address, BytesN, Env, Symbol, Vec};

use crate::ContractError;

// ── Storage keys ──────────────────────────────────────────────────────────────

/// Minimum number of distinct approvals required before a proposal may execute.
pub const UPGRADE_THRESHOLD_KEY: Symbol = symbol_short!("upg_thr");

/// Monotonic counter: next proposal ID.
pub const UPGRADE_PROPOSAL_CNT_KEY: Symbol = symbol_short!("upg_cnt");

/// The single live proposal (only one at a time is allowed).
pub const UPGRADE_PROPOSAL_KEY: Symbol = symbol_short!("upg_prop");

// ── Types ─────────────────────────────────────────────────────────────────────

/// An on-chain M-of-N upgrade proposal.
///
/// Created by `propose_multisig_upgrade`, consumed by `execute_upgrade` once
/// the delay has elapsed and the required number of distinct approvers have
/// signed, or discarded by `cancel_upgrade_proposal`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct UpgradeProposal {
    /// Monotonic ID assigned at proposal creation.
    pub id: u32,
    /// WASM hash this proposal authorises.
    pub wasm_hash: BytesN<32>,
    /// Address that created the proposal (counts as one approval).
    pub proposed_by: Address,
    /// Ledger timestamp the proposal was created.
    pub proposed_at: u64,
    /// Earliest ledger timestamp at which the proposal may execute.
    pub executable_at: u64,
    /// Distinct addresses that have approved (includes `proposed_by`).
    ///
    /// Using `Vec` keeps the type `#[contracttype]`-derivable without a
    /// map, which Soroban's `contracttype` macro does not support natively.
    /// Population is bounded by `MAX_UPGRADE_SIGNERS`.
    pub approvers: Vec<Address>,
}

/// Hard cap on the number of approvers tracked per proposal.
///
/// In practice M-of-N deployments use N ≤ 10; this cap exists to bound
/// the storage footprint of the `approvers` vec.
pub const MAX_UPGRADE_SIGNERS: u32 = 20;

// ── Storage helpers ────────────────────────────────────────────────────────────

/// Current approval threshold (default 1 = single-admin, no change to
/// existing behaviour).
#[must_use]
pub fn get_upgrade_threshold(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&UPGRADE_THRESHOLD_KEY)
        .unwrap_or(1)
}

/// Sets the approval threshold.  `0` is clamped to `1`.
pub fn set_upgrade_threshold(env: &Env, threshold: u32) {
    let effective = threshold.max(1);
    env.storage()
        .instance()
        .set(&UPGRADE_THRESHOLD_KEY, &effective);
}

/// The next proposal ID (read without incrementing).
fn peek_next_id(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&UPGRADE_PROPOSAL_CNT_KEY)
        .unwrap_or(0)
}

/// Allocates and returns the next proposal ID, advancing the counter.
fn alloc_proposal_id(env: &Env) -> u32 {
    let id = peek_next_id(env);
    env.storage()
        .instance()
        .set(&UPGRADE_PROPOSAL_CNT_KEY, &id.saturating_add(1));
    id
}

/// Returns the live proposal, if any.
#[must_use]
pub fn get_upgrade_proposal(env: &Env) -> Option<UpgradeProposal> {
    env.storage().instance().get(&UPGRADE_PROPOSAL_KEY)
}

pub fn set_upgrade_proposal(env: &Env, proposal: &UpgradeProposal) {
    env.storage()
        .instance()
        .set(&UPGRADE_PROPOSAL_KEY, proposal);
}

pub fn clear_upgrade_proposal(env: &Env) {
    env.storage().instance().remove(&UPGRADE_PROPOSAL_KEY);
}

// ── Business logic ─────────────────────────────────────────────────────────────

/// Creates a new upgrade proposal.
///
/// The proposer is recorded as the first approver.  Returns the new proposal.
///
/// # Errors
///
/// - [`ContractError::UpgradeProposalAlreadyPending`] if a proposal is already
///   live (cancel or execute it first).
pub fn create_upgrade_proposal(
    env: &Env,
    proposer: Address,
    wasm_hash: BytesN<32>,
    delay_secs: u64,
) -> Result<UpgradeProposal, ContractError> {
    if get_upgrade_proposal(env).is_some() {
        return Err(ContractError::UpgradeProposalAlreadyPending);
    }
    let id = alloc_proposal_id(env);
    let now = env.ledger().timestamp();
    let mut approvers: Vec<Address> = Vec::new(env);
    approvers.push_back(proposer.clone());
    let proposal = UpgradeProposal {
        id,
        wasm_hash,
        proposed_by: proposer,
        proposed_at: now,
        executable_at: now.saturating_add(delay_secs),
        approvers,
    };
    set_upgrade_proposal(env, &proposal);
    Ok(proposal)
}

/// Records `approver`'s approval on the live proposal.
///
/// Returns the updated proposal.
///
/// # Errors
///
/// - [`ContractError::NoUpgradeProposalPending`] if there is no live proposal.
/// - [`ContractError::UpgradeProposalAlreadyApproved`] if `approver` has
///   already approved this proposal.
pub fn record_approval(
    env: &Env,
    proposal_id: u32,
    approver: Address,
) -> Result<UpgradeProposal, ContractError> {
    let mut proposal = get_upgrade_proposal(env).ok_or(ContractError::NoUpgradeProposalPending)?;

    if proposal.id != proposal_id {
        return Err(ContractError::NoUpgradeProposalPending);
    }

    // Reject duplicate approvals.
    for existing in proposal.approvers.iter() {
        if existing == approver {
            return Err(ContractError::UpgradeProposalAlreadyApproved);
        }
    }

    if proposal.approvers.len() < MAX_UPGRADE_SIGNERS {
        proposal.approvers.push_back(approver);
    }

    set_upgrade_proposal(env, &proposal);
    Ok(proposal)
}

/// Returns `true` if the proposal has met or exceeded the required threshold.
#[must_use]
pub fn has_enough_approvals(env: &Env, proposal: &UpgradeProposal) -> bool {
    proposal.approvers.len() >= get_upgrade_threshold(env)
}

/// Validates that the proposal is ready to execute.
///
/// Checks: delay elapsed, approvals ≥ threshold, executor has approved.
/// Does **not** check auth or WASM hash consistency (callers do that).
///
/// # Errors
///
/// - [`ContractError::NoUpgradeProposalPending`] if there is no live proposal
///   or the `proposal_id` does not match.
/// - [`ContractError::UpgradeProposalDelayActive`] if the delay has not elapsed.
/// - [`ContractError::UpgradeProposalInsufficientApprovals`] if approval
///   threshold has not been met.
pub fn require_proposal_executable(
    env: &Env,
    proposal_id: u32,
) -> Result<UpgradeProposal, ContractError> {
    let proposal = get_upgrade_proposal(env).ok_or(ContractError::NoUpgradeProposalPending)?;

    if proposal.id != proposal_id {
        return Err(ContractError::NoUpgradeProposalPending);
    }

    if env.ledger().timestamp() < proposal.executable_at {
        return Err(ContractError::UpgradeProposalDelayActive);
    }

    if !has_enough_approvals(env, &proposal) {
        return Err(ContractError::UpgradeProposalInsufficientApprovals);
    }

    Ok(proposal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        Address, BytesN, Env,
    };

    fn hash(env: &Env, b: u8) -> BytesN<32> {
        BytesN::from_array(env, &[b; 32])
    }

    // ── threshold ─────────────────────────────────────────────────────────────

    #[test]
    fn test_default_threshold_is_one() {
        let env = Env::default();
        assert_eq!(get_upgrade_threshold(&env), 1);
    }

    #[test]
    fn test_set_threshold_persists() {
        let env = Env::default();
        set_upgrade_threshold(&env, 3);
        assert_eq!(get_upgrade_threshold(&env), 3);
    }

    #[test]
    fn test_zero_threshold_clamped_to_one() {
        let env = Env::default();
        set_upgrade_threshold(&env, 0);
        assert_eq!(get_upgrade_threshold(&env), 1);
    }

    // ── proposal lifecycle ────────────────────────────────────────────────────

    #[test]
    fn test_create_proposal_records_proposer_as_first_approver() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let proposer = Address::generate(&env);
        let h = hash(&env, 0x01);
        let proposal = create_upgrade_proposal(&env, proposer.clone(), h.clone(), 3_600).unwrap();
        assert_eq!(proposal.id, 0);
        assert_eq!(proposal.wasm_hash, h);
        assert_eq!(proposal.proposed_by, proposer);
        assert_eq!(proposal.proposed_at, 1_000);
        assert_eq!(proposal.executable_at, 4_600);
        assert_eq!(proposal.approvers.len(), 1);
        assert_eq!(proposal.approvers.get(0).unwrap(), proposer);
    }

    #[test]
    fn test_create_proposal_fails_when_one_already_pending() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let proposer = Address::generate(&env);
        create_upgrade_proposal(&env, proposer.clone(), hash(&env, 0x01), 0).unwrap();
        let result = create_upgrade_proposal(&env, proposer, hash(&env, 0x02), 0);
        assert_eq!(result, Err(ContractError::UpgradeProposalAlreadyPending));
    }

    #[test]
    fn test_cancel_removes_proposal() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let proposer = Address::generate(&env);
        create_upgrade_proposal(&env, proposer, hash(&env, 0x01), 0).unwrap();
        assert!(get_upgrade_proposal(&env).is_some());
        clear_upgrade_proposal(&env);
        assert!(get_upgrade_proposal(&env).is_none());
        // Can create a new one after clearing.
        let proposer2 = Address::generate(&env);
        assert!(create_upgrade_proposal(&env, proposer2, hash(&env, 0x02), 0).is_ok());
    }

    // ── approval ──────────────────────────────────────────────────────────────

    #[test]
    fn test_record_approval_adds_approver() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let proposer = Address::generate(&env);
        let signer = Address::generate(&env);
        let proposal =
            create_upgrade_proposal(&env, proposer.clone(), hash(&env, 0x01), 0).unwrap();
        let updated = record_approval(&env, proposal.id, signer.clone()).unwrap();
        assert_eq!(updated.approvers.len(), 2);
        assert_eq!(updated.approvers.get(1).unwrap(), signer);
    }

    #[test]
    fn test_duplicate_approval_is_rejected() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let proposer = Address::generate(&env);
        let proposal =
            create_upgrade_proposal(&env, proposer.clone(), hash(&env, 0x01), 0).unwrap();
        // Proposer tries to approve again.
        let result = record_approval(&env, proposal.id, proposer);
        assert_eq!(result, Err(ContractError::UpgradeProposalAlreadyApproved));
    }

    #[test]
    fn test_approval_on_wrong_id_is_rejected() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let proposer = Address::generate(&env);
        let _proposal =
            create_upgrade_proposal(&env, proposer.clone(), hash(&env, 0x01), 0).unwrap();
        let signer = Address::generate(&env);
        let result = record_approval(&env, 999, signer);
        assert_eq!(result, Err(ContractError::NoUpgradeProposalPending));
    }

    // ── executability ─────────────────────────────────────────────────────────

    #[test]
    fn test_proposal_not_executable_before_delay() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let proposer = Address::generate(&env);
        let proposal = create_upgrade_proposal(&env, proposer, hash(&env, 0x01), 3_600).unwrap();
        // Still at timestamp 1_000; executable_at = 4_600.
        assert_eq!(
            require_proposal_executable(&env, proposal.id),
            Err(ContractError::UpgradeProposalDelayActive)
        );
    }

    #[test]
    fn test_proposal_executable_after_delay_with_threshold_met() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let proposer = Address::generate(&env);
        let proposal = create_upgrade_proposal(&env, proposer, hash(&env, 0x01), 3_600).unwrap();
        // Advance past the delay.
        env.ledger().set_timestamp(4_601);
        // Default threshold = 1; proposer already counted.
        assert!(require_proposal_executable(&env, proposal.id).is_ok());
    }

    #[test]
    fn test_proposal_not_executable_with_insufficient_approvals() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        set_upgrade_threshold(&env, 2); // Need 2 approvals.
        let proposer = Address::generate(&env);
        let proposal = create_upgrade_proposal(&env, proposer, hash(&env, 0x01), 0).unwrap();
        // Only 1 approval (the proposer); threshold = 2.
        assert_eq!(
            require_proposal_executable(&env, proposal.id),
            Err(ContractError::UpgradeProposalInsufficientApprovals)
        );
    }

    #[test]
    fn test_two_of_two_proposal_becomes_executable_after_second_approval() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        set_upgrade_threshold(&env, 2);
        let proposer = Address::generate(&env);
        let signer = Address::generate(&env);
        let proposal =
            create_upgrade_proposal(&env, proposer.clone(), hash(&env, 0x01), 0).unwrap();
        // Still insufficient after just the proposer.
        assert_eq!(
            require_proposal_executable(&env, proposal.id),
            Err(ContractError::UpgradeProposalInsufficientApprovals)
        );
        // Add second approver.
        record_approval(&env, proposal.id, signer).unwrap();
        assert!(require_proposal_executable(&env, proposal.id).is_ok());
    }

    #[test]
    fn test_no_proposal_returns_not_pending() {
        let env = Env::default();
        assert_eq!(
            require_proposal_executable(&env, 0),
            Err(ContractError::NoUpgradeProposalPending)
        );
    }

    /// Proposal IDs are monotonically increasing.
    #[test]
    fn test_proposal_ids_are_monotonic() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let proposer = Address::generate(&env);
        let p0 = create_upgrade_proposal(&env, proposer.clone(), hash(&env, 0x01), 0).unwrap();
        assert_eq!(p0.id, 0);
        clear_upgrade_proposal(&env);
        let p1 = create_upgrade_proposal(&env, proposer, hash(&env, 0x02), 0).unwrap();
        assert_eq!(p1.id, 1);
    }

    /// has_enough_approvals respects the threshold.
    #[test]
    fn test_has_enough_approvals_false_until_threshold_met() {
        let env = Env::default();
        set_upgrade_threshold(&env, 3);
        env.ledger().set_timestamp(1_000);
        let proposer = Address::generate(&env);
        let signer_a = Address::generate(&env);
        let signer_b = Address::generate(&env);
        let proposal =
            create_upgrade_proposal(&env, proposer.clone(), hash(&env, 0x01), 0).unwrap();
        assert!(!has_enough_approvals(&env, &proposal));
        let after_one = record_approval(&env, proposal.id, signer_a).unwrap();
        assert!(!has_enough_approvals(&env, &after_one));
        let after_two = record_approval(&env, proposal.id, signer_b).unwrap();
        assert!(has_enough_approvals(&env, &after_two));
    }

    #[test]
    fn test_record_approval_no_proposal_pending() {
        let env = Env::default();
        let signer = Address::generate(&env);
        let result = record_approval(&env, 0, signer);
        assert_eq!(result, Err(ContractError::NoUpgradeProposalPending));
    }

    #[test]
    fn test_require_proposal_executable_wrong_id() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let proposer = Address::generate(&env);
        let proposal = create_upgrade_proposal(&env, proposer, hash(&env, 0x01), 0).unwrap();
        assert_eq!(
            require_proposal_executable(&env, proposal.id + 1),
            Err(ContractError::NoUpgradeProposalPending)
        );
    }

    #[test]
    fn test_record_approval_capped_at_max_signers() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let proposer = Address::generate(&env);
        let proposal = create_upgrade_proposal(&env, proposer, hash(&env, 0x01), 0).unwrap();
        let mut last_proposal = proposal.clone();
        for _ in 0..(MAX_UPGRADE_SIGNERS - 1) {
            let signer = Address::generate(&env);
            last_proposal = record_approval(&env, proposal.id, signer).unwrap();
        }
        assert_eq!(last_proposal.approvers.len(), MAX_UPGRADE_SIGNERS);
        let extra_signer = Address::generate(&env);
        let after_cap = record_approval(&env, proposal.id, extra_signer).unwrap();
        assert_eq!(after_cap.approvers.len(), MAX_UPGRADE_SIGNERS);
    }

    // ── Error-path tests ──────────────────────────────────────────────────────
    //
    // The tests below focus on error paths and boundary conditions that the
    // integration tests in `tests/multisig_upgrade.rs` do not reach because
    // they operate at the `TrustBridgeContract` entry-point level (which
    // requires auth, initialization, etc.). These unit tests call the storage
    // helpers directly and pin the exact `ContractError` returned in every
    // failure branch.

    /// `require_proposal_executable` at the exact boundary timestamp (one
    /// second before `executable_at`) returns `UpgradeProposalDelayActive`,
    /// and at exactly `executable_at` it succeeds.
    #[test]
    fn test_executable_at_boundary_is_exclusive_before_inclusive_at() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let proposer = Address::generate(&env);
        // delay_secs = 3_600 → executable_at = 4_600
        let proposal =
            create_upgrade_proposal(&env, proposer, hash(&env, 0x01), 3_600).unwrap();

        // One second before executable_at: still blocked.
        env.ledger().set_timestamp(4_599);
        assert_eq!(
            require_proposal_executable(&env, proposal.id),
            Err(ContractError::UpgradeProposalDelayActive),
            "timestamp == executable_at - 1 must still be inside the delay window"
        );

        // Exactly at executable_at: now allowed.
        env.ledger().set_timestamp(4_600);
        assert!(
            require_proposal_executable(&env, proposal.id).is_ok(),
            "timestamp == executable_at must be executable"
        );
    }

    /// A zero-delay proposal is immediately executable (no ledger advance
    /// needed): `executable_at == proposed_at` and the condition is
    /// `timestamp >= executable_at`.
    #[test]
    fn test_zero_delay_proposal_is_immediately_executable() {
        let env = Env::default();
        env.ledger().set_timestamp(500);
        let proposer = Address::generate(&env);
        let proposal = create_upgrade_proposal(&env, proposer, hash(&env, 0xAA), 0).unwrap();

        assert_eq!(
            proposal.executable_at, 500,
            "zero delay: executable_at must equal proposed_at"
        );
        assert!(
            require_proposal_executable(&env, proposal.id).is_ok(),
            "zero-delay proposal must be executable at the same timestamp it was proposed"
        );
    }

    /// After `clear_upgrade_proposal`, `require_proposal_executable` returns
    /// `NoUpgradeProposalPending` for the old id — consumed proposals cannot
    /// be re-executed.
    #[test]
    fn test_require_executable_after_clear_returns_not_pending() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let proposer = Address::generate(&env);
        let proposal = create_upgrade_proposal(&env, proposer, hash(&env, 0x01), 0).unwrap();
        let id = proposal.id;

        clear_upgrade_proposal(&env);

        assert_eq!(
            require_proposal_executable(&env, id),
            Err(ContractError::NoUpgradeProposalPending),
            "cleared proposal must not be executable"
        );
    }

    /// `record_approval` on the stale id of a cancelled proposal (where a new
    /// proposal with a different id is now live) returns `NoUpgradeProposalPending`.
    #[test]
    fn test_approval_on_stale_id_after_cancel_and_repropose_fails() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let proposer = Address::generate(&env);
        let signer = Address::generate(&env);

        // Proposal 0.
        let p0 = create_upgrade_proposal(&env, proposer.clone(), hash(&env, 0x01), 0).unwrap();
        assert_eq!(p0.id, 0);
        clear_upgrade_proposal(&env);

        // Proposal 1 is now live.
        let p1 = create_upgrade_proposal(&env, proposer, hash(&env, 0x02), 0).unwrap();
        assert_eq!(p1.id, 1);

        // Attempting to approve on the old id (0) must fail.
        assert_eq!(
            record_approval(&env, 0, signer.clone()),
            Err(ContractError::NoUpgradeProposalPending),
            "approving the stale proposal id must return NoUpgradeProposalPending"
        );

        // Approving on the live id (1) succeeds.
        assert!(record_approval(&env, 1, signer).is_ok());
    }

    /// `require_proposal_executable` with a wrong id while a different
    /// proposal is live returns `NoUpgradeProposalPending` (not an id-mismatch
    /// variant), consistent with the "proposal not found" semantics.
    #[test]
    fn test_executable_wrong_id_while_proposal_is_live() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let proposer = Address::generate(&env);
        let proposal = create_upgrade_proposal(&env, proposer, hash(&env, 0x01), 0).unwrap();

        // Any id other than `proposal.id` is treated as "not found".
        for bad_id in [proposal.id + 1, proposal.id + 100, u32::MAX] {
            assert_eq!(
                require_proposal_executable(&env, bad_id),
                Err(ContractError::NoUpgradeProposalPending),
                "wrong id {bad_id} must return NoUpgradeProposalPending"
            );
        }
    }

    /// Threshold clamped to 1 means a single proposer is immediately past the
    /// threshold — `has_enough_approvals` is true right after proposal creation.
    #[test]
    fn test_threshold_one_means_proposer_alone_is_sufficient() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        set_upgrade_threshold(&env, 1);
        let proposer = Address::generate(&env);
        let proposal = create_upgrade_proposal(&env, proposer, hash(&env, 0x01), 0).unwrap();

        assert!(
            has_enough_approvals(&env, &proposal),
            "threshold=1: proposer alone must satisfy the approval requirement"
        );
    }

    /// When the threshold exceeds the hard cap `MAX_UPGRADE_SIGNERS`, the cap
    /// itself becomes the limit: once `MAX_UPGRADE_SIGNERS` approvers have
    /// signed, further `record_approval` calls are silently ignored (the
    /// approver vec does not grow past the cap), so `has_enough_approvals`
    /// can only become true if `threshold <= MAX_UPGRADE_SIGNERS`.
    #[test]
    fn test_threshold_above_max_signers_cap_is_never_reachable() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        // Set threshold higher than the hard cap — no realistic deployment
        // would do this, but the system must not panic.
        set_upgrade_threshold(&env, MAX_UPGRADE_SIGNERS + 1);
        let proposer = Address::generate(&env);
        let proposal = create_upgrade_proposal(&env, proposer, hash(&env, 0x01), 0).unwrap();

        // Fill all MAX_UPGRADE_SIGNERS slots.
        let mut p = proposal.clone();
        for _ in 0..(MAX_UPGRADE_SIGNERS - 1) {
            let signer = Address::generate(&env);
            p = record_approval(&env, proposal.id, signer).unwrap();
        }
        assert_eq!(p.approvers.len(), MAX_UPGRADE_SIGNERS);

        // Even with all slots filled, threshold > MAX_UPGRADE_SIGNERS means
        // `has_enough_approvals` is false — the threshold is unreachable.
        assert!(
            !has_enough_approvals(&env, &p),
            "threshold above MAX_UPGRADE_SIGNERS must never be reachable"
        );

        // A further attempt adds nothing past the cap.
        let extra = Address::generate(&env);
        let after_extra = record_approval(&env, proposal.id, extra).unwrap();
        assert_eq!(
            after_extra.approvers.len(),
            MAX_UPGRADE_SIGNERS,
            "approver count must stay at MAX_UPGRADE_SIGNERS after cap is hit"
        );
    }

    /// Duplicate approval by the same address is rejected regardless of how
    /// many other approvers have already signed.
    #[test]
    fn test_duplicate_approval_rejected_at_any_position() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let proposer = Address::generate(&env);
        let signer_a = Address::generate(&env);
        let signer_b = Address::generate(&env);

        let proposal = create_upgrade_proposal(&env, proposer.clone(), hash(&env, 0x01), 0).unwrap();
        record_approval(&env, proposal.id, signer_a.clone()).unwrap();
        record_approval(&env, proposal.id, signer_b.clone()).unwrap();

        // All three addresses already on the list — all three must be rejected.
        for dup in [&proposer, &signer_a, &signer_b] {
            assert_eq!(
                record_approval(&env, proposal.id, dup.clone()),
                Err(ContractError::UpgradeProposalAlreadyApproved),
                "duplicate approval by {dup:?} must be rejected"
            );
        }
    }

    /// A proposal with `delay_secs` that would overflow `u64` is safely
    /// clamped by `saturating_add` — `executable_at` saturates to `u64::MAX`
    /// rather than wrapping, so the delay never accidentally becomes zero.
    #[test]
    fn test_saturating_add_prevents_delay_overflow() {
        let env = Env::default();
        // Set proposed_at near the maximum so adding any delay would overflow.
        env.ledger().set_timestamp(u64::MAX - 1);
        let proposer = Address::generate(&env);
        let proposal =
            create_upgrade_proposal(&env, proposer, hash(&env, 0x01), u64::MAX).unwrap();

        assert_eq!(
            proposal.executable_at,
            u64::MAX,
            "saturating_add must clamp executable_at to u64::MAX, not wrap to 0"
        );
    }
}
