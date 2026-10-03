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
pub const PAUSED_KEY: Symbol = symbol_short!("pause");
/// Last `PauseReason` recorded by `pause` / `unpause`.
pub const PAUSE_RSN_KEY: Symbol = symbol_short!("pause_rsn");
/// Consolidated alias for [`PAUSE_RSN_KEY`].
pub const PAUSE_REASON_KEY: Symbol = PAUSE_RSN_KEY;
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

/// Seconds a `set_role` grant must wait before it can be activated (Issue #220).
/// 0 disables the timelock, matching the cooldown convention, so existing
/// deployments keep their instant-grant behaviour until an admin opts in.
pub const ROLE_DELAY_KEY: Symbol = symbol_short!("roldelay");

/// Key prefix for a pending (timelocked) role grant, namespaced by address.
pub const PENDING_ROLE_KEY: Symbol = symbol_short!("pendrole");

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
/// Number of chunk pages in the chunked username index.
///
/// The persisted instance symbol is `"chkcnt"` (Soroban `symbol_short!` limit
/// is 9 bytes). Do **not** introduce a second constant or write `"chunkcnt"`:
/// live instances already store the count under `"chkcnt"` (Issue #468).
pub const CHUNK_CNT_KEY: Symbol = symbol_short!("chkcnt");
/// Monotonic counter bumped every time the flat index's existing positions
/// shift — i.e. on every removal (Issue #215). An opaque pagination cursor
/// embeds the generation at issue time; `decode_cursor` rejects a cursor
/// whose generation no longer matches, rather than silently resuming at a
/// drifted offset.
pub const INDEX_GEN_KEY: Symbol = symbol_short!("idx_gen");
pub const LAST_ACT_KEY: Symbol = symbol_short!("lastact");
/// Ledger sequence containing the most recently emitted contract event (Issue #282).
///
/// This instance value is updated as part of the same invocation that publishes
/// an event. `0` means the deployed instance has not emitted an event yet.
pub const LAST_EVENT_LEDGER_KEY: Symbol = symbol_short!("evtledger");
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

/// A role grant that has been requested but is still inside its timelock
/// window (Issue #220). Held until `activate_role` applies it or
/// `cancel_role_grant` drops it.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct PendingRoleGrant {
    pub role: Role,
    /// Ledger timestamp from which `activate_role` will succeed.
    pub activate_at: u64,
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
    /// Digest of the SBOM produced for this build (Issue #224). `None` when the
    /// operator has not recorded one — including every record written before
    /// this field existed, which is the explicit migration path: call
    /// `set_provenance_digests` once after upgrading to backfill it.
    pub sbom_hash: Option<BytesN<32>>,
    /// Digest of the source tree the WASM was built from (Issue #224).
    pub source_hash: Option<BytesN<32>>,
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

/// Documented layout version for [`ExportPage`] and its records.
/// Breaking changes to the struct layout or field ordering bump this version.
pub const EXPORT_PAGE_LAYOUT_VERSION: u32 = 2;

/// A single exported record tuple: `(github_username, ContributorRecord)`.
pub type ExportRecord = (String, ContributorRecord);

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

/// Write `admin` into instance storage. Used only by `initialize`.
pub fn set_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&ADMIN_KEY, admin);
}

// ── Timelocked role grants (Issue #220) ───────────────────────────────────────

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
        let Some(address) = index.get(i) else {
            continue;
        };
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

#[allow(dead_code)] // Issue #248: staged for consolidated role-gated entry points.
                    // Covered directly by `test_has_role_or_admin_*` tests below.
                    // Do NOT remove: used by future role-consolidation refactor tracked
                    // in the issue backlog. Emergency keys are also covered by this helper.
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
    env.storage().instance().set(&VERIFIER_ALLOWLIST_KEY, list);
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
pub fn run_migration_steps(env: &Env, current: (u32, u32, u32), target: (u32, u32, u32)) -> u32 {
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
        .or_else(|| {
            env.storage()
                .instance()
                .get::<Symbol, u32>(&symbol_short!("p_reason"))
        })
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

/// Returns the ledger sequence containing the latest contract event.
///
/// The value is `0` until the first event is emitted. It intentionally uses
/// instance storage: it is a small, cheap cursor for indexers rather than a
/// historical event log.
#[must_use]
pub fn get_last_event_ledger(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&LAST_EVENT_LEDGER_KEY)
        .unwrap_or(0)
}

/// Records the ledger sequence for an event being emitted in this invocation.
pub fn set_last_event_ledger(env: &Env) {
    env.storage()
        .instance()
        .set(&LAST_EVENT_LEDGER_KEY, &env.ledger().sequence());
}

// ── Issue #248: Direct coverage for has_role_or_admin ────────────────────────
//
// These tests run against the storage helper directly, providing machine-
// checked coverage for every branch so the #[allow(dead_code)] annotation is
// backed by real test execution rather than only by a prose comment.

#[cfg(test)]
mod storage_dead_code_tests {
    use super::*;
    use soroban_sdk::{symbol_short, testutils::Address as _, Address, Env, Symbol};

    fn make_env() -> Env {
        Env::default()
    }

    fn setup_contract(env: &Env) -> Address {
        let contract_id = env.register(crate::TrustBridgeContract, ());
        let admin = Address::generate(env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            set_admin(env, &admin);
        });
        contract_id
    }

    /// The admin address returns `true` regardless of the `expected_role`.
    #[test]
    fn test_has_role_or_admin_admin_always_true() {
        let env = make_env();
        let contract_id = setup_contract(&env);
        let admin = env.as_contract(&contract_id, || get_admin(&env).unwrap());

        env.as_contract(&contract_id, || {
            assert!(
                has_role_or_admin(&env, &admin, Role::Verifier),
                "admin must return true for Verifier role check"
            );
            assert!(
                has_role_or_admin(&env, &admin, Role::Revoker),
                "admin must return true for Revoker role check"
            );
            assert!(
                has_role_or_admin(&env, &admin, Role::Upgrader),
                "admin must return true for Upgrader role check"
            );
        });
    }

    /// An address that holds the exact expected role returns `true`.
    #[test]
    fn test_has_role_or_admin_matching_role_true() {
        let env = make_env();
        let contract_id = setup_contract(&env);
        let verifier = Address::generate(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            set_role(&env, &verifier, Role::Verifier);
            assert!(
                has_role_or_admin(&env, &verifier, Role::Verifier),
                "address with Verifier role must return true for Verifier check"
            );
        });
    }

    /// An address that holds a *different* role returns `false`.
    #[test]
    fn test_has_role_or_admin_wrong_role_false() {
        let env = make_env();
        let contract_id = setup_contract(&env);
        let revoker = Address::generate(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            set_role(&env, &revoker, Role::Revoker);
            assert!(
                !has_role_or_admin(&env, &revoker, Role::Verifier),
                "Revoker must not pass a Verifier role check"
            );
        });
    }

    /// An address with no role at all returns `false`.
    #[test]
    fn test_has_role_or_admin_no_role_false() {
        let env = make_env();
        let contract_id = setup_contract(&env);
        let stranger = Address::generate(&env);

        env.as_contract(&contract_id, || {
            assert!(
                !has_role_or_admin(&env, &stranger, Role::Verifier),
                "address with no role must return false"
            );
        });
    }

    /// Authoritative chunk-count key is `"chkcnt"`, not the longer `"chunkcnt"`
    /// spelling that appeared in rent/footprint docs (Issue #468). Reads and
    /// writes must use [`CHUNK_CNT_KEY`] so a duplicate definition cannot
    /// silently split the persisted counter.
    #[test]
    fn test_chunk_cnt_key_is_chkcnt_not_chunkcnt() {
        let env = make_env();
        let contract_id = setup_contract(&env);

        assert_eq!(
            CHUNK_CNT_KEY,
            symbol_short!("chkcnt"),
            "CHUNK_CNT_KEY must remain the persisted chkcnt symbol"
        );
        assert_ne!(
            CHUNK_CNT_KEY,
            symbol_short!("chunkcnt"),
            "chunkcnt is not a storage key and must not alias CHUNK_CNT_KEY"
        );

        env.as_contract(&contract_id, || {
            set_chunk_count(&env, 7);
            let via_canonical = env
                .storage()
                .instance()
                .get::<Symbol, u32>(&CHUNK_CNT_KEY)
                .unwrap_or(0);
            let via_chkcnt = env
                .storage()
                .instance()
                .get::<Symbol, u32>(&symbol_short!("chkcnt"))
                .unwrap_or(0);
            let via_chunkcnt = env
                .storage()
                .instance()
                .get::<Symbol, u32>(&symbol_short!("chunkcnt"))
                .unwrap_or(0);
            assert_eq!(via_canonical, 7);
            assert_eq!(via_chkcnt, 7, "get_chunk_count must persist under chkcnt");
            assert_eq!(
                via_chunkcnt, 0,
                "chunkcnt must not receive the chunk-count write"
            );
            assert_eq!(get_chunk_count(&env), 7);
        });
    }

    /// An address explicitly assigned `Role::Admin` via RBAC (not the primary
    /// admin key) also returns `true`.
    #[test]
    fn test_has_role_or_admin_rbac_admin_true() {
        let env = make_env();
        let contract_id = setup_contract(&env);
        let rbac_admin = Address::generate(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            set_role(&env, &rbac_admin, Role::Admin);
            assert!(
                has_role_or_admin(&env, &rbac_admin, Role::Revoker),
                "RBAC Admin must return true for any role check"
            );
        });
    }
}

/// Report returned by `repair_index` (Issue #368).
///
/// Contains the stored counter values before any correction and the values
/// recomputed by walking the index, so an operator can review the discrepancy
/// before calling again with `apply = true`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct RepairReport {
    /// Value of `count` as read from instance storage.
    pub stored_total: u32,
    /// Value recomputed by counting every username in the index that has a
    /// stored record.
    pub recomputed_total: u32,
    /// Value of `verified` as read from instance storage.
    pub stored_verified: u32,
    /// Value recomputed by counting every record whose `verified` flag is
    /// `true`.
    pub recomputed_verified: u32,
    /// `true` when either counter differed from its recomputed value.
    /// When `false`, `apply = true` is a no-op.
    pub drifted: bool,
}

/// Recomputes `count` and `verified` by walking the chunked username index and
/// checking each stored record (Issue #368).
///
/// Pass `apply = false` for a dry run — nothing is written. Pass `apply = true`
/// to correct any drift found; a call that finds no drift never writes.
pub fn repair_index(env: &Env, apply: bool) -> RepairReport {
    let index = get_index(env);
    let mut recomputed_total: u32 = 0;
    let mut recomputed_verified: u32 = 0;
    for username in index.iter() {
        if let Some(record) = get_record(env, &username) {
            recomputed_total = recomputed_total.saturating_add(1);
            if record.verified {
                recomputed_verified = recomputed_verified.saturating_add(1);
            }
        }
    }
    let stored_total = get_count(env);
    let stored_verified = get_verified_count(env);
    let drifted = stored_total != recomputed_total || stored_verified != recomputed_verified;
    if apply && drifted {
        set_count(env, recomputed_total);
        set_verified_count(env, recomputed_verified);
    }
    RepairReport {
        stored_total,
        recomputed_total,
        stored_verified,
        recomputed_verified,
        drifted,
    }
}
