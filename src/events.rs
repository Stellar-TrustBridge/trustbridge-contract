use soroban_sdk::{contractevent, Address, BytesN, String, Symbol};

use crate::domain::EventDomain;

/// Emitted when a GitHub username is registered or re-registered to a Stellar address.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredEvent {
    #[topic]
    pub github_username: String,
    pub stellar_address: Address,
    pub timestamp: u64,
    pub sponsor: Option<Address>,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

/// Emitted when `set_bot_status` changes a record's `is_bot` flag (Issue #374).
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BotStatusChangedEvent {
    #[topic]
    pub github_username: String,
    pub is_bot: bool,
    /// Admin or registrant that changed the flag.
    pub actor: Address,
    pub timestamp: u64,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

/// Emitted when a registration is removed by the registrant or admin.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemovedEvent {
    #[topic]
    pub github_username: String,
    pub stellar_address: Address,
    pub timestamp: u64,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

/// Emitted when an admin or Verifier marks a contributor as verified.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedEvent {
    #[topic]
    pub github_username: String,
    pub stellar_address: Address,
    pub timestamp: u64,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

/// Emitted when an admin or Verifier revokes a contributor's verified status.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationRevokedEvent {
    #[topic]
    pub github_username: String,
    pub stellar_address: Address,
    pub timestamp: u64,
    /// Numeric reason code explaining why verification was revoked.
    pub reason_code: u32,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

/// Emitted when the contract WASM is upgraded via `upgrade`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradedEvent {
    #[topic]
    pub new_wasm_hash: BytesN<32>,
    pub version: (u32, u32, u32),
    pub timestamp: u64,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

/// Emitted when the contract is paused via `pause`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PausedEvent {
    #[topic]
    pub admin: Address,
    pub timestamp: u64,
    /// Numeric reason code from `PauseReason` explaining why the contract was paused.
    pub reason_code: u32,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

/// Emitted when the contract is unpaused via `unpause`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnpausedEvent {
    #[topic]
    pub admin: Address,
    pub timestamp: u64,
    /// Numeric reason code from `PauseReason` explaining why the contract was unpaused.
    pub reason_code: u32,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

/// Emitted when the contract enters emergency shutdown via `emergency_pause`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyPausedEvent {
    #[topic]
    pub admin: Address,
    pub timestamp: u64,
    /// Numeric reason code from `PauseReason` explaining the emergency pause.
    pub reason_code: u32,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

/// Emitted when the contract exits emergency shutdown via `clear_emergency_pause`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyClearedEvent {
    #[topic]
    pub admin: Address,
    pub timestamp: u64,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

/// Emitted when the guardian address is set or removed.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardianChangedEvent {
    #[topic]
    pub guardian: Option<Address>,
    pub admin: Address,
    pub timestamp: u64,
}

/// Emitted when a pending role grant is queued behind the timelock.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleGrantPendingEvent {
    #[topic]
    pub address: Address,
    pub role: u32,
    pub admin: Address,
    pub activate_at: u64,
    pub timestamp: u64,
}

/// Emitted when a queued role grant is cancelled before activation.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleGrantCancelledEvent {
    #[topic]
    pub address: Address,
    pub admin: Address,
    pub timestamp: u64,
}

/// Emitted when `rename` moves a registration between usernames.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenamedEvent {
    #[topic]
    pub old_username: String,
    #[topic]
    pub new_username: String,
    pub stellar_address: Address,
    pub verification_cleared: bool,
    pub timestamp: u64,
}

/// Emitted when an address rotation is requested.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RotationRequestedEvent {
    #[topic]
    pub github_username: String,
    pub current_address: Address,
    pub new_address: Address,
    pub executable_at: u64,
    pub timestamp: u64,
}

/// Emitted when a pending address rotation is executed.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RotationExecutedEvent {
    #[topic]
    pub github_username: String,
    pub old_address: Address,
    pub new_address: Address,
    pub timestamp: u64,
}

/// Emitted when a pending address rotation is cancelled.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RotationCancelledEvent {
    #[topic]
    pub github_username: String,
    pub cancelled_by: Address,
    pub timestamp: u64,
}

/// Emitted when a WASM hash is staged ahead of an upgrade.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WasmStagedEvent {
    #[topic]
    pub wasm_hash: BytesN<32>,
    pub staged_by: Address,
    pub timestamp: u64,
}

/// Emitted when an upgrade attestation is recorded.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeAttestedEvent {
    #[topic]
    pub wasm_hash: BytesN<32>,
    pub expires_at: u64,
    pub timestamp: u64,
}

/// Emitted when a pending attestation is withdrawn.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttestationClearedEvent {
    #[topic]
    pub wasm_hash: BytesN<32>,
    pub expires_at: u64,
    pub timestamp: u64,
}

/// Emitted when a role is granted to an address via `set_role`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleGrantedEvent {
    #[topic]
    pub address: Address,
    /// Numeric discriminant of the [`Role`][crate::storage::Role] granted.
    pub role: u32,
    pub admin: Address,
    pub timestamp: u64,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

/// Emitted when a role is revoked from an address via `remove_role`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleRevokedEvent {
    #[topic]
    pub address: Address,
    pub admin: Address,
    pub timestamp: u64,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

/// Emitted when a large `batch_remove` is proposed for dual-control execution
/// (Issue #219).
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchRemoveProposedEvent {
    #[topic]
    pub proposed_by: Address,
    /// Number of usernames in the proposed batch.
    pub count: u32,
    pub timestamp: u64,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

/// Emitted when a pending large `batch_remove` proposal is executed by a
/// second, distinct address (Issue #219).
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchRemoveExecutedEvent {
    #[topic]
    pub executed_by: Address,
    pub proposed_by: Address,
    /// Number of usernames in the executed batch.
    pub count: u32,
    /// Number of usernames actually removed (partial-success semantics
    /// match `batch_remove`'s own `BatchSummary`).
    pub successful: u32,
    pub timestamp: u64,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

/// Emitted when a pending large `batch_remove` proposal is cancelled without
/// executing (Issue #219).
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchRemoveCancelledEvent {
    #[topic]
    pub cancelled_by: Address,
    pub proposed_by: Address,
    pub timestamp: u64,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

/// Emitted when an admin starts a challenge on a squatted username (Issue #214).
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChallengeStartedEvent {
    #[topic]
    pub github_username: String,
    pub challenged_by: Address,
    pub resolve_after: u64,
    pub timestamp: u64,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

/// Emitted when an admin cancels a pending challenge (Issue #214).
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChallengeCancelledEvent {
    #[topic]
    pub github_username: String,
    pub cancelled_by: Address,
    pub timestamp: u64,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

/// Emitted when an admin completes a challenge and removes the squatted
/// registration (Issue #214).
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChallengeCompletedEvent {
    #[topic]
    pub github_username: String,
    pub completed_by: Address,
    pub timestamp: u64,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedWasmClearedEvent {
    #[topic]
    pub wasm_hash: BytesN<32>,
    pub cleared_by: Address,
    pub timestamp: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeProposedEvent {
    #[topic]
    pub proposal_id: u32,
    pub proposed_by: Address,
    pub wasm_hash: BytesN<32>,
    pub executable_at: u64,
    pub timestamp: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeApprovedEvent {
    #[topic]
    pub proposal_id: u32,
    pub approved_by: Address,
    pub approval_count: u32,
    pub timestamp: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeProposalExecutedEvent {
    #[topic]
    pub proposal_id: u32,
    pub executed_by: Address,
    pub wasm_hash: BytesN<32>,
    pub approval_count: u32,
    pub timestamp: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeProposalCancelledEvent {
    #[topic]
    pub proposal_id: u32,
    pub cancelled_by: Address,
    pub wasm_hash: BytesN<32>,
    pub timestamp: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationConfiguredEvent {
    #[topic]
    pub admin: Address,
    pub attestation: Symbol,
    pub expires_in: u64,
    pub threshold: u32,
    pub timestamp: u64,
}

#[cfg(test)]
mod test {
    use crate::{TrustBridgeContract, TrustBridgeContractClient};
}
