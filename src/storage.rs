//! Instance vs. persistent storage layout, TTL constants, and per-key rent
//! behavior are documented in `docs/STORAGE_RENT.md` — read that before
//! changing `TTL_THRESHOLD` / `TTL_BUMP` or adding a new persistent key.

use soroban_sdk::{symbol_short, xdr::ToXdr, Address, Bytes, BytesN, Env, String, Symbol, Vec};

use crate::ContractError;

/// Canonical storage key for a GitHub username (Issue #194).
///
/// Every persistent key namespaced by a username — `reg`, `chllng`,
/// `pend_rev`, `lastact`, `pendrot` — is built from this canonical form, so
/// `Alice`, `ALICE`, and `alice` all resolve to the same underlying entry. See
/// `docs/SECURITY.md#username-case-folding` for the fold rule (ASCII lower)
/// and the squatting scenario this closes.
fn canon(env: &Env, username: &String) -> String {
    crate::utils::canonicalize_username(env, username)
}

pub const VER_CFG_KEY: Symbol = symbol_short!("vrfy_cfg");

// ── Storage keys ────────────────────────────────────────────────────────────

pub const REG_KEY: Symbol = symbol_short!("reg");
/// Storage key for the admin address. `initialize` is the **only** place
/// that writes this key, gated by `AlreadyInitialized` so it can run once.
/// No other public entry point mutates it — the admin is immutable after
/// init; rotation requires redeploying a new instance. See
/// `docs/SECURITY.md#admin-key-management` (Issue #97).
pub const ADMIN_KEY: Symbol = symbol_short!("admin");
pub const COUNT_KEY: Symbol = symbol_short!("count");
pub const VCOUNT_KEY: Symbol = symbol_short!("vcount");
/// Monotonic count of verifications ever granted (Issue #229). Unlike
/// `VCOUNT_KEY` this never decreases, so revoking does not erase the fact that
/// a contributor was verified at some point.
pub const EVER_VCOUNT_KEY: Symbol = symbol_short!("evcount");
pub const INDEX_KEY: Symbol = symbol_short!("idx");
pub const ORG_INDEX_KEY: Symbol = symbol_short!("orgidx");
pub const TEAM_INDEX_KEY: Symbol = symbol_short!("tmidx");

/// Distinguishes personal accounts from organization and team entries.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[soroban_sdk::contracttype]
pub enum EntityType {
    Personal = 0,
    Org = 1,
    Team = 2,
}
pub const PAUSED_KEY: Symbol = symbol_short!("pause");
/// Last `PauseReason` recorded by `pause` / `unpause`.
pub const PAUSE_RSN_KEY: Symbol = symbol_short!("pause_rsn");
pub const COOLDOWN_KEY: Symbol = symbol_short!("cdown");
/// Seconds a requested address rotation must wait before it can execute
/// (Issue #234). 0 disables the delay, matching the cooldown convention.
pub const ROT_DELAY_KEY: Symbol = symbol_short!("rotdelay");
/// Key prefix for a username's pending address rotation (Issue #234).
pub const PENDING_ROT_KEY: Symbol = symbol_short!("pendrot");
// Pending reverify flag per username
pub const PENDING_REVERIFY_KEY: Symbol = symbol_short!("pend_rev");
// Emergency pause flag and timestamp — wired as guardian circuit breaker (Issue #196)
pub const EMERGENCY_PAUSE_KEY: Symbol = symbol_short!("emrg_ps");
pub const EMERGENCY_PAUSE_TS_KEY: Symbol = symbol_short!("emerg_ts");
/// Storage key for the designated guardian address (Issue #196).
/// The guardian may trip the emergency pause but may NOT upgrade the contract.
pub const GUARDIAN_KEY: Symbol = symbol_short!("guardian");
pub const LAST_UPG_KEY: Symbol = symbol_short!("lastupg");
pub const VER_KEY: Symbol = symbol_short!("ver");
/// Key for the network id recorded at `initialize` (Issue #231).
///
/// Holds `env.ledger().network_id()` — the SHA-256 of the network passphrase —
/// as observed when the instance was initialized. Instances initialized before
/// this key existed have no value, which is treated as "untagged" and allowed
/// through; see [`require_matching_network`].
pub const NETWORK_KEY: Symbol = symbol_short!("network");

pub const ROLE_KEY: Symbol = symbol_short!("role");

/// Key for the enumerable index of addresses that currently hold a role
/// (Issue #228). Maintained by [`set_role`] and [`remove_role`] so it can
/// never drift from the per-address `ROLE_KEY` entries.
pub const ROLE_IDX_KEY: Symbol = symbol_short!("role_idx");

/// Maximum entries returned by one `get_role_holders` page (Issue #228).
///
/// Role holders are privileged addresses, so the population is small by
/// design; this exists to bound the response, not to paginate a large set.
pub const MAX_ROLE_PAGE_LIMIT: u32 = 50;

/// Key prefix for chunked username index entries.
pub const CHUNK_KEY: Symbol = symbol_short!("chunk");
pub const CHUNK_CNT_KEY: Symbol = symbol_short!("chkcnt");
/// Monotonic counter bumped every time the flat index's existing positions
/// shift — i.e. on every removal (Issue #215). An opaque pagination cursor
/// embeds the generation at issue time; `decode_cursor` rejects a cursor
/// whose generation no longer matches, rather than silently resuming at a
/// drifted offset.
pub const INDEX_GEN_KEY: Symbol = symbol_short!("idx_gen");
pub const LAST_ACT_KEY: Symbol = symbol_short!("lastact");
/// Key for the WASM provenance record (Wave #24).
pub const PROV_KEY: Symbol = symbol_short!("prov");
/// Key for the pending upgrade attestation (Wave #24).
pub const ATTEST_KEY: Symbol = symbol_short!("attest");
/// Key for audit log entries list.
pub const AUDIT_LOG_KEY: Symbol = symbol_short!("adt_log");
/// Key for audit stats.
pub const AUDIT_STATS_KEY: Symbol = symbol_short!("adt_stat");
/// Key prefix for per-username challenge records (Issue #214).
pub const CHALLENGE_KEY: Symbol = symbol_short!("chllng");
/// Default challenge delay in seconds: 48 hours gives the registrant time to
/// prove GitHub ownership off-chain before the name is freed.
pub const DEFAULT_CHALLENGE_DELAY_SECS: u64 = 172_800; // 48 hours

/// Key for the pause reason code (Issue #211).
pub const PAUSE_REASON_KEY: Symbol = symbol_short!("p_reason");

/// Key for the reserved username set (Issue #213).
pub const RESERVED_KEY: Symbol = symbol_short!("reserved");

/// Maximum entries in the reserved username list (Issue #213).
pub const MAX_RESERVED: u32 = 200;

/// Hard cap on the number of fallback addresses per registration (Issue #238).
/// Prevents unbounded storage growth from a single registration.
pub const MAX_FALLBACK_ADDRESSES: u32 = 5;

/// Key for the version stored at `storage::get_version` / `set_version`.
/// Aliased as VERSION_KEY for callers that use that name.
pub const VERSION_KEY: Symbol = VER_KEY;

/// Key for a pending admin transfer proposal (Issue #195).
pub const ADMIN_TRANSFER_KEY: Symbol = symbol_short!("adm_xfr");

/// Key for whether WASM attestation is required before upgrade (Issue #198).
pub const ATTEST_REQUIRED_KEY: Symbol = symbol_short!("att_req");

/// Key prefix for a role assignment's expiry timestamp (Issue #221).
///
/// Absent (no entry) means the role granted at `ROLE_KEY` for that address
/// never expires — the pre-existing behavior. Present means the grant is only
/// valid while `env.ledger().timestamp() < expires_at`.
pub const ROLE_EXPIRY_KEY: Symbol = symbol_short!("role_exp");

/// Key prefix for the ledger timestamp a username was last `verify()`'d
/// (Issue #218).
pub const VERIFIED_AT_KEY: Symbol = symbol_short!("vfy_at");

/// Key for the `batch_remove` dual-control size threshold (Issue #219).
/// `0` (the default) disables dual control — every batch is executed
/// directly by a single admin call, matching pre-#219 behavior.
pub const BATCH_REMOVE_THRESHOLD_KEY: Symbol = symbol_short!("brm_thr");

/// Key for the single pending large-`batch_remove` proposal (Issue #219).
/// Only one proposal may be live at a time.
pub const PENDING_BATCH_REMOVE_KEY: Symbol = symbol_short!("brm_pend");

/// Seconds a `batch_remove` dual-control proposal stays valid before
/// `execute_batch_remove` treats it as gone (Issue #219). Prevents a stale
/// proposal from being executed long after the operator context that
/// justified it has passed.
pub const BATCH_REMOVE_PROPOSAL_TTL_SECS: u64 = 86_400; // 24 hours

// ── Pagination constants ─────────────────────────────────────────────────────

pub const DEFAULT_PAGE_LIMIT: u32 = 20;
pub const MAX_PAGE_LIMIT: u32 = 100;

/// Maximum number of usernames per chunked index entry.
pub const CHUNK_SIZE: u32 = 50;

// ── TTL constants (ledger-based, ~5s/ledger) ────────────────────────────────
//
// Stellar closes a ledger roughly every 5 seconds, so ~17,280 ledgers is a day.

/// Ledgers per day at the ~5s close time, used to express the policy in days.
pub const LEDGERS_PER_DAY: u32 = 17_280;

/// Persistent entries are bumped when their remaining TTL drops below this
/// (~30 days). `extend_ttl` is a no-op when the remaining TTL already exceeds
/// the threshold, so this is what keeps a hot record from paying the
/// extension cost on every single read.
pub const TTL_THRESHOLD: u32 = LEDGERS_PER_DAY * 30;

/// Extend to this many ledgers from the current one (~90 days). Comfortably
/// inside the network's maximum persistent TTL, so an extension is never
/// rejected for overshooting the cap.
pub const TTL_BUMP: u32 = LEDGERS_PER_DAY * 90;

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
#[repr(u32)]
pub enum Role {
    Admin = 1,
    Upgrader = 2,
    /// May call `verify` but not `revoke_verification`.
    Verifier = 3,
    /// May call `revoke_verification` but not `verify`.
    /// Separates the power to grant verification from the power to withdraw it,
    /// so a compromised Verifier key cannot silently undo payout eligibility.
    Revoker = 4,
}

/// Typed reason code for `pause`, `unpause`, and `set_paused` (Issue #211).
///
/// Stored on-chain alongside the pause flag so incident reviewers can
/// distinguish a maintenance pause from a security freeze without replaying
/// event history. All mutation entry points that flip the pause flag require
/// a valid `PauseReason`; unknown codes fail with
/// [`ContractError::InvalidPauseReason`].
///
/// | Code | Name | When to use |
/// |------|------|-------------|
/// | 1 | `Maintenance` | Planned upgrade window or admin maintenance |
/// | 2 | `SecurityIncident` | Freeze after a detected exploit or suspicious activity |
/// | 3 | `RegulatoryHold` | Compliance or legal hold requirement |
/// | 4 | `Unpause` | Resuming normal operation (used with `unpause`) |
/// | 99 | `Other` | Any reason not covered above |
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
#[repr(u32)]
pub enum PauseReason {
    Maintenance = 1,
    SecurityIncident = 2,
    RegulatoryHold = 3,
    Unpause = 4,
    Other = 99,
}

impl PauseReason {
    /// Returns `true` if `code` maps to a known `PauseReason` discriminant.
    #[must_use]
    pub fn is_valid(code: u32) -> bool {
        matches!(code, 1 | 2 | 3 | 4 | 99)
    }

    /// Converts a raw u32 to the corresponding `PauseReason`, or `None` for
    /// unrecognized codes.
    #[must_use]
    pub fn from_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(PauseReason::Maintenance),
            2 => Some(PauseReason::SecurityIncident),
            3 => Some(PauseReason::RegulatoryHold),
            4 => Some(PauseReason::Unpause),
            99 => Some(PauseReason::Other),
            _ => None,
        }
    }
}

/// An on-chain record for a registered contributor.
///
/// Stored under `(Symbol("reg"), github_username)` in persistent storage.
/// TTL is extended on every read and write; use `extend_registry_ttl` to
/// refresh cold entries before they are archived.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct ContributorRecord {
    /// The Stellar G-address that owns this registration (identity address).
    pub stellar_address: Address,
    /// The Stellar address where payouts should be sent. Defaults to
    /// `stellar_address` if not explicitly set, allowing contributors to
    /// separate their identity from their payment destination.
    pub payout_address: Address,
    /// Ledger timestamp when this record was last written.
    ///
    /// Stored as `u32` instead of `u64` to save 4 bytes per record. Soroban
    /// ledger timestamps (Unix seconds) fit in u32 until ~2106 — well beyond
    /// the expected lifetime of any TrustBridge contract instance. The cast
    /// from `env.ledger().timestamp()` (`u64`) to `u32` is a deliberate
    /// truncation that will not wrap in practice.
    pub registered_at: u32,
    /// Whether the contributor has been verified by an admin or Verifier.
    pub verified: bool,
    pub is_bot: bool,
}

/// Provenance of the currently deployed WASM executable (Wave #24).
///
/// `upgrade` previously left no queryable trace of what it did — it wrote a
/// bare timestamp to `LAST_UPG_KEY` and published an event. Events are not
/// contract state: an auditor asking "what is deployed right now, and what did
/// it replace?" had to reconstruct the answer by replaying the whole event
/// history, and could not do it from a contract call at all.
///
/// This is the answer as a single readable record. `previous_wasm_hash` is what
/// makes it a chain rather than a snapshot: each record names its predecessor,
/// so the lineage can be walked backwards through historical `UpgradedEvent`s
/// even though only the head is stored.
/// Semantic version triple used by `WasmProvenance`.
///
/// Stored as a named struct so that `#[soroban_sdk::contracttype]` can
/// derive the XDR serialization — bare `(u32, u32, u32)` tuples are not
/// supported inside `Option` by the macro.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct VersionTriple {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct WasmProvenance {
    /// Hash of the WASM currently executing.
    pub wasm_hash: BytesN<32>,
    /// Hash this one replaced. `None` for the first upgrade after deployment.
    pub previous_wasm_hash: Option<BytesN<32>>,
    /// Address that authorised the upgrade.
    pub upgraded_by: Address,
    /// Ledger timestamp the upgrade was applied.
    pub upgraded_at: u64,
    /// Contract version recorded at upgrade time. Empty vec == unset.
    pub version: Vec<u32>,
    /// Whether the hash had been attested before it was applied.
    pub attested: bool,
}

/// An admin's advance declaration of the WASM hash they intend to deploy.
///
/// Optional two-step upgrade. When an attestation is live, `upgrade` will only
/// accept the hash it names — so a compromised admin key cannot swap in a
/// different binary at the moment of the upgrade without first publishing that
/// intent, on-chain, ahead of time.
///
/// The expiry is the point: an attestation that never lapsed would be a
/// standing authorisation for that hash, which is strictly worse than none.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct WasmAttestation {
    /// Hash the admin has declared they intend to deploy.
    pub wasm_hash: BytesN<32>,
    /// Ledger timestamp after which this attestation is no longer valid.
    pub expires_at: u64,
    /// Address that published the attestation.
    pub attested_by: Address,
    /// Ledger timestamp the attestation was published.
    pub attested_at: u64,
}

/// A pending admin-transfer proposal (Issue #195).
///
/// Created by `propose_admin_transfer` and consumed by `execute_admin_transfer`
/// after the mandatory delay elapses. `cancel_admin_transfer` removes the
/// pending record at any time before execution.
///
/// Only one proposal may be live at a time. A second call to
/// `propose_admin_transfer` while one is pending overwrites it, which is
/// intentional: the admin may correct a mistaken address during the delay
/// window.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct AdminTransferProposal {
    /// The candidate that will become admin after the delay.
    pub new_admin: Address,
    /// The current admin that proposed the transfer.
    pub proposed_by: Address,
    /// Ledger timestamp when `propose_admin_transfer` was called.
    pub proposed_at: u64,
    /// Earliest ledger timestamp at which `execute_admin_transfer` may run.
    pub executable_at: u64,
}

/// An address rotation that has been requested but not yet executed (Issue #234).
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct PendingRotation {
    /// The address the registration will move to once executed.
    pub new_address: Address,
    /// Ledger timestamp the rotation was requested.
    pub requested_at: u64,
    /// Ledger timestamp from which the rotation may be executed.
    pub executable_at: u64,
}

/// Existence proof for a single record, shaped for light clients (Issue #230).
///
/// Lets an indexer or the GitHub action confirm one registration without
/// paging the whole registry, and carries what it needs to fetch or revive the
/// underlying ledger entry itself.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct RecordProof {
    /// Whether a record is currently stored for this username.
    pub exists: bool,
    /// The record's verified flag. Always `false` when `exists` is `false`.
    pub verified: bool,
    /// Ledger timestamp the record was last written, or 0 when absent.
    pub registered_at: u32,
    /// Ledger sequence this proof was taken at.
    pub as_of_ledger: u32,
    /// Remaining-TTL threshold below which the entry is bumped, in ledgers.
    pub ttl_threshold_ledgers: u32,
    /// How far ahead of the current ledger a bump extends the entry.
    pub ttl_bump_ledgers: u32,
    /// Symbol half of the record's storage key. The full key is
    /// `(key_prefix, github_username)` — see `docs/STORAGE_RENT.md`.
    pub key_prefix: Symbol,
}

/// Aggregate registry statistics returned by `get_stats`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct Stats {
    /// Total number of registered contributors.
    pub total: u32,
    /// Number of contributors **currently** verified. Decreases on revoke.
    pub verified: u32,
    /// Number of verifications ever granted, including any later revoked
    /// (Issue #229). Monotonic: this never decreases.
    pub ever_verified: u32,
}

/// A single page of registry records returned by paginated export functions.
///
/// `next_cursor` is `None` when this is the last page. Pass it as `cursor` to
/// the next call to advance the page. `has_more` mirrors `next_cursor.is_some()`
/// for clients that prefer a boolean sentinel.
///
/// `next_cursor` is an **opaque** token (Issue #215): callers must not
/// construct one themselves or interpret its bytes — pass back exactly what
/// the contract returned. It embeds the index generation at issue time, so a
/// cursor becomes invalid (`ContractError::InvalidCursor`) rather than
/// silently skipping or duplicating records if a username was removed from
/// the registry after the cursor was issued. See `docs/DASHBOARD_SYNC.md`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct ExportPage {
    /// Records in this page: `(github_username, ContributorRecord)` pairs.
    pub records: Vec<(String, ContributorRecord)>,
    /// Opaque cursor to pass to the next call, or `None` if this is the last
    /// page. See the struct docs — never construct or decode this yourself.
    pub next_cursor: Option<BytesN<8>>,
    /// Total number of records in the registry at query time.
    pub total: u32,
    /// Merkle root over `records`, in page order (Issue #216). All-zero for
    /// an empty page. See `crate::merkle` for the leaf encoding and tree
    /// construction, so off-chain tooling can build an inclusion proof
    /// against this root without re-fetching the whole registry.
    pub merkle_root: BytesN<32>,
    /// `true` if there are more records after this page.
    pub has_more: bool,
}

/// A signed export attestation binding one [`ExportPage`] to a deterministic
/// digest, the contract's schema version, and the ledger it was read at
/// (Issue #223).
///
/// Intended for air-gapped audits: an auditor who receives the JSON produced
/// by `scripts/export_registry.sh` alongside this struct can recompute the
/// same digest offline from the raw page contents and compare it byte for
/// byte, without ever reconnecting to the network — the digest is the only
/// thing they need to trust the export matches what the contract actually
/// held at `ledger`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct ExportAttestation {
    /// The page this attestation covers — identical to what
    /// `get_registered_paginated` would return for the same `cursor`/`limit`.
    pub page: ExportPage,
    /// SHA-256 digest over `page`'s XDR encoding. See
    /// [`build_export_digest`] for exactly what is hashed.
    pub digest: BytesN<32>,
    /// Contract schema version `(major, minor, patch)` as a flat `Vec<u32>`,
    /// read the same way `get_version` resolves it.
    pub version: Vec<u32>,
    /// Ledger sequence the page and digest were computed at.
    pub ledger: u32,
}

/// On-chain health snapshot returned by `get_health` (Issue #210).
///
/// All fields are read from instance storage in a single contract call, so
/// dashboards and CI probes get a coherent view without five separate RPC
/// requests. The function is read-only, requires no auth, and works while
/// the contract is paused.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct HealthSnapshot {
    /// Whether the contract is currently paused.
    pub paused: bool,
    /// Schema version tuple `(major, minor, patch)` as a flat `Vec<u32>`.
    pub version: Vec<u32>,
    /// Total registered contributor count.
    pub total: u32,
    /// Verified contributor count.
    pub verified: u32,
    /// Configured WASM upgrade cooldown in seconds (0 = no cooldown).
    pub cooldown_secs: u64,
    /// Seconds remaining until the upgrade cooldown expires, or 0 if not
    /// in cooldown or no cooldown is configured.
    pub cooldown_remaining_secs: u64,
    /// Whether a non-expired upgrade attestation is currently live.
    pub attestation_present: bool,
}

/// A pending squatter-challenge record stored per username (Issue #214).
///
/// Admin starts a challenge on a registered name. After `resolve_after` the
/// admin may complete the challenge and remove the registration. Until then
/// the username is locked: re-registration is blocked and `remove` (by the
/// registrant) is still allowed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct ChallengeRecord {
    /// Address of the admin that started the challenge.
    pub challenged_by: Address,
    /// Ledger timestamp when the challenge was created.
    pub started_at: u64,
    /// Ledger timestamp before which the challenge cannot be completed.
    pub resolve_after: u64,
}

// ── Challenge storage helpers ─────────────────────────────────────────────────

pub fn get_challenge(env: &Env, github_username: &String) -> Option<ChallengeRecord> {
    env.storage()
        .persistent()
        .get(&(CHALLENGE_KEY, canon(env, github_username)))
}

pub fn set_challenge(env: &Env, github_username: &String, record: &ChallengeRecord) {
    let key = (CHALLENGE_KEY, canon(env, github_username));
    env.storage().persistent().set(&key, record);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
}

pub fn remove_challenge(env: &Env, github_username: &String) {
    env.storage()
        .persistent()
        .remove(&(CHALLENGE_KEY, canon(env, github_username)));
}

pub fn has_challenge(env: &Env, github_username: &String) -> bool {
    env.storage()
        .persistent()
        .has(&(CHALLENGE_KEY, canon(env, github_username)))
}

/// Network id recorded at `initialize` (Issue #231), or `None` for an
/// instance initialized before network tagging existed.
pub fn get_network_id(env: &Env) -> Option<BytesN<32>> {
    env.storage().instance().get(&NETWORK_KEY)
}

/// Records `network_id` as the instance's tag. Overwrites any existing value —
/// callers are responsible for only calling this when that is intended
/// (`initialize`, and `adopt_network_tag` for a previously-untagged instance).
pub fn set_network_id(env: &Env, network_id: &BytesN<32>) {
    env.storage().instance().set(&NETWORK_KEY, network_id);
}

/// Fails with [`ContractError::NetworkMismatch`] if a network id was recorded
/// at `initialize` and it differs from the network this call is executing on.
///
/// An instance with no recorded tag (deployed before Issue #231) is treated as
/// untagged and allowed through — there is nothing to compare against.
pub fn require_matching_network(env: &Env) -> Result<(), ContractError> {
    match get_network_id(env) {
        Some(recorded) if recorded != env.ledger().network_id() => {
            Err(ContractError::NetworkMismatch)
        }
        _ => Ok(()),
    }
}

/// Fails unless the contract is initialized **and** running on the network it
/// was initialized on.
///
/// The network check rides along here rather than at each entry point because
/// this is the one call every gated function already makes — putting it here
/// means a new entry point cannot forget it. See
/// [`require_matching_network`] for the policy and its migration behaviour.
///
/// # Errors
///
/// - [`ContractError::NotInitialized`] if `initialize` has not been called.
/// - [`ContractError::NetworkMismatch`] if the recorded network id differs from
///   the executing one.
pub fn require_initialized(env: &Env) -> Result<(), ContractError> {
    if !env.storage().instance().has(&ADMIN_KEY) {
        return Err(ContractError::NotInitialized);
    }
    require_matching_network(env)
}

pub fn get_admin(env: &Env) -> Result<Address, ContractError> {
    require_initialized(env)?;
    env.storage()
        .instance()
        .get(&ADMIN_KEY)
        .ok_or(ContractError::NotInitialized)
}

pub fn get_record(env: &Env, github_username: &String) -> Option<ContributorRecord> {
    let key = (REG_KEY, canon(env, github_username));
    let record: Option<ContributorRecord> = env.storage().persistent().get(&key);
    if record.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
    }
    record
}

pub fn set_record(env: &Env, github_username: &String, record: &ContributorRecord) {
    let key = (REG_KEY, canon(env, github_username));
    env.storage().persistent().set(&key, record);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
}

/// Extends a single record's TTL without deserialising it (Wave #7).
///
/// `get_record` also extends as a side effect of reading, but it pays to decode
/// the `ContributorRecord` first. A keeper bumping thousands of entries does not
/// want the value, only the extension — this skips that cost.
///
/// Returns whether the entry existed. A missing entry is not an error: the
/// keeper's list is built off-chain and can lag behind removals.
pub fn extend_record_ttl(env: &Env, github_username: &String) -> bool {
    let key = (REG_KEY, canon(env, github_username));
    if !env.storage().persistent().has(&key) {
        return false;
    }
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
    true
}

pub fn remove_record(env: &Env, github_username: &String) {
    let key = (REG_KEY, canon(env, github_username));
    env.storage().persistent().remove(&key);
}

/// Existence check for a single registration.
///
/// **Complexity: O(1).** This is a direct persistent-key `has` on
/// `(REG_KEY, github_username)` — it never touches the flat or chunked
/// username index, so its cost does not grow with the number of registrations
/// or chunks (Issue #291). It also does not deserialize the
/// [`ContributorRecord`] or extend its TTL; callers that need the record or a
/// TTL bump must go through [`get_record`].
pub fn has_record(env: &Env, github_username: &String) -> bool {
    env.storage()
        .persistent()
        .has(&(REG_KEY, github_username.clone()))
}

/// Build the light-client existence proof for `github_username` (Issue #230).
///
/// Deliberately reads through `get_record`, so an existing record gets the same
/// TTL bump any other read would give it and a proof cannot be used to sidestep
/// keeping a live record alive.
///
/// The exact `liveUntilLedgerSeq` is not returned: a contract cannot read its
/// own entry's TTL on-chain. The key and the TTL policy are returned instead so
/// a client can read `liveUntilLedgerSeq` straight from the ledger entry — see
/// `docs/STORAGE_RENT.md`.
pub fn build_record_proof(env: &Env, github_username: &String) -> RecordProof {
    let record = get_record(env, github_username);
    let as_of_ledger = env.ledger().sequence();
    match record {
        Some(record) => RecordProof {
            exists: true,
            verified: record.verified,
            registered_at: record.registered_at,
            as_of_ledger,
            ttl_threshold_ledgers: TTL_THRESHOLD,
            ttl_bump_ledgers: TTL_BUMP,
            key_prefix: REG_KEY,
        },
        None => RecordProof {
            exists: false,
            verified: false,
            registered_at: 0,
            as_of_ledger,
            ttl_threshold_ledgers: TTL_THRESHOLD,
            ttl_bump_ledgers: TTL_BUMP,
            key_prefix: REG_KEY,
        },
    }
}

// ── Counters ─────────────────────────────────────────────────────────────────

pub fn get_count(env: &Env) -> u32 {
    env.storage().instance().get(&COUNT_KEY).unwrap_or(0)
}

pub fn set_count(env: &Env, count: u32) {
    env.storage().instance().set(&COUNT_KEY, &count);
}

pub fn get_verified_count(env: &Env) -> u32 {
    env.storage().instance().get(&VCOUNT_KEY).unwrap_or(0)
}

pub fn set_verified_count(env: &Env, count: u32) {
    env.storage().instance().set(&VCOUNT_KEY, &count);
}

/// Verifications ever granted, including those later revoked (Issue #229).
///
/// Instances deployed before this counter existed have no stored value. Rather
/// than reporting zero — which would claim nobody was ever verified while
/// `get_verified_count` says otherwise — they fall back to the live verified
/// count, the tightest lower bound the contract can still prove.
pub fn get_ever_verified_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&EVER_VCOUNT_KEY)
        .unwrap_or_else(|| get_verified_count(env))
}

pub fn set_ever_verified_count(env: &Env, count: u32) {
    env.storage().instance().set(&EVER_VCOUNT_KEY, &count);
}

/// Record one more verification in the monotonic counter. Saturates rather than
/// wrapping, so the figure can only ever stall, never run backwards.
pub fn bump_ever_verified_count(env: &Env) {
    let next = get_ever_verified_count(env).saturating_add(1);
    set_ever_verified_count(env, next);
}

// ── Flat username index ──────────────────────────────────────────────────────

pub fn get_index(env: &Env) -> Vec<String> {
    env.storage()
        .instance()
        .get(&INDEX_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_index(env: &Env, index: &Vec<String>) {
    env.storage().instance().set(&INDEX_KEY, index);
}

/// Returns a bounded page of usernames from the flat index starting at `offset`.
///
/// Used by `get_registered_page` for admin exports. Clamps `limit` to
/// `MAX_PAGE_LIMIT` and applies `DEFAULT_PAGE_LIMIT` when `limit == 0`.
pub fn get_index_page(env: &Env, offset: u32, limit: u32) -> Vec<String> {
    let index = get_index(env);
    let mut page = Vec::new(env);

    let effective_limit = if limit == 0 {
        DEFAULT_PAGE_LIMIT
    } else {
        limit.min(MAX_PAGE_LIMIT)
    };

    if offset >= index.len() {
        return page;
    }

    let end = offset.saturating_add(effective_limit).min(index.len());
    for i in offset..end {
        if let Some(u) = index.get(i) {
            page.push_back(u);
        }
    }
    page
}

// ── Chunked username index ───────────────────────────────────────────────────

pub fn get_chunk_count(env: &Env) -> u32 {
    env.storage().instance().get(&CHUNK_CNT_KEY).unwrap_or(0)
}

pub fn set_chunk_count(env: &Env, count: u32) {
    env.storage().instance().set(&CHUNK_CNT_KEY, &count);
}

pub fn get_chunk(env: &Env, chunk_idx: u32) -> Vec<String> {
    let key = (CHUNK_KEY, chunk_idx);
    let chunk: Vec<String> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));
    if !chunk.is_empty() {
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
    }
    chunk
}

pub fn set_chunk(env: &Env, chunk_idx: u32, chunk: &Vec<String>) {
    let key = (CHUNK_KEY, chunk_idx);
    env.storage().persistent().set(&key, chunk);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
}

pub fn add_to_index(env: &Env, github_username: &String) {
    // 1. Maintain legacy single-vec index
    let mut index = get_index(env);
    index.push_back(canon(env, github_username));
    set_index(env, &index);

    // 2. Maintain chunked index
    let chunk_cnt = get_chunk_count(env);
    if chunk_cnt == 0 {
        let mut first_chunk = Vec::new(env);
        first_chunk.push_back(canon(env, github_username));
        set_chunk(env, 0, &first_chunk);
        set_chunk_count(env, 1);
    } else {
        let last_idx = chunk_cnt - 1;
        let mut last_chunk = get_chunk(env, last_idx);
        if last_chunk.len() >= CHUNK_SIZE {
            let mut new_chunk = Vec::new(env);
            new_chunk.push_back(canon(env, github_username));
            set_chunk(env, chunk_cnt, &new_chunk);
            set_chunk_count(env, chunk_cnt + 1);
        } else {
            last_chunk.push_back(canon(env, github_username));
            set_chunk(env, last_idx, &last_chunk);
        }
    }
}

/// Removes `github_username` from both the legacy flat index and the chunked
/// index.
///
/// Empty-registry invariant (Issue #92): removing the last remaining entry
/// must leave the legacy index at length 0 and the chunk that held it empty,
/// not a stale hole — `get_all_registered`, `get_index_page`, and the export
/// paths must all observe a clean empty registry afterward, and a subsequent
/// registration must proceed exactly as it would on a never-used registry.
/// Covered by `test_remove_last_user_returns_registry_to_empty_state` in
/// `src/lib.rs`.
pub fn remove_from_index(env: &Env, github_username: &String) {
    // The index stores the canonical form (see `add_to_index`), so the
    // comparison below must fold the same way or a case-variant caller would
    // silently fail to remove the entry it just looked up.
    let target = canon(env, github_username);

    // 1. Legacy index update
    let index = get_index(env);
    let mut next = Vec::new(env);
    for i in 0..index.len() {
        let username = index.get(i).unwrap();
        if username != target {
            next.push_back(username);
        }
    }
    set_index(env, &next);

    // Every position after the removed entry just shifted back by one, so
    // any pagination cursor issued before this point is no longer safe to
    // resume from (Issue #215) — bump the generation so `decode_cursor`
    // rejects it instead of silently returning drifted results.
    bump_index_generation(env);

    // 2. Chunked index update
    let chunk_cnt = get_chunk_count(env);
    for c in 0..chunk_cnt {
        let chunk = get_chunk(env, c);
        let mut new_chunk = Vec::new(env);
        let mut found = false;
        for i in 0..chunk.len() {
            let username = chunk.get(i).unwrap();
            if username == target {
                found = true;
            } else {
                new_chunk.push_back(username);
            }
        }
        if found {
            set_chunk(env, c, &new_chunk);
            break;
        }
    }
}

// ── Opaque pagination cursors (Issue #215) ───────────────────────────────────

/// Current index generation. Bumped by [`remove_from_index`] every time an
/// existing entry's position could shift. Instance-scoped, defaulting to `0`
/// for a fresh or pre-Issue-#215 instance.
pub fn get_index_generation(env: &Env) -> u32 {
    env.storage().instance().get(&INDEX_GEN_KEY).unwrap_or(0)
}

fn bump_index_generation(env: &Env) {
    let next = get_index_generation(env).saturating_add(1);
    env.storage().instance().set(&INDEX_GEN_KEY, &next);
}

/// Packs a flat-index offset together with the current index generation into
/// an 8-byte opaque cursor: bytes `0..4` are the offset, bytes `4..8` are the
/// generation, both big-endian. Callers must treat the result as opaque —
/// this layout is an internal implementation detail, not a stabilized wire
/// format.
fn encode_cursor(env: &Env, offset: u32) -> BytesN<8> {
    let generation = get_index_generation(env);
    let mut bytes = [0u8; 8];
    bytes[0..4].copy_from_slice(&offset.to_be_bytes());
    bytes[4..8].copy_from_slice(&generation.to_be_bytes());
    BytesN::from_array(env, &bytes)
}

/// Decodes and validates a cursor previously returned by this contract.
///
/// # Errors
///
/// - [`ContractError::InvalidCursor`] if the cursor's embedded generation no
///   longer matches the current one — meaning a username was removed from
///   the registry (shifting positions) since the cursor was issued — or if
///   the embedded offset is past the current registry size. Either way, the
///   safe recovery is to restart pagination from `cursor = None`; existing
///   idempotent-upsert indexer logic (see `docs/DASHBOARD_SYNC.md`) makes a
///   restart safe to replay.
fn decode_cursor(env: &Env, cursor: &BytesN<8>) -> Result<u32, ContractError> {
    let bytes = cursor.to_array();
    let offset = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let generation = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);

    if generation != get_index_generation(env) {
        return Err(ContractError::InvalidCursor);
    }
    if offset > get_count(env) {
        return Err(ContractError::InvalidCursor);
    }
    Ok(offset)
}

// ── Paginated export (Issue #1 & #3) ─────────────────────────────────────────

/// Returns a bounded page of `(username, record)` pairs starting at `cursor`.
///
/// `cursor` is `None` to start from the beginning, or a value previously
/// returned as `ExportPage::next_cursor` to continue — see the opaque-cursor
/// notes on [`ExportPage`] and [`decode_cursor`] (Issue #215).
///
/// `limit == 0` falls back to `DEFAULT_PAGE_LIMIT`; anything above
/// `MAX_PAGE_LIMIT` is clamped down to it rather than rejected — a caller
/// asking for too much gets the largest page the contract allows instead of
/// an error.
///
/// # Errors
///
/// - [`ContractError::NotInitialized`] if `initialize` has not been called.
/// - [`ContractError::InvalidCursor`] if `cursor` is `Some` and fails to
///   decode against the current index generation or registry size.
pub fn get_registered_paginated_internal(
    env: &Env,
    cursor: Option<BytesN<8>>,
    limit: u32,
) -> Result<ExportPage, ContractError> {
    require_initialized(env)?;

    let offset = match cursor {
        Some(c) => decode_cursor(env, &c)?,
        None => 0,
    };

    let effective_limit = if limit == 0 {
        DEFAULT_PAGE_LIMIT
    } else {
        limit.min(MAX_PAGE_LIMIT)
    };

    let total_count = get_count(env);
    let mut records = Vec::new(env);

    if offset >= total_count {
        return Ok(ExportPage {
            records,
            next_cursor: None,
            total: total_count,
            has_more: false,
            merkle_root: crate::merkle::empty_root(env),
        });
    }

    let index = get_index(env);
    let end = (offset.saturating_add(effective_limit)).min(index.len());

    for i in offset..end {
        if let Some(username) = index.get(i) {
            if let Some(record) = get_record(env, &username) {
                records.push_back((username, record));
            }
        }
    }

    let next_cursor = if end < index.len() {
        Some(encode_cursor(env, end))
    } else {
        None
    };
    let has_more = next_cursor.is_some();
    let merkle_root = crate::merkle::root_of_records(env, &records);

    Ok(ExportPage {
        records,
        next_cursor,
        total: total_count,
        has_more,
        merkle_root,
    })
}

/// Builds the deterministic digest used by [`ExportAttestation`] (Issue #223).
///
/// Hashes `page`'s full XDR encoding with SHA-256. XDR encoding of a
/// `#[contracttype]` struct is deterministic for a given value — same
/// records, same cursor state, same total, same bytes in, same bytes out —
/// so two calls against unchanged state always produce the same digest, and
/// any change to a single record, the total, or the pagination cursor
/// changes it. The exact byte layout is XDR, not part of this contract's
/// public surface; callers should treat the digest as opaque and compare it
/// byte for byte rather than parse it.
#[must_use]
pub fn build_export_digest(env: &Env, page: &ExportPage) -> BytesN<32> {
    let encoded: Bytes = page.clone().to_xdr(env);
    env.crypto().sha256(&encoded).into()
}

// ── Stats ────────────────────────────────────────────────────────────────────

// Wave #41: build_stats is the single centralized constructor for `Stats`.
// All stats reads (get_stats, and any future indexer/dashboard aggregate
// endpoints) should route through it rather than building `Stats { .. }`
// literals directly, so count/verified-count semantics stay in one place.
pub fn build_stats(total: u32, verified: u32, ever_verified: u32) -> Stats {
    Stats {
        total,
        verified,
        ever_verified,
    }
}

pub fn get_stats(env: &Env) -> Stats {
    build_stats(
        get_count(env),
        get_verified_count(env),
        get_ever_verified_count(env),
    )
}

// ── Cooldown / upgrade timelock ───────────────────────────────────────────────

pub fn get_cooldown(env: &Env) -> u64 {
    env.storage().instance().get(&COOLDOWN_KEY).unwrap_or(0)
}

pub fn get_emergency_pause(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&EMERGENCY_PAUSE_KEY)
        .unwrap_or(false)
}

pub fn set_emergency_pause(env: &Env, flag: bool) {
    env.storage().instance().set(&EMERGENCY_PAUSE_KEY, &flag);
}

pub fn get_emergency_pause_ts(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&EMERGENCY_PAUSE_TS_KEY)
        .unwrap_or(0)
}

pub fn set_emergency_pause_ts(env: &Env, ts: u64) {
    env.storage().instance().set(&EMERGENCY_PAUSE_TS_KEY, &ts);
}

/// Rejects the call while the emergency pause flag is set.
pub fn require_not_emergency_paused(env: &Env) -> Result<(), ContractError> {
    if get_emergency_pause(env) {
        Err(ContractError::Paused)
    } else {
        Ok(())
    }
}

pub fn get_org_index(env: &Env) -> Vec<String> {
    env.storage()
        .instance()
        .get(&ORG_INDEX_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_org_index(env: &Env, index: &Vec<String>) {
    env.storage().instance().set(&ORG_INDEX_KEY, index);
}

pub fn add_to_org_index(env: &Env, org_name: &String) {
    let mut index = get_org_index(env);
    index.push_back(org_name.clone());
    set_org_index(env, &index);
}

pub fn remove_from_org_index(env: &Env, org_name: &String) {
    let index = get_org_index(env);
    let mut next = Vec::new(env);
    for i in 0..index.len() {
        let name = index.get(i).unwrap();
        if name != *org_name {
            next.push_back(name);
        }
    }
    set_org_index(env, &next);
}

pub fn get_team_index(env: &Env) -> Vec<String> {
    env.storage()
        .instance()
        .get(&TEAM_INDEX_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn set_team_index(env: &Env, index: &Vec<String>) {
    env.storage().instance().set(&TEAM_INDEX_KEY, index);
}

pub fn add_to_team_index(env: &Env, key: &String) {
    let mut index = get_team_index(env);
    index.push_back(key.clone());
    set_team_index(env, &index);
}

pub fn remove_from_team_index(env: &Env, key: &String) {
    let index = get_team_index(env);
    let mut next = Vec::new(env);
    for i in 0..index.len() {
        let k = index.get(i).unwrap();
        if k != *key {
            next.push_back(k);
        }
    }
    set_team_index(env, &next);
}

// ── Guardian (Issue #196) ─────────────────────────────────────────────────────

/// Returns the designated guardian address, or `None` if none has been set.
pub fn get_guardian(env: &Env) -> Option<Address> {
    env.storage().instance().get(&GUARDIAN_KEY)
}

/// Sets (or replaces) the guardian address. Admin-only write path.
pub fn set_guardian_address(env: &Env, guardian: &Address) {
    env.storage().instance().set(&GUARDIAN_KEY, guardian);
}

/// Removes the guardian address entirely.
pub fn remove_guardian(env: &Env) {
    env.storage().instance().remove(&GUARDIAN_KEY);
}

/// Returns `true` when `address` is the current guardian.
pub fn is_guardian(env: &Env, address: &Address) -> bool {
    matches!(get_guardian(env), Some(g) if g == *address)
}

pub fn set_cooldown(env: &Env, cooldown_seconds: u64) {
    env.storage()
        .instance()
        .set(&COOLDOWN_KEY, &cooldown_seconds);
}

/// Returns `true` if the contract's pause flag is set.
pub fn is_paused(env: &Env) -> bool {
    env.storage().instance().get(&PAUSED_KEY).unwrap_or(false)
}

/// Rejects the call while the contract is paused (normal or emergency).
pub fn require_not_paused(env: &Env) -> Result<(), ContractError> {
    if is_paused(env) || get_emergency_pause(env) {
        Err(ContractError::Paused)
    } else {
        Ok(())
    }
}

/// Sets the contract pause flag. Called by `pause` / `unpause`.
pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&PAUSED_KEY, &paused);
}

/// Returns whether `github_username` has a pending re-verification flag set.
///
/// This flag is set when a verified user re-registers to a different Stellar
/// address, indicating a new off-chain GitHub identity check is needed.
/// It is cleared when the record is successfully `verify`'d.
pub fn get_pending_reverify(env: &Env, github_username: &String) -> bool {
    let key = (PENDING_REVERIFY_KEY, canon(env, github_username));
    env.storage().persistent().get(&key).unwrap_or(false)
}

pub fn set_pending_reverify(env: &Env, github_username: &String, flag: bool) {
    let key = (PENDING_REVERIFY_KEY, canon(env, github_username));
    env.storage().persistent().set(&key, &flag);
}

pub fn clear_pending_reverify(env: &Env, github_username: &String) {
    let key = (PENDING_REVERIFY_KEY, canon(env, github_username));
    env.storage().persistent().remove(&key);
}

pub fn get_last_upgrade(env: &Env) -> u64 {
    env.storage().instance().get(&LAST_UPG_KEY).unwrap_or(0)
}

pub fn set_last_upgrade(env: &Env, timestamp: u64) {
    env.storage().instance().set(&LAST_UPG_KEY, &timestamp);
}

// ── Version ──────────────────────────────────────────────────────────────────

/// Returns the version recorded at initialize time, or `None` for instances
/// deployed before version tracking existed.
pub fn get_version(env: &Env) -> Option<(u32, u32, u32)> {
    env.storage().instance().get(&VERSION_KEY)
}

pub fn set_version(env: &Env, version: (u32, u32, u32)) {
    env.storage().instance().set(&VERSION_KEY, &version);
}

// ─── WASM provenance & attestation (Wave #24) ────────────────────────────────

/// Provenance of the currently deployed WASM. `None` before the first upgrade.
pub fn get_wasm_provenance(env: &Env) -> Option<WasmProvenance> {
    env.storage().instance().get(&PROV_KEY)
}

pub fn set_wasm_provenance(env: &Env, provenance: &WasmProvenance) {
    env.storage().instance().set(&PROV_KEY, provenance);
}

/// The pending upgrade attestation, if one has been published.
///
/// Returns the raw record regardless of expiry — callers decide what to do with
/// a lapsed attestation, and `get_wasm_attestation` is also a read endpoint
/// where seeing the expired value is useful for diagnosis.
pub fn get_wasm_attestation(env: &Env) -> Option<WasmAttestation> {
    env.storage().instance().get(&ATTEST_KEY)
}

pub fn set_wasm_attestation(env: &Env, attestation: &WasmAttestation) {
    env.storage().instance().set(&ATTEST_KEY, attestation);
}

pub fn remove_wasm_attestation(env: &Env) {
    env.storage().instance().remove(&ATTEST_KEY);
}

// ── Admin transfer (Issue #195) ───────────────────────────────────────────────

/// Returns the pending admin transfer proposal, if one exists.
pub fn get_admin_transfer(env: &Env) -> Option<AdminTransferProposal> {
    env.storage().instance().get(&ADMIN_TRANSFER_KEY)
}

/// Stores a new admin transfer proposal, overwriting any existing one.
pub fn set_admin_transfer(env: &Env, proposal: &AdminTransferProposal) {
    env.storage().instance().set(&ADMIN_TRANSFER_KEY, proposal);
}

/// Removes the pending admin transfer proposal.
pub fn clear_admin_transfer(env: &Env) {
    env.storage().instance().remove(&ADMIN_TRANSFER_KEY);
}

// ── Attestation-required config (Issue #198) ──────────────────────────────────

/// Returns whether WASM attestation is required before an upgrade.
/// Defaults to `false` (opt-in two-step mode) to preserve backward compatibility.
pub fn is_attestation_required(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&ATTEST_REQUIRED_KEY)
        .unwrap_or(false)
}

/// Sets the attestation-required flag.
pub fn set_attestation_required(env: &Env, required: bool) {
    env.storage()
        .instance()
        .set(&ATTEST_REQUIRED_KEY, &required);
}

// ── Per-user action cooldown (Wave #33) ──────────────────────────────────────

/// Timestamp of `github_username`'s last cooldown-tracked action, or 0 if it
/// has none. Cooldown is tracked per username rather than globally so one
/// contributor's activity cannot block everyone else's.
pub fn get_last_action(env: &Env, github_username: &String) -> u64 {
    env.storage()
        .persistent()
        .get(&(LAST_ACT_KEY, canon(env, github_username)))
        .unwrap_or(0)
}

/// Records the ledger timestamp of the last mutating action for `github_username`.
pub fn set_last_action(env: &Env, github_username: &String, timestamp: u64) {
    let key = (LAST_ACT_KEY, canon(env, github_username));
    env.storage().persistent().set(&key, &timestamp);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
}

/// True when the configured cooldown has not yet elapsed since
/// `github_username`'s last tracked action. A cooldown of 0 disables the
/// check entirely.
pub fn is_in_cooldown(env: &Env, github_username: &String) -> bool {
    let cooldown = get_cooldown(env);
    if cooldown == 0 {
        return false;
    }
    let last = get_last_action(env, github_username);
    if last == 0 {
        return false;
    }
    env.ledger().timestamp() < last.saturating_add(cooldown)
}

// ── Per-verifier verify/revoke rate limit (Issue #292) ───────────────────────
//
// A Verifier or Revoker key can otherwise call `verify` / `revoke_verification`
// / `batch_verify` as fast as it can submit transactions, bloating the event
// stream and burning instruction budget during a Wave. This is an on-chain cap
// on the number of verify/revoke *units* one actor may spend per ledger.
//
// - The counter is keyed per actor address and per ledger sequence. When the
//   ledger rolls over, the first call in the new ledger sees a stale sequence
//   and resets the count to 0 — there is no cross-ledger carry.
// - `batch_verify` spends `usernames.len()` units, so a batch cannot be used to
//   sidestep the per-ledger cap.
// - The admin is exempt: callers check `is_admin_caller` first and never reach
//   the rate check. This is deliberate — incident response must not be throttled.
// - Reads are never rate-limited; only the mutating verify/revoke paths are.

/// Instance key holding the configured per-ledger verify/revoke cap.
/// Absent → [`DEFAULT_VERIFIES_PER_LEDGER`]. Explicitly set to 0 → disabled.
pub const VERIFY_LIMIT_KEY: Symbol = symbol_short!("vfylimit");

/// Persistent key prefix for the per-`(verifier, ledger)` spend counter.
/// Value is `(u32 ledger_seq, u32 units_spent)`.
pub const VERIFY_RATE_KEY: Symbol = symbol_short!("vfyrate");

/// Default cap when the admin has not configured one: 20 verify/revoke units
/// per verifier per ledger (~4 per second at 5s ledgers).
pub const DEFAULT_VERIFIES_PER_LEDGER: u32 = 20;

/// The active per-ledger verify/revoke cap. Returns the stored value when the
/// admin has set one (including 0, which disables the limit), otherwise
/// [`DEFAULT_VERIFIES_PER_LEDGER`].
pub fn get_verify_limit(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&VERIFY_LIMIT_KEY)
        .unwrap_or(DEFAULT_VERIFIES_PER_LEDGER)
}

/// Sets the per-ledger verify/revoke cap. `0` disables rate limiting entirely.
pub fn set_verify_limit(env: &Env, limit: u32) {
    env.storage().instance().set(&VERIFY_LIMIT_KEY, &limit);
}

/// Charges `units` against `verifier`'s allowance for the current ledger.
///
/// Resets the counter transparently on ledger rollover. Returns
/// [`ContractError::VerifyRateLimited`] — without writing — if the charge would
/// push this ledger's spend past the cap. A cap of 0 short-circuits to `Ok`.
///
/// The admin is expected to have been filtered out by the caller before this
/// runs; nothing here special-cases it.
pub fn charge_verify_rate(
    env: &Env,
    verifier: &Address,
    units: u32,
) -> Result<(), ContractError> {
    let limit = get_verify_limit(env);
    if limit == 0 {
        return Ok(());
    }

    let current_seq = env.ledger().sequence();
    let key = (VERIFY_RATE_KEY, verifier.clone());
    let (seq, spent): (u32, u32) = env.storage().persistent().get(&key).unwrap_or((0, 0));

    let spent_this_ledger = if seq == current_seq { spent } else { 0 };
    let next = spent_this_ledger.saturating_add(units);
    if next > limit {
        return Err(ContractError::VerifyRateLimited);
    }

    env.storage().persistent().set(&key, &(current_seq, next));
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
    Ok(())
}

/// Units `verifier` has already spent in the current ledger (0 after rollover).
/// Read-only helper for tests and diagnostics.
pub fn verify_units_spent(env: &Env, verifier: &Address) -> u32 {
    let key = (VERIFY_RATE_KEY, verifier.clone());
    match env.storage().persistent().get::<_, (u32, u32)>(&key) {
        Some((seq, spent)) if seq == env.ledger().sequence() => spent,
        _ => 0,
    }
}

// ── Role-based access control ─────────────────────────────────────────────────

/// Raw existence check against `ROLE_KEY`, ignoring expiry (Issue #221).
///
/// Used only to decide whether `set_role_with_expiry` is granting a brand new
/// role (and therefore needs an index entry) versus refreshing one that
/// already has an index entry, possibly expired. An expired-but-not-yet-
/// `remove_role`'d entry must **not** be treated as new, or re-granting it
/// would append a duplicate to the enumeration index.
fn role_key_exists(env: &Env, address: &Address) -> bool {
    env.storage().persistent().has(&(ROLE_KEY, address.clone()))
}

/// The expiry timestamp for `address`'s current role grant, or `None` if it
/// was granted with no expiry (the default via `set_role`), or if `address`
/// holds no role at all (Issue #221).
#[must_use]
pub fn get_role_expiry(env: &Env, address: &Address) -> Option<u64> {
    env.storage()
        .persistent()
        .get(&(ROLE_EXPIRY_KEY, address.clone()))
}

/// `true` once `address`'s role grant has an expiry and the ledger clock has
/// reached or passed it (Issue #221). A grant with no expiry never expires.
#[must_use]
pub fn is_role_expired(env: &Env, address: &Address) -> bool {
    match get_role_expiry(env, address) {
        Some(expires_at) => env.ledger().timestamp() >= expires_at,
        None => false,
    }
}

/// Returns `address`'s currently active role, or `None` if it holds no role
/// **or** its grant has expired (Issue #221).
///
/// Expiry is lazy: an expired grant's `ROLE_KEY` / `ROLE_EXPIRY_KEY` entries
/// are left in storage untouched — only `remove_role` deletes them and drops
/// the address from `get_role_holders`. Every role check in this contract
/// (`verify`, `revoke_verification`, `batch_verify`, `get_role_holders`,
/// `has_role_or_admin`) is built on this function, so a lapsed grant reads as
/// "no role" everywhere without each call site re-checking expiry itself.
pub fn get_role(env: &Env, address: &Address) -> Option<Role> {
    let role: Option<Role> = env.storage().persistent().get(&(ROLE_KEY, address.clone()));
    let role = role?;
    if is_role_expired(env, address) {
        return None;
    }
    Some(role)
}

/// Grants `role` to `address` with no expiry and keeps the enumeration index
/// in step. Equivalent to `set_role_with_expiry(env, address, role, None)`.
///
/// Re-granting to an address that already holds a role overwrites the role but
/// must **not** append a second index entry, or the address would be reported
/// twice by `get_role_holders`.
pub fn set_role(env: &Env, address: &Address, role: &Role) {
    set_role_with_expiry(env, address, role, None);
}

/// Grants `role` to `address`, optionally expiring at `expires_at` (a ledger
/// timestamp) so a contractor or bot key cannot linger as a live `Verifier`
/// or `Revoker` after off-boarding (Issue #221).
///
/// `expires_at: None` grants a role that never expires — identical to
/// `set_role`. `Some(ts)` in the past or exactly `env.ledger().timestamp()`
/// grants a role that reads as already expired the moment this call
/// returns; the grant is still recorded (and still shows up, unexpired-index-
/// wise, in `get_role_holders`) — it just never resolves as held by
/// `get_role`. This is deliberate: it is simpler for callers to reason about
/// than a validation error, and mirrors how the rest of this contract treats
/// boundary timestamps (`is_role_expired` uses `>=`).
///
/// Re-granting to an address that already has a `ROLE_KEY` entry (expired or
/// not) overwrites the role and expiry but does not touch the enumeration
/// index a second time.
///
/// The contract admin's authority is never affected by this: `is_admin_caller`
/// and `get_admin` read `ADMIN_KEY`, a separate, immutable storage slot that
/// this function never touches — only the `Role::Admin` RBAC entry granted
/// alongside it in `initialize` can expire, and only if a caller explicitly
/// grants `Role::Admin` with an expiry through this function.
pub fn set_role_with_expiry(env: &Env, address: &Address, role: &Role, expires_at: Option<u64>) {
    let is_new = !role_key_exists(env, address);
    env.storage()
        .persistent()
        .set(&(ROLE_KEY, address.clone()), role);

    let expiry_key = (ROLE_EXPIRY_KEY, address.clone());
    match expires_at {
        Some(ts) => {
            env.storage().persistent().set(&expiry_key, &ts);
            env.storage()
                .persistent()
                .extend_ttl(&expiry_key, TTL_THRESHOLD, TTL_BUMP);
        }
        None => env.storage().persistent().remove(&expiry_key),
    }

    if is_new {
        add_to_role_index(env, address);
    }
}

/// Revokes `address`'s role and drops it from the enumeration index.
pub fn remove_role(env: &Env, address: &Address) {
    env.storage()
        .persistent()
        .remove(&(ROLE_KEY, address.clone()));
    env.storage()
        .persistent()
        .remove(&(ROLE_EXPIRY_KEY, address.clone()));
    remove_from_role_index(env, address);
}

/// An `(address, role)` pair as returned by `get_role_holders` (Issue #228).
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct RoleHolder {
    pub address: Address,
    pub role: Role,
}

/// Raw enumeration index: every address that currently holds a role.
#[must_use]
pub fn get_role_index(env: &Env) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(&ROLE_IDX_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

fn set_role_index(env: &Env, index: &Vec<Address>) {
    env.storage().persistent().set(&ROLE_IDX_KEY, index);
    env.storage()
        .persistent()
        .extend_ttl(&ROLE_IDX_KEY, TTL_THRESHOLD, TTL_BUMP);
}

fn add_to_role_index(env: &Env, address: &Address) {
    let mut index = get_role_index(env);
    // Guard against a double entry even though `set_role` already checks for
    // an existing role: the two must agree, and this is the cheaper place to
    // be certain of it.
    if index.iter().any(|a| a == *address) {
        return;
    }
    index.push_back(address.clone());
    set_role_index(env, &index);
}

fn remove_from_role_index(env: &Env, address: &Address) {
    let index = get_role_index(env);
    let mut compacted = Vec::new(env);
    let mut found = false;
    for entry in index.iter() {
        if entry == *address {
            found = true;
        } else {
            compacted.push_back(entry);
        }
    }
    // Skip the write when nothing changed — `remove_role` is callable against
    // an address that never held a role, and that must not cost a storage write
    // or bump the index TTL.
    if found {
        set_role_index(env, &compacted);
    }
}

/// One page of `(address, role)` pairs, ordered by grant time.
///
/// Entries whose `ROLE_KEY` lookup comes back empty are skipped rather than
/// reported with a placeholder role: the index is maintained in lockstep with
/// the role entries, so a miss means the two have drifted, and inventing a
/// role for a stale index entry would hand the dashboard a privileged address
/// that does not exist on chain. Because the lookup goes through
/// [`get_role`], an address whose grant has **expired** (Issue #221) is
/// skipped the same way — expired role holders silently drop out of this
/// listing until `remove_role` compacts the index, matching `get_role`'s
/// "expired reads as absent" semantics.
#[must_use]
pub fn get_role_holders_internal(env: &Env, offset: u32, limit: u32) -> Vec<RoleHolder> {
    let capped = if limit == 0 || limit > MAX_ROLE_PAGE_LIMIT {
        MAX_ROLE_PAGE_LIMIT
    } else {
        limit
    };

    let index = get_role_index(env);
    let mut page = Vec::new(env);
    if offset >= index.len() {
        return page;
    }

    let end = offset.saturating_add(capped).min(index.len());
    for i in offset..end {
        let Some(address) = index.get(i) else { continue };
        if let Some(role) = get_role(env, &address) {
            page.push_back(RoleHolder { address, role });
        }
    }
    page
}

/// Number of addresses currently holding a role.
#[must_use]
pub fn get_role_holder_count(env: &Env) -> u32 {
    get_role_index(env).len()
}

/// True when `address` is the contract admin.
pub fn is_admin_caller(env: &Env, address: &Address) -> bool {
    matches!(get_admin(env), Ok(admin) if admin == *address)
}

#[allow(dead_code)] // Staged for role-gated entry points; covered by role tests.
pub fn has_role_or_admin(env: &Env, address: &Address, expected_role: Role) -> bool {
    if let Ok(admin) = get_admin(env) {
        if *address == admin {
            return true;
        }
    }
    match get_role(env, address) {
        Some(Role::Admin) => true,
        Some(r) => r == expected_role,
        None => false,
    }
}

// ── Verifier allowlist with on-chain expiry (Issue #293) ─────────────────────
//
// `set_role(Verifier)` is unbounded in both time and count. A campaign wants a
// small, hard-capped allowlist whose members auto-expire. No generic role-expiry
// mechanism exists in this contract, so expiry is implemented here (composed
// into the one place it is needed) rather than as a second parallel system.
//
// - Stored as a single `Vec<VerifierAllowEntry>` in instance storage. The cap
//   keeps it tiny, so a full-vector rewrite per mutation is fine.
// - `expires_at == 0` means "no expiry" (a standing campaign verifier).
// - Expired entries are pruned lazily on every write, so storage does not
//   accumulate dead members; `is_active_verifier` also treats a not-yet-pruned
//   expired entry as inactive, so a read is correct even between writes.

/// Instance key for the verifier allowlist vector (Issue #293).
pub const VERIFIER_ALLOWLIST_KEY: Symbol = symbol_short!("vfyallow");

/// Hard cap on the number of concurrently allowlisted verifiers (Issue #293).
pub const MAX_VERIFIERS: u32 = 10;

/// One entry in the verifier allowlist.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct VerifierAllowEntry {
    /// The allowlisted verifier address.
    pub address: Address,
    /// Ledger timestamp after which this entry is inactive. `0` == no expiry.
    pub expires_at: u64,
    /// Ledger timestamp the entry was added or last refreshed.
    pub added_at: u64,
}

/// The raw allowlist, including any entries that have expired but not yet been
/// pruned. Empty when no verifier has ever been allowlisted.
pub fn get_verifier_allowlist(env: &Env) -> Vec<VerifierAllowEntry> {
    env.storage()
        .instance()
        .get(&VERIFIER_ALLOWLIST_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

fn set_verifier_allowlist(env: &Env, list: &Vec<VerifierAllowEntry>) {
    env.storage()
        .instance()
        .set(&VERIFIER_ALLOWLIST_KEY, list);
}

/// `true` when the allowlist has ever been populated. Used to decide whether the
/// allowlist gates `verify` or the contract is still in pure role-based mode.
pub fn verifier_allowlist_active(env: &Env) -> bool {
    env.storage().instance().has(&VERIFIER_ALLOWLIST_KEY)
}

/// Drop every entry whose expiry has passed. Returns how many were removed.
pub fn prune_expired_verifiers(env: &Env, now: u64) -> u32 {
    let list = get_verifier_allowlist(env);
    let mut kept: Vec<VerifierAllowEntry> = Vec::new(env);
    let mut removed = 0u32;
    for e in list.iter() {
        if e.expires_at != 0 && now >= e.expires_at {
            removed += 1;
        } else {
            kept.push_back(e);
        }
    }
    if removed > 0 {
        set_verifier_allowlist(env, &kept);
    }
    removed
}

/// Number of entries that are currently active (present and not expired).
pub fn active_verifier_count(env: &Env, now: u64) -> u32 {
    let mut n = 0u32;
    for e in get_verifier_allowlist(env).iter() {
        if e.expires_at == 0 || now < e.expires_at {
            n += 1;
        }
    }
    n
}

/// `true` when `address` is on the allowlist and not expired as of `now`.
pub fn is_active_verifier(env: &Env, address: &Address, now: u64) -> bool {
    for e in get_verifier_allowlist(env).iter() {
        if e.address == *address {
            return e.expires_at == 0 || now < e.expires_at;
        }
    }
    false
}

/// Add `address` to the allowlist, or refresh its expiry if already present.
///
/// Expired entries are pruned first (so a lapsed member does not consume a
/// slot). Refreshing an existing member never counts against the cap; adding a
/// brand-new member does.
///
/// # Errors
///
/// - [`ContractError::VerifierExpiryInPast`] if `expires_at` is non-zero and not
///   strictly in the future.
/// - [`ContractError::VerifierAllowlistFull`] if adding a new member would
///   exceed [`MAX_VERIFIERS`].
pub fn add_verifier(
    env: &Env,
    address: &Address,
    expires_at: u64,
    now: u64,
) -> Result<(), ContractError> {
    if expires_at != 0 && expires_at <= now {
        return Err(ContractError::VerifierExpiryInPast);
    }

    prune_expired_verifiers(env, now);
    let list = get_verifier_allowlist(env);

    // Refresh path: address already listed → update expiry in place.
    let mut next: Vec<VerifierAllowEntry> = Vec::new(env);
    let mut refreshed = false;
    for e in list.iter() {
        if e.address == *address {
            next.push_back(VerifierAllowEntry {
                address: address.clone(),
                expires_at,
                added_at: now,
            });
            refreshed = true;
        } else {
            next.push_back(e);
        }
    }

    if !refreshed {
        if next.len() >= MAX_VERIFIERS {
            return Err(ContractError::VerifierAllowlistFull);
        }
        next.push_back(VerifierAllowEntry {
            address: address.clone(),
            expires_at,
            added_at: now,
        });
    }

    set_verifier_allowlist(env, &next);
    Ok(())
}

/// Remove `address` from the allowlist.
///
/// # Errors
///
/// - [`ContractError::VerifierNotAllowlisted`] if `address` is not on the list.
pub fn remove_verifier(env: &Env, address: &Address, now: u64) -> Result<(), ContractError> {
    let list = get_verifier_allowlist(env);
    let mut next: Vec<VerifierAllowEntry> = Vec::new(env);
    let mut found = false;
    for e in list.iter() {
        if e.address == *address {
            found = true;
        } else if e.expires_at != 0 && now >= e.expires_at {
            // opportunistically drop other expired entries too
        } else {
            next.push_back(e);
        }
    }
    if !found {
        return Err(ContractError::VerifierNotAllowlisted);
    }
    set_verifier_allowlist(env, &next);
    Ok(())
}

/// Slots still available before the [`MAX_VERIFIERS`] cap, counting only active
/// (non-expired) members.
pub fn verifier_slots_remaining(env: &Env, now: u64) -> u32 {
    MAX_VERIFIERS.saturating_sub(active_verifier_count(env, now))
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct VerificationConfig {
    pub attestation: Symbol,
    pub expires_in: u64,
    pub threshold: u32,
}

pub fn is_verification_configured(env: &Env) -> bool {
    env.storage().instance().has(&VER_CFG_KEY)
}

pub fn get_verification_config(env: &Env) -> Option<VerificationConfig> {
    env.storage().instance().get(&VER_CFG_KEY)
}

/// Stores the verification configuration. Idempotent — caller must gate
/// on [`is_verification_configured`] first.
pub fn set_verification_config(env: &Env, attestation: Symbol, expires_in: u64, threshold: u32) {
    let config = VerificationConfig {
        attestation,
        expires_in,
        threshold,
    };
    env.storage().instance().set(&VER_CFG_KEY, &config);
}

// ── Time-bounded verification (Issue #218) ───────────────────────────────────
//
// `config_verification`'s `expires_in` used to be stored and never read. A
// stolen or transferred GitHub account stayed `verified` forever unless an
// admin noticed and called `revoke_verification` — a verify-once-forever
// flag keeps paying out an attacker indefinitely. This wires `expires_in`
// into an actual expiry check, keyed off the ledger timestamp `verify()`
// last ran at.
//
// Expiry here is **lazy**, the same policy `Role` expiry (Issue #221) uses:
// `ContributorRecord.verified` is not flipped back to `false` in storage when
// the window lapses. Instead, `is_verification_expired` is the source of
// truth callers check alongside the raw flag; `is_verification_active` (in
// `lib.rs`) combines the two into the single effective read. This keeps
// `verified_count` / `get_stats` a cheap O(1) counter rather than a full
// registry scan on every read — see the doc comment on `get_stats` for what
// that counter does and does not include.

/// Ledger timestamp `github_username` was last `verify()`'d, or `None` if it
/// has never been verified (or `revoke_verification` cleared it) (Issue #218).
#[must_use]
pub fn get_verified_at(env: &Env, github_username: &String) -> Option<u64> {
    env.storage()
        .persistent()
        .get(&(VERIFIED_AT_KEY, github_username.clone()))
}

/// Records the ledger timestamp of a successful `verify()` call.
pub fn set_verified_at(env: &Env, github_username: &String, timestamp: u64) {
    let key = (VERIFIED_AT_KEY, github_username.clone());
    env.storage().persistent().set(&key, &timestamp);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
}

/// Clears the recorded verification timestamp. Called by
/// `revoke_verification` so a later fresh `verify()` is not compared against
/// a stale timestamp from a previous, since-revoked grant.
pub fn clear_verified_at(env: &Env, github_username: &String) {
    env.storage()
        .persistent()
        .remove(&(VERIFIED_AT_KEY, github_username.clone()));
}

/// `true` once `github_username`'s verification has passed the configured
/// `expires_in` window (Issue #218).
///
/// `false` whenever expiry cannot apply: verification was never configured
/// (`config_verification` was never called), `expires_in == 0` (configured
/// with no expiry), or the username has no recorded verification timestamp.
/// Callers combine this with the raw `ContributorRecord.verified` flag —
/// this function does not check `verified` itself, so it says nothing about
/// whether the username is verified at all, only whether a **prior**
/// verification's clock has run out.
#[must_use]
pub fn is_verification_expired(env: &Env, github_username: &String) -> bool {
    let Some(config) = get_verification_config(env) else {
        return false;
    };
    if config.expires_in == 0 {
        return false;
    }
    let Some(verified_at) = get_verified_at(env, github_username) else {
        return false;
    };
    env.ledger().timestamp() >= verified_at.saturating_add(config.expires_in)
}

// ── Audit log persistence ──────────────────────────────────────────────────

pub const MAX_AUDIT_LOG_ENTRIES: u32 = 100;

pub fn get_audit_logs(env: &Env) -> Vec<crate::audit::AuditLogEntry> {
    env.storage()
        .instance()
        .get(&AUDIT_LOG_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn push_audit_entry(env: &Env, entry: crate::audit::AuditLogEntry) {
    let mut logs = get_audit_logs(env);
    let mut stats = get_audit_stats(env);

    stats.record_event(entry.event_type);
    set_audit_stats(env, &stats);

    if logs.len() >= MAX_AUDIT_LOG_ENTRIES {
        logs.pop_front();
    }
    logs.push_back(entry);
    env.storage().instance().set(&AUDIT_LOG_KEY, &logs);
}

pub fn get_audit_stats(env: &Env) -> crate::audit::AuditStats {
    env.storage()
        .instance()
        .get(&AUDIT_STATS_KEY)
        .unwrap_or_default()
}

pub fn set_audit_stats(env: &Env, stats: &crate::audit::AuditStats) {
    env.storage().instance().set(&AUDIT_STATS_KEY, stats);
}

// ── Migration step registry (Issue #207) ─────────────────────────────────────
//
// Each entry maps a `(from_major, from_minor, from_patch)` version to a
// concrete data-migration function.  `run_migration_steps` walks the table in
// order and applies every applicable step whose `from` version is less than
// `target`, skipping steps already past (idempotency via version check) and
// steps that would overshoot `target`.
//
// v1.0.0 → v1.1.0  NormalizeRegisteredAt
//   `ContributorRecord.registered_at` was stored as `u64` in the initial
//   schema and later changed to `u32` (saves 4 bytes per record, fits until
//   2106).  On a freshly-deployed v1.1.0+ instance the field is always `u32`,
//   but instances upgraded from v1.0.0 may carry stale `u64` XDR.  This step
//   visits each record in the flat index and rewrites it, touching only records
//   that can be deserialized (stale ones fail silently so the batch is never
//   partially-poison).  The step is a no-op on a clean deployment.

/// Describes one migration step in the registry table.
pub struct MigrationStep {
    /// The version this step migrates **from** (exclusive lower bound).
    /// Applied only when `current_version < from_version` is false *and*
    /// `from_version <= target_version`.
    pub from_version: (u32, u32, u32),
}

/// All known migration steps, in ascending version order.
///
/// Add new entries here when a layout change requires a data migration.
pub const MIGRATION_STEPS: &[MigrationStep] = &[
    MigrationStep {
        from_version: (1, 0, 0),
    }, // v1.0.0 → v1.1.0: NormalizeRegisteredAt (no-op on fresh deploys)
];

/// Runs every migration step whose `from_version` falls in the window
/// `(current, target]` and returns the number of steps applied.
///
/// Idempotent: calling again with the same `current` / `target` pair
/// returns 0 because `current >= step.from_version` after the first run.
pub fn run_migration_steps(
    env: &Env,
    current: (u32, u32, u32),
    target: (u32, u32, u32),
) -> u32 {
    let mut applied: u32 = 0;

    for step in MIGRATION_STEPS {
        // Only apply steps that close the gap between current and target.
        if step.from_version < current || step.from_version >= target {
            continue;
        }

        // v1.0.0 → v1.1.0: NormalizeRegisteredAt
        // Re-save every record so the XDR uses the current ContributorRecord
        // layout. Records that are already correct are rewritten identically
        // (idempotent). Records that are missing or unreadable are skipped.
        if step.from_version == (1, 0, 0) {
            let index = get_index(env);
            for i in 0..index.len() {
                if let Some(username) = index.get(i) {
                    if let Some(record) = get_record(env, &username) {
                        // Re-serialise with the current layout.
                        set_record(env, &username, &record);
                    }
                }
            }
        }

        applied = applied.saturating_add(1);
    }

    applied
}

// ── Pause reason ─────────────────────────────────────────────────────────────

/// Record why the contract was last paused or unpaused.
pub fn set_pause_reason(env: &Env, reason: PauseReason) {
    env.storage()
        .instance()
        .set(&PAUSE_RSN_KEY, &(reason as u32));
}

/// The last recorded pause reason, or `None` if the contract has never been
/// paused on this instance.
pub fn get_pause_reason(env: &Env) -> Option<PauseReason> {
    env.storage()
        .instance()
        .get::<Symbol, u32>(&PAUSE_RSN_KEY)
        .and_then(PauseReason::from_code)
}

// ── Reserved usernames (Issue #213) ──────────────────────────────────────────

/// The reserved username list, empty when nothing has been reserved.
pub fn get_reserved_list(env: &Env) -> Vec<String> {
    env.storage()
        .instance()
        .get(&RESERVED_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

/// Case-insensitive membership test against the reserved list.
pub fn is_reserved(env: &Env, username: &String) -> bool {
    let reserved = get_reserved_list(env);
    for entry in reserved.iter() {
        if crate::utils::eq_ignore_ascii_case(&entry, username) {
            return true;
        }
    }
    false
}

/// Add `username` to the reserved list.
///
/// # Errors
///
/// - [`ContractError::AlreadyReserved`] if it is already on the list.
/// - [`ContractError::ReservedListFull`] if the list already holds
///   `MAX_RESERVED` entries.
pub fn add_to_reserved(env: &Env, username: &String) -> Result<(), ContractError> {
    if is_reserved(env, username) {
        return Err(ContractError::AlreadyReserved);
    }
    let mut reserved = get_reserved_list(env);
    if reserved.len() >= MAX_RESERVED {
        return Err(ContractError::ReservedListFull);
    }
    reserved.push_back(username.clone());
    env.storage().instance().set(&RESERVED_KEY, &reserved);
    Ok(())
}

/// Remove `username` from the reserved list.
///
/// # Errors
///
/// - [`ContractError::NotReserved`] if it is not currently reserved.
pub fn remove_from_reserved(env: &Env, username: &String) -> Result<(), ContractError> {
    let reserved = get_reserved_list(env);
    let mut remaining: Vec<String> = Vec::new(env);
    let mut found = false;
    for entry in reserved.iter() {
        if crate::utils::eq_ignore_ascii_case(&entry, username) {
            found = true;
        } else {
            remaining.push_back(entry);
        }
    }
    if !found {
        return Err(ContractError::NotReserved);
    }
    env.storage().instance().set(&RESERVED_KEY, &remaining);
    Ok(())
}

// ── Index compaction (Issue #209) ────────────────────────────────────────────

/// Rebuild the chunked index densely from the flat index.
///
/// Removals leave holes in the chunk pages; this re-partitions the flat index
/// into contiguous full chunks plus one partial tail and drops the persistent
/// entries that are no longer backed by any username. Returns the number of
/// chunks written.
pub fn compact_chunked_index(env: &Env) -> u32 {
    let index = get_index(env);
    let previous_chunks = get_chunk_count(env);

    let mut chunk_idx: u32 = 0;
    let mut current: Vec<String> = Vec::new(env);
    for username in index.iter() {
        current.push_back(username);
        if current.len() >= CHUNK_SIZE {
            set_chunk(env, chunk_idx, &current);
            chunk_idx += 1;
            current = Vec::new(env);
        }
    }

    // A partial tail still needs a page of its own.
    if !current.is_empty() {
        set_chunk(env, chunk_idx, &current);
        chunk_idx += 1;
    }

    // Reclaim pages the compacted index no longer reaches.
    let mut stale = chunk_idx;
    while stale < previous_chunks {
        env.storage().persistent().remove(&(CHUNK_KEY, stale));
        stale += 1;
    }

    set_chunk_count(env, chunk_idx);
    chunk_idx
}

// ── Address rotation (Issue #234) ────────────────────────────────────────────

/// Seconds a requested rotation must wait before it can execute. 0 disables the
/// delay, in which case `register` keeps its direct dual-auth address change.
pub fn get_rotation_delay(env: &Env) -> u64 {
    env.storage().instance().get(&ROT_DELAY_KEY).unwrap_or(0)
}

pub fn set_rotation_delay(env: &Env, seconds: u64) {
    env.storage().instance().set(&ROT_DELAY_KEY, &seconds);
}

pub fn get_pending_rotation(env: &Env, github_username: &String) -> Option<PendingRotation> {
    let key = (PENDING_ROT_KEY, canon(env, github_username));
    let pending: Option<PendingRotation> = env.storage().persistent().get(&key);
    if pending.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
    }
    pending
}

pub fn set_pending_rotation(env: &Env, github_username: &String, rotation: &PendingRotation) {
    let key = (PENDING_ROT_KEY, canon(env, github_username));
    env.storage().persistent().set(&key, rotation);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
}

pub fn remove_pending_rotation(env: &Env, github_username: &String) {
    env.storage()
        .persistent()
        .remove(&(PENDING_ROT_KEY, canon(env, github_username)));
}

pub fn has_pending_rotation(env: &Env, github_username: &String) -> bool {
    env.storage()
        .persistent()
        .has(&(PENDING_ROT_KEY, canon(env, github_username)))
}

// ── Dual-control batch_remove (Issue #219) ───────────────────────────────────
//
// `batch_remove` used to be single-auth: one admin signature could delete up
// to `MAX_WRITE_BATCH` registrations in one invocation. Above a configurable
// size threshold, that single signature is no longer enough — the batch must
// be proposed by one admin-equivalent address and executed by a *different*
// one. Below the threshold, `batch_remove` is untouched.

/// A large `batch_remove` proposed for dual-control execution.
///
/// Created by `propose_batch_remove`, consumed by `execute_batch_remove`
/// (which performs the actual removals), and discardable at any time via
/// `cancel_batch_remove`. Only one proposal may be pending at a time.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct PendingBatchRemove {
    /// The exact usernames to remove once executed.
    pub usernames: Vec<String>,
    /// The admin-equivalent address that proposed the batch. `execute_batch_remove`
    /// rejects a caller equal to this address — dual control requires a
    /// *second* key.
    pub proposed_by: Address,
    /// Ledger timestamp `propose_batch_remove` was called.
    pub proposed_at: u64,
}

/// The configured `batch_remove` dual-control size threshold (Issue #219).
///
/// `0` (the default) disables dual control entirely: every batch, regardless
/// of size, is executed directly by `batch_remove`'s single-admin path —
/// identical to pre-#219 behavior. A non-zero threshold means any
/// `batch_remove` call whose `usernames.len()` exceeds it must instead go
/// through `propose_batch_remove` → `execute_batch_remove`.
#[must_use]
pub fn get_batch_remove_threshold(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&BATCH_REMOVE_THRESHOLD_KEY)
        .unwrap_or(0)
}

pub fn set_batch_remove_threshold(env: &Env, threshold: u32) {
    env.storage()
        .instance()
        .set(&BATCH_REMOVE_THRESHOLD_KEY, &threshold);
}

/// `true` when a batch of `batch_size` usernames must go through the
/// propose/execute dual-control flow instead of `batch_remove` directly.
///
/// Strictly greater-than: a batch exactly *at* the threshold is still
/// "below or at" and unaffected, matching `batch_remove`'s pre-#219 shape
/// check (`size > 0 && size <= max_batch_size`) — only batches *above* the
/// threshold require the second signature.
#[must_use]
pub fn requires_batch_remove_dual_control(env: &Env, batch_size: u32) -> bool {
    let threshold = get_batch_remove_threshold(env);
    threshold > 0 && batch_size > threshold
}

/// The pending large-batch proposal, if one exists — regardless of whether it
/// has expired. `execute_batch_remove` is responsible for treating an expired
/// proposal as gone; this raw getter just reports what is in storage.
#[must_use]
pub fn get_pending_batch_remove(env: &Env) -> Option<PendingBatchRemove> {
    env.storage().instance().get(&PENDING_BATCH_REMOVE_KEY)
}

pub fn set_pending_batch_remove(env: &Env, proposal: &PendingBatchRemove) {
    env.storage()
        .instance()
        .set(&PENDING_BATCH_REMOVE_KEY, proposal);
}

pub fn clear_pending_batch_remove(env: &Env) {
    env.storage().instance().remove(&PENDING_BATCH_REMOVE_KEY);
}

/// `true` once a pending proposal's `BATCH_REMOVE_PROPOSAL_TTL_SECS` window
/// has elapsed. A stale proposal that nobody executed or cancelled should not
/// remain executable indefinitely — the operational context that justified
/// the specific username list may no longer hold.
#[must_use]
pub fn is_batch_remove_proposal_expired(env: &Env, proposal: &PendingBatchRemove) -> bool {
    env.ledger().timestamp()
        >= proposal
            .proposed_at
            .saturating_add(BATCH_REMOVE_PROPOSAL_TTL_SECS)
}

// ── Index repair (admin) ──────────────────────────────────────────────────────

/// Report returned by `repair_index`: what `count` / `verified` currently say
/// on chain versus what a fresh walk of the chunked index and every stored
/// record says they should be.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct RepairReport {
    /// `count` as stored on chain before this call.
    pub stored_count: u32,
    /// `count` recomputed by walking every chunk and counting entries that
    /// still have a live record.
    pub recomputed_count: u32,
    /// `verified` as stored on chain before this call.
    pub stored_verified: u32,
    /// `verified` recomputed the same way, counting only entries whose
    /// record has `verified == true`.
    pub recomputed_verified: u32,
    /// Whether either stored counter disagreed with the recomputed value.
    pub drifted: bool,
    /// Whether the stored counters were overwritten with the recomputed
    /// values by this call. Always `false` when `apply` was `false` (dry
    /// run) or when no drift was found — a clean report never writes, even
    /// with `apply == true`.
    pub applied: bool,
}

/// Recomputes `count` and `verified` from the chunked username index and
/// each entry's stored record, independent of the counters themselves — so
/// it reports correctly even if `count` or `verified` have already drifted.
///
/// With `apply == false` this is a dry run: nothing is written, only a
/// [`RepairReport`] is returned. With `apply == true`, the stored counters
/// are corrected to the recomputed values, but only if drift was actually
/// found. Never touches the flat index, individual records, or the chunked
/// index itself — only the two aggregate counters.
///
/// See `docs/SECURITY.md#index-repair-repair_index` for when an operator
/// should reach for this instead of trusting the live counters.
pub fn repair_index(env: &Env, apply: bool) -> RepairReport {
    let chunk_count = get_chunk_count(env);
    let mut recomputed_count: u32 = 0;
    let mut recomputed_verified: u32 = 0;

    for c in 0..chunk_count {
        let chunk = get_chunk(env, c);
        for i in 0..chunk.len() {
            if let Some(username) = chunk.get(i) {
                if let Some(record) = get_record(env, &username) {
                    recomputed_count = recomputed_count.saturating_add(1);
                    if record.verified {
                        recomputed_verified = recomputed_verified.saturating_add(1);
                    }
                }
            }
        }
    }

    let stored_count = get_count(env);
    let stored_verified = get_verified_count(env);
    let drifted = stored_count != recomputed_count || stored_verified != recomputed_verified;
    let applied = apply && drifted;

    if applied {
        set_count(env, recomputed_count);
        set_verified_count(env, recomputed_verified);
    }

    RepairReport {
        stored_count,
        recomputed_count,
        stored_verified,
        recomputed_verified,
        drifted,
        applied,
    }
}
