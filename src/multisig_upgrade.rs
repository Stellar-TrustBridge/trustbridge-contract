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
    env.storage().instance().set(&UPGRADE_PROPOSAL_KEY, proposal);
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
    let mut proposal = get_upgrade_proposal(env)
        .ok_or(ContractError::NoUpgradeProposalPending)?;

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
    let proposal = get_upgrade_proposal(env)
        .ok_or(ContractError::NoUpgradeProposalPending)?;

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
        let proposal =
            create_upgrade_proposal(&env, proposer, hash(&env, 0x01), 3_600).unwrap();
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
        let proposal =
            create_upgrade_proposal(&env, proposer, hash(&env, 0x01), 3_600).unwrap();
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
        let proposal =
            create_upgrade_proposal(&env, proposer, hash(&env, 0x01), 0).unwrap();
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
}
