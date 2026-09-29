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
///
/// Carries the same [`EventDomain`] tagging as every other lifecycle event so
/// indexers that filter by domain observe emergency transitions too (Issue #408).
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

/// Emitted when the contract exits emergency shutdown via `emergency_unpause`.
///
/// Carries the same [`EventDomain`] tagging as every other lifecycle event so
/// indexers that filter by domain observe emergency transitions too (Issue #408).
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyUnpausedEvent {
    #[topic]
    pub admin: Address,
    pub timestamp: u64,
    /// Numeric reason code from `PauseReason` explaining the emergency unpause.
    pub reason_code: u32,
    /// Deployment that emitted this event — contract id, network, and
    /// contract version. See [`EventDomain`] for why indexers need it.
    pub domain: EventDomain,
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
    pub proposal_id: BytesN<32>,
    pub proposed_by: Address,
    pub wasm_hash: BytesN<32>,
    pub timestamp: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeApprovedEvent {
    #[topic]
    pub proposal_id: BytesN<32>,
    pub approved_by: Address,
    pub timestamp: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeProposalExecutedEvent {
    #[topic]
    pub proposal_id: BytesN<32>,
    pub executed_by: Address,
    pub timestamp: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeProposalCancelledEvent {
    #[topic]
    pub proposal_id: BytesN<32>,
    pub cancelled_by: Address,
    pub timestamp: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationConfiguredEvent {
    #[topic]
    pub admin: Address,
    pub attestation: String,
    pub expires_in: u64,
    pub threshold: u32,
    pub timestamp: u64,
}

#[cfg(test)]
mod test {
    use crate::{TrustBridgeContract, TrustBridgeContractClient};
}
