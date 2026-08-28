//! Domain separation for events and records (Issues #226, #231).
//!
//! # Why this exists
//!
//! Two related problems share one answer.
//!
//! **Replay collisions across deployments (#226).** Events carried no marker
//! saying *which* contract instance on *which* network produced them. An
//! indexer that reconciles by `(github_username, event_type, timestamp)` — the
//! only fields it had — cannot tell a genuine re-registration from the same
//! event re-read out of a redeployed contract's history. Redeploy the contract
//! and replay, and every historical registration either looks like a duplicate
//! of a live record or like a brand-new user, depending on which way the
//! indexer resolves the tie. Neither is right.
//!
//! **Cross-network record mixing (#231).** A Stellar G-address is
//! network-agnostic: the same keypair is valid on Futurenet, testnet, and the
//! public network. Nothing about a stored `ContributorRecord` says which
//! network its registration was meant for, so a consumer holding a record has
//! to infer the network from whichever RPC URL it happened to dial. Get that
//! wrong and a payout is computed against the wrong ledger.
//!
//! Both are answered by naming the deployment: contract id, network, and
//! contract version, together identifying exactly one instance of exactly one
//! build on exactly one network.
//!
//! # Where the network id comes from
//!
//! [`Env::ledger().network_id()`] is the SHA-256 of the network passphrase, so
//! the contract can determine its own network with no new parameters and
//! nothing for a deployer to get wrong. That is deliberate: an operator-supplied
//! network tag is exactly the sort of field that gets copy-pasted from a
//! testnet runbook into a mainnet deploy.
//!
//! `initialize` records the network id it saw. Every later read compares the
//! live network against that record — see
//! [`require_matching_network`][crate::storage::require_matching_network] — so
//! state restored onto a different network fails closed instead of silently
//! serving records that were never meant for it.

use soroban_sdk::{contracttype, Address, BytesN, Env};

/// Schema version for the event envelope itself.
///
/// Distinct from the contract version: this changes only when the *shape* of
/// [`EventDomain`] changes, so an indexer can branch on the envelope without
/// having to track every contract release. Starts at 1 — the first version
/// that carries a domain at all.
pub const EVENT_DOMAIN_VERSION: u32 = 1;

/// Identity of the deployment that produced an event.
///
/// Attached to every event this contract emits. `(contract_id, network_id)` is
/// the deduplication key an indexer should use alongside whatever it already
/// keys on: it is stable for the life of a deployment and differs across
/// redeploys and networks, which is precisely the distinction that was missing.
///
/// `contract_version` is included so a replay can be attributed to the build
/// that emitted it, which matters when an upgrade changes what an event means.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventDomain {
    /// Address of the emitting contract instance.
    pub contract_id: Address,
    /// SHA-256 of the network passphrase — see [`Env::ledger().network_id()`].
    pub network_id: BytesN<32>,
    /// Contract semantic version at emit time, as `(major, minor, patch)`.
    pub contract_version: (u32, u32, u32),
    /// Schema version of this envelope. See [`EVENT_DOMAIN_VERSION`].
    pub domain_version: u32,
}

impl EventDomain {
    /// Builds the domain for the currently executing contract.
    ///
    /// Reads live host state rather than stored values so the domain always
    /// describes the invocation that is actually running. In particular the
    /// version comes from the caller, which passes the value it just read or
    /// wrote — during `upgrade` those differ, and the event must carry the
    /// version being recorded, not a stale instance read.
    #[must_use]
    pub fn new(env: &Env, contract_version: (u32, u32, u32)) -> Self {
        EventDomain {
            contract_id: env.current_contract_address(),
            network_id: env.ledger().network_id(),
            contract_version,
            domain_version: EVENT_DOMAIN_VERSION,
        }
    }
}
