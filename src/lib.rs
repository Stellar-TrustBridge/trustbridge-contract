#![no_std]

mod audit;
mod batch;
mod contract;
mod domain;
mod error;
mod events;
mod merkle;
mod multisig_upgrade;
mod oracle_proof;
mod staged_wasm;
mod storage;
mod utils;
mod version;

pub use audit::{AuditConfig, AuditEventType, AuditLogEntry, AuditStats};
pub use batch::{BatchConfig, BatchOperationResult, BatchSummary, MAX_WRITE_BATCH};
pub use domain::{EventDomain, EVENT_DOMAIN_VERSION};
pub use error::{ContractError, ErrorCategory};
pub use events::{
    AttestationClearedEvent,
    // Audit config (Issue #412)
    AuditConfigChangedEvent,
    BatchRemoveCancelledEvent,
    BatchRemoveExecutedEvent,
    BatchRemoveProposedEvent,
    BotStatusChangedEvent,
    ChallengeCancelledEvent,
    ChallengeCompletedEvent,
    ChallengeStartedEvent,
    EmergencyClearedEvent,
    EmergencyPausedEvent,
    GuardianChangedEvent,
    PausedEvent,
    RegisteredEvent,
    RemovedEvent,
    RenamedEvent,
    RoleGrantCancelledEvent,
    RoleGrantPendingEvent,
    RoleGrantedEvent,
    RoleRevokedEvent,
    RotationCancelledEvent,
    RotationExecutedEvent,
    RotationRequestedEvent,
    StagedWasmClearedEvent,
    UnpausedEvent,
    UpgradeApprovedEvent,
    UpgradeAttestedEvent,
    UpgradeProposalCancelledEvent,
    UpgradeProposalExecutedEvent,
    // Multisig upgrade (Issue #301)
    UpgradeProposedEvent,
    UpgradedEvent,
    VerificationConfiguredEvent,
    VerificationRevokedEvent,
    VerifiedEvent,
    // Staged WASM (Issue #300)
    WasmStagedEvent,
};
pub use multisig_upgrade::{UpgradeProposal, MAX_UPGRADE_SIGNERS};
pub use oracle_proof::{
    get_oracle_allowlist, set_oracle_allowlist, verify_with_proof, OracleProof,
};
pub use staged_wasm::StagedWasm;
pub use storage::{
    ChallengeRecord, ContributorRecord, ExportAttestation, ExportPage, HealthProbes, HealthSnapshot,
    PauseReason, PendingBatchRemove, PendingRoleGrant, PendingRotation, RecordProof, RepairReport,
    Role, RoleHolder, Stats, VerificationConfig, VerifierAllowEntry, WasmAttestation, WasmProvenance,
    MAX_FALLBACK_ADDRESSES, MAX_VERIFIERS,
};
pub use version::Version;

use crate::storage::get_public_paginated_internal;

use soroban_sdk::{contract, contractimpl, Address, BytesN, Env, String, Symbol, Vec};

use crate::storage::{
    add_to_index, add_verifier as storage_add_verifier, build_record_proof,
    bump_ever_verified_count, charge_verify_rate, clear_pending_reverify, get_admin,
    get_audit_config as storage_get_audit_config, get_audit_logs, get_audit_stats, get_challenge,
    get_cooldown as storage_get_cooldown, get_count, get_emergency_pause, get_emergency_pause_ts,
    get_ever_verified_count as storage_get_ever_verified_count,
    get_guardian as storage_get_guardian, get_index,
    get_last_event_ledger as storage_get_last_event_ledger, get_last_upgrade,
    get_network_id as storage_get_network_id, get_pending_reverify,
    get_pending_role as storage_get_pending_role,
    get_pending_rotation as storage_get_pending_rotation, get_record,
    get_registered_paginated_internal, get_role as storage_get_role,
    get_role_delay as storage_get_role_delay,
    get_role_holder_count as storage_get_role_holder_count, get_role_holders_internal,
    get_rotation_delay as storage_get_rotation_delay, get_stats as read_stats,
    get_verification_config, get_verified_count as storage_get_verified_count,
    get_verifier_allowlist, get_verify_limit, get_verify_limit as storage_get_verify_limit,
    get_version as storage_get_version, get_wasm_attestation, get_wasm_provenance, has_challenge,
    has_pending_rotation, has_record, is_active_verifier as storage_is_active_verifier,
    is_admin_caller, is_attestation_required, is_guardian, is_in_cooldown,
    is_paused as storage_is_paused, prune_expired_verifiers, push_audit_entry, remove_challenge,
    remove_from_index, remove_guardian as storage_remove_guardian, remove_pending_role,
    remove_pending_rotation, remove_record, remove_role as storage_remove_role,
    remove_verifier as storage_remove_verifier, remove_wasm_attestation,
    require_attestation_if_required, require_initialized, require_not_paused,
    require_role_not_expired, run_migration_steps, set_audit_config as storage_set_audit_config,
    set_challenge, set_cooldown as storage_set_cooldown, set_count, set_emergency_pause,
    set_emergency_pause_ts, set_ever_verified_count, set_guardian_address, set_last_action,
    set_last_event_ledger, set_last_upgrade, set_network_id, set_paused as set_paused_state,
    set_pending_reverify, set_pending_role, set_pending_rotation, set_record,
    set_role as storage_set_role, set_role_delay as storage_set_role_delay,
    set_rotation_delay as storage_set_rotation_delay, set_verified_count, set_verify_limit,
    set_verify_limit as storage_set_verify_limit, set_version, set_wasm_attestation,
    set_wasm_provenance, verifier_allowlist_active, verifier_slots_remaining,
    PendingRoleGrant as PendingRoleGrantRecord, ADMIN_KEY, DEFAULT_CHALLENGE_DELAY_SECS,
};

use crate::utils::{
    eq_ignore_ascii_case, is_valid_github_username, is_zero_address, MAX_USERNAME_LEN,
};

/// Valid revoke reason codes for `revoke_verification`.
///
/// These codes are emitted in `VerificationRevokedEvent.reason_code` and
/// validated on-chain so only known reasons are accepted.
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum RevokeReason {
    /// GitHub identity proof was found to be invalid or fabricated.
    IdentityFraud = 1,
    /// Account was compromised or the private key was leaked.
    CompromisedKey = 2,
    /// Regulatory or legal requirement to revoke verification.
    Regulatory = 3,
    /// Duplicate registration detected for the same GitHub identity.
    DuplicateRegistration = 4,
    /// Operator error during the verification process.
    OperatorError = 5,
    /// The contributor requested removal under GDPR or similar privacy law.
    GdprErasure = 6,
    /// Any other reason not covered by the specific codes above.
    Other = 99,
}

impl RevokeReason {
    /// Returns `true` if `code` is a valid `RevokeReason` discriminant.
    #[must_use]
    pub fn is_valid(code: u32) -> bool {
        matches!(code, 1 | 2 | 3 | 4 | 5 | 6 | 99)
    }
}

/// Version this WASM was built at. Instances whose stored version predates
/// version tracking fall back to this.
pub const CONTRACT_VERSION: Version = Version {
    major: 1,
    minor: 1,
    patch: 0,
};

/// Builds the [`EventDomain`] stamped onto every event this contract emits
/// (Issue #226).
///
/// The version is read from instance storage rather than [`CONTRACT_VERSION`]
/// so an instance deployed before version tracking, or one mid-upgrade, still
/// reports the version its state actually claims. `CONTRACT_VERSION` is the
/// fallback for instances that have no stored version at all — the same
/// resolution `get_version` uses, so the value in an event always matches what
/// a reader would see from a contract call.
fn event_domain(env: &Env) -> EventDomain {
    // Event construction and publication are in the same contract invocation.
    // If publication traps, Soroban rolls this cursor update back atomically.
    set_last_event_ledger(env);
    let version = storage_get_version(env).unwrap_or_else(|| CONTRACT_VERSION.to_tuple());
    EventDomain::new(env, version)
}

/// Shared tail of `verify` and `verify_with_oracle_proof`: flips `record` to
/// verified and updates counters, events, and the audit log. Authorization is
/// the caller's responsibility.
/// Rejects a record whose bound Stellar address is the zero/burn address.
///
/// Operator `verify` / `batch_verify` attest GitHub ownership off-chain; the
/// on-chain binding is the stored `stellar_address`. Events and counters must
/// never attach verification to a caller-supplied or zero address.
fn require_bound_record_address(
    env: &Env,
    record: &ContributorRecord,
) -> Result<(), ContractError> {
    if is_zero_address(env, &record.stellar_address) {
        return Err(ContractError::ZeroAddress);
    }
    Ok(())
}

fn mark_verified(
    env: &Env,
    github_username: String,
    mut record: ContributorRecord,
    actor: Address,
) -> Result<(), ContractError> {
    require_bound_record_address(env, &record)?;
    // Issue #218: an expired prior verification does not block a fresh
    // one — only a still-active verification does.
    let was_active =
        record.verified && !crate::storage::is_verification_expired(env, &github_username);
    if was_active {
        return Err(ContractError::AlreadyVerified);
    }

    let was_verified = record.verified;
    record.verified = true;
    set_record(env, &github_username, &record);
    let timestamp = env.ledger().timestamp();
    crate::storage::set_verified_at(env, &github_username, timestamp);

    // Only a genuine false→true transition is a new verification for
    // counting purposes — renewing an expired-but-never-revoked
    // verification does not touch `verified_count` a second time, since
    // it was never decremented when the previous grant expired (expiry
    // is lazy; see `is_verification_expired`).
    if !was_verified {
        set_verified_count(env, storage_get_verified_count(env).saturating_add(1));
        bump_ever_verified_count(env);
    }

    // Clear pending reverify flag upon successful verification
    clear_pending_reverify(env, &github_username);
    VerifiedEvent {
        github_username: github_username.clone(),
        stellar_address: record.stellar_address.clone(),
        timestamp,
        domain: event_domain(env),
    }
    .publish(env);

    push_audit_entry(
        env,
        AuditLogEntry::new(AuditEventType::UserVerified, timestamp, Some(actor))
            .with_username(github_username)
            .with_address(record.stellar_address),
    );

    Ok(())
}

#[contract]
pub struct TrustBridgeContract;

#[cfg(test)]
extern crate alloc;
#[cfg(test)]
extern crate std;

#[cfg(test)]
mod test;
