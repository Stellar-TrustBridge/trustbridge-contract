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

#[cfg(test)]
mod test {
    use crate::{TrustBridgeContract, TrustBridgeContractClient};
    use crate::domain::EventDomain;
    use crate::utils::generate_event_id;
    use soroban_sdk::{testutils::Address as _, Address, Bytes, Env, String, Symbol};

    /// Builds a deterministic [`EventDomain`] for the hashing-input corpus.
    fn domain(env: &Env, contract_id: &[u8; 32], network: &str, version: (u32, u32, u32)) -> EventDomain {
        EventDomain {
            contract_id: Bytes::from_slice(env, contract_id),
            network: String::from_str(env, network),
            version,
        }
    }

    /// Documents the exact hashing inputs consumed by `generate_event_id`:
    /// the event `domain` (contract id, network, version) plus the event
    /// `payload` bytes. The id is a pure function of these two inputs, so
    /// identical inputs must always yield identical ids (replayable
    /// fixtures) while distinct domain/payload pairs must not collide.
    #[test]
    fn generate_event_id_is_deterministic() {
        let env = Env::default();
        let d = domain(&env, &[7u8; 32], "testnet", (1, 0, 0));
        let payload = Bytes::from_slice(&env, b"registered:alice");

        let first = generate_event_id(&env, &d, &payload);
        let second = generate_event_id(&env, &d, &payload);

        assert_eq!(first, second, "same inputs must produce the same event id");
    }

    /// Fixed corpus of distinct domain/payload pairs; every generated id
    /// must be unique so indexers can key events without collisions.
    #[test]
    fn generate_event_id_does_not_collide() {
        let env = Env::default();

        let corpus: [(EventDomain, &[u8]); 4] = [
            (domain(&env, &[1u8; 32], "testnet", (1, 0, 0)), b"registered:alice"),
            (domain(&env, &[1u8; 32], "testnet", (1, 0, 0)), b"registered:bob"),
            (domain(&env, &[2u8; 32], "testnet", (1, 0, 0)), b"registered:alice"),
            (domain(&env, &[1u8; 32], "mainnet", (1, 0, 0)), b"registered:alice"),
        ];

        let mut ids: Vec<Bytes> = Vec::new();
        for (d, payload) in corpus.iter() {
            let payload = Bytes::from_slice(&env, payload);
            let id = generate_event_id(&env, d, &payload);
            assert!(
                !ids.contains(&id),
                "distinct domain/payload pairs must not collide"
            );
            ids.push(id);
        }
    }

    #[test]
    fn registered_event_carries_domain() {
        let env = Env::default();
        let contract_id = env.register(TrustBridgeContract, ());
        let client = TrustBridgeContractClient::new(&env, &contract_id);
        let _ = client;
        let _ = Address::generate(&env);
        let _ = Symbol::new(&env, "registered");
    }
}
