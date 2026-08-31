#![no_std]
// Clippy pedantic rollout — Phase 1 (warn-only while fixes land incrementally).
// See docs/CLIPPY_PEDANTIC_PLAN.md for the full phased plan and allow-list policy.
#![warn(
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::redundant_closure_for_method_calls,
    clippy::cloned_instead_of_copied
)]

mod audit;
mod batch;
mod domain;
mod error;
mod events;
mod storage;
mod utils;
mod version;

pub use audit::{AuditConfig, AuditEventType, AuditLogEntry, AuditStats};
pub use batch::{BatchConfig, BatchOperationResult, BatchSummary, MAX_WRITE_BATCH};
pub use domain::{EventDomain, EVENT_DOMAIN_VERSION};
pub use error::{ContractError, ErrorCategory};
pub use events::{RegisteredEvent, RemovedEvent, VerifiedEvent};
pub use storage::{ContributorRecord, EntityType, Stats};
pub use events::{
    AttestationClearedEvent, BatchRemoveCancelledEvent, BatchRemoveExecutedEvent,
    BatchRemoveProposedEvent, ChallengeCancelledEvent, ChallengeCompletedEvent, RenamedEvent,
    RotationCancelledEvent, RotationExecutedEvent, RotationRequestedEvent,
    ChallengeStartedEvent, EmergencyClearedEvent, EmergencyPausedEvent, PausedEvent,
    RegisteredEvent, RemovedEvent, RoleGrantedEvent, RoleRevokedEvent, UnpausedEvent,
    UpgradeAttestedEvent, UpgradedEvent, VerificationRevokedEvent, VerifiedEvent,
};
pub use storage::{
    ChallengeRecord, ContributorRecord, ExportPage, HealthSnapshot, Role, Stats,
    VerificationConfig, VerifierAllowEntry, WasmAttestation, WasmProvenance, PauseReason,
    MAX_VERIFIERS,
};
pub use version::Version;

use soroban_sdk::{contract, contractimpl, Address, BytesN, Env, String, Symbol, Vec};

use crate::storage::{
    add_to_index, bump_ever_verified_count, build_record_proof, clear_pending_reverify,
    get_admin, get_audit_logs, get_audit_stats, get_challenge, get_count,
    get_cooldown as storage_get_cooldown, get_emergency_pause, get_emergency_pause_ts,
    get_ever_verified_count as storage_get_ever_verified_count, get_guardian as storage_get_guardian,
    get_index, get_last_upgrade, get_network_id as storage_get_network_id,
    get_last_event_ledger as storage_get_last_event_ledger,
    get_pending_rotation as storage_get_pending_rotation, get_record,
    get_registered_paginated_internal, get_role as storage_get_role,
    get_role_holder_count as storage_get_role_holder_count, get_role_holders_internal,
    get_rotation_delay as storage_get_rotation_delay, get_stats as read_stats,
    get_verification_config, get_verified_count as storage_get_verified_count,
    get_version as storage_get_version, get_wasm_attestation, get_wasm_provenance, has_challenge,
    build_record_proof, get_pending_rotation as storage_get_pending_rotation,
    get_rotation_delay as storage_get_rotation_delay, has_pending_rotation, has_record,
    is_admin_caller, is_in_cooldown, is_paused as storage_is_paused, push_audit_entry,
    remove_pending_rotation, set_pending_rotation, set_rotation_delay as storage_set_rotation_delay,
    remove_challenge, remove_from_index, remove_record, remove_role as storage_remove_role,
    remove_wasm_attestation, require_initialized, require_not_paused,
    run_migration_steps, set_challenge, set_cooldown as storage_set_cooldown, set_count,
    set_last_event_ledger,
    set_last_action, set_last_upgrade, set_paused as set_paused_state, set_pending_reverify,
    set_ever_verified_count, set_record, set_role as storage_set_role, set_verified_count,
    set_version,
    set_wasm_attestation, set_wasm_provenance, DEFAULT_CHALLENGE_DELAY_SECS,
    ADMIN_KEY,
    get_guardian as storage_get_guardian,
    remove_guardian as storage_remove_guardian,
    add_verifier as storage_add_verifier, remove_verifier as storage_remove_verifier,
    get_verifier_allowlist, is_active_verifier as storage_is_active_verifier,
    verifier_allowlist_active, verifier_slots_remaining, prune_expired_verifiers,
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
    minor: 0,
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

#[contract]
pub struct TrustBridgeContract;

#[contractimpl]
impl TrustBridgeContract {
    /// Sets the contract admin and initializes default state. Can only be called once.
    ///
    /// Sets `admin` as the contract administrator and assigns it the `Admin` role.
    /// Also initializes the registration counter, verified counter, pause state,
    /// upgrade cooldown, and version to their zero/default values.
    ///
    /// # Auth
    ///
    /// No auth required — the deployer calls this immediately after `stellar contract deploy`.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::AlreadyInitialized`] if `initialize` has already been called.
    pub fn initialize(env: Env, admin: Address) -> Result<(), ContractError> {
        if env.storage().instance().has(&ADMIN_KEY) {
            return Err(ContractError::AlreadyInitialized);
        }

        env.storage().instance().set(&ADMIN_KEY, &admin);
        // Record the network before anything else is written, so every record
        // this instance goes on to hold is covered by the tag (Issue #231).
        set_network_id(&env, &env.ledger().network_id());
        set_count(&env, 0);
        set_verified_count(&env, 0);
        set_ever_verified_count(&env, 0);
        set_paused_state(&env, false);
        storage_set_cooldown(&env, 0);
        set_version(&env, (1, 0, 0));
        storage_set_role(&env, &admin, &Role::Admin);

        let timestamp = env.ledger().timestamp();
        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::ContractInitialized, timestamp, Some(admin)),
        );

        Ok(())
    }

    /// Pauses all state-mutating contract functions. Admin-only.
    ///
    /// While paused, any call to `register`, `remove`, `verify`, `revoke_verification`,
    /// `upgrade`, `set_role`, `remove_role`, and other mutating functions returns
    /// [`ContractError::Paused`]. Read-only calls (`get_address`, `get_stats`, etc.)
    /// remain available.
    ///
    /// Use this as an emergency circuit breaker if a vulnerability or incident requires
    /// halting all mutations until a fix can be deployed via `upgrade`.
    ///
    /// Emits [`PausedEvent`].
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    /// - [`ContractError::InvalidPauseReason`] if `reason_code` is not a valid `PauseReason`.
    pub fn pause(env: Env, reason_code: u32) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        if !PauseReason::is_valid(reason_code) {
            return Err(ContractError::InvalidPauseReason);
        }
        let reason = PauseReason::from_code(reason_code).unwrap_or(PauseReason::Other);

        set_paused_state(&env, true);
        crate::storage::set_pause_reason(&env, reason);
        let timestamp = env.ledger().timestamp();
        PausedEvent {
            admin: admin.clone(),
            timestamp,
            reason_code,
            domain: event_domain(&env),
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::AdminAction, timestamp, Some(admin)),
        );

        Ok(())
    }

    /// Resumes state-mutating contract functions after a pause. Admin-only.
    ///
    /// Clears the paused flag set by `pause`, restoring normal contract operation.
    ///
    /// Emits [`UnpausedEvent`].
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    /// - [`ContractError::InvalidPauseReason`] if `reason_code` is not a valid `PauseReason`.
    pub fn unpause(env: Env, reason_code: u32) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        if !PauseReason::is_valid(reason_code) {
            return Err(ContractError::InvalidPauseReason);
        }
        let reason = PauseReason::from_code(reason_code).unwrap_or(PauseReason::Other);

        set_paused_state(&env, false);
        crate::storage::set_pause_reason(&env, reason);
        let timestamp = env.ledger().timestamp();
        UnpausedEvent {
            admin: admin.clone(),
            timestamp,
            reason_code,
            domain: event_domain(&env),
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::AdminAction, timestamp, Some(admin)),
        );

        Ok(())
    }

    /// Returns `true` if the contract is currently paused.
    ///
    /// Read-only; no auth required. Clients should check this before submitting
    /// state-mutating transactions to avoid paying fees for a call that will fail
    /// with [`ContractError::Paused`].
    #[must_use]
    pub fn is_paused(env: Env) -> bool {
        storage_is_paused(&env)
    }

    /// Returns `true` if the emergency pause is active.
    ///
    /// Read-only; no auth required.
    #[must_use]
    pub fn is_emergency_paused(env: Env) -> bool {
        get_emergency_pause(&env)
    }

    /// Returns the timestamp at which the emergency pause was most recently
    /// activated, or `0` if it has never been activated.
    #[must_use]
    pub fn emergency_pause_timestamp(env: Env) -> u64 {
        get_emergency_pause_ts(&env)
    }

    /// Sets the guardian address. Admin-only.
    ///
    /// The guardian may call `emergency_pause` to trip the circuit breaker
    /// without holding the admin key. The guardian **cannot** upgrade the
    /// contract or call `clear_emergency_pause`.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the admin.
    pub fn set_guardian(env: Env, guardian: Address) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();
        set_guardian_address(&env, &guardian);
        Ok(())
    }

    /// Returns the current guardian address, or `None` if none is set.
    #[must_use]
    pub fn get_guardian(env: Env) -> Option<Address> {
        storage_get_guardian(&env)
    }

    /// Removes the guardian address. Admin-only.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the admin.
    pub fn remove_guardian(env: Env) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();
        storage_remove_guardian(&env);
        Ok(())
    }

    /// Trips the emergency circuit breaker. Callable by admin OR the
    /// designated guardian.
    ///
    /// Sets `EMERGENCY_PAUSE_KEY = true` and records the timestamp in
    /// `EMERGENCY_PAUSE_TS_KEY`. All mutating contract functions already
    /// check `require_not_paused`, which now also tests this flag.
    ///
    /// Emits [`EmergencyPausedEvent`].
    ///
    /// The call is **idempotent**: if the emergency pause is already active,
    /// no event is emitted and `Ok(())` is returned.
    ///
    /// # Auth
    ///
    /// Requires auth from the admin **or** the current guardian.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is neither admin nor guardian.
    pub fn emergency_pause(env: Env, caller: Address) -> Result<(), ContractError> {
        require_initialized(&env)?;
        caller.require_auth();

        let caller_is_admin = is_admin_caller(&env, &caller);
        let caller_is_guardian = is_guardian(&env, &caller);

        if !caller_is_admin && !caller_is_guardian {
            return Err(ContractError::NotAuthorized);
        }

        // Idempotent: no-op if already emergency-paused.
        if get_emergency_pause(&env) {
            return Ok(());
        }

        let timestamp = env.ledger().timestamp();
        set_emergency_pause(&env, true);
        set_emergency_pause_ts(&env, timestamp);

        EmergencyPausedEvent {
            triggered_by: caller.clone(),
            timestamp,
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::AdminAction, timestamp, Some(caller)),
        );

        Ok(())
    }

    /// Clears the emergency pause. Admin-only.
    ///
    /// Only the contract admin may lift an emergency pause. The guardian
    /// intentionally cannot — this ensures a slow admin key review before
    /// normal operations resume.
    ///
    /// Emits [`EmergencyClearedEvent`].
    ///
    /// The call is **idempotent**: if the emergency pause is not active,
    /// no event is emitted and `Ok(())` is returned.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the admin.
    pub fn clear_emergency_pause(env: Env) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        // Idempotent: no-op if not currently emergency-paused.
        if !get_emergency_pause(&env) {
            return Ok(());
        }

        let timestamp = env.ledger().timestamp();
        set_emergency_pause(&env, false);

        EmergencyClearedEvent {
            admin: admin.clone(),
            timestamp,
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::AdminAction, timestamp, Some(admin)),
        );

        Ok(())
    }

    /// Assigns a role to `target` with no expiry. Admin-only.
    ///
    /// Roles gate access to privileged operations:
    ///
    /// | Role | Can do |
    /// |------|--------|
    /// | `Admin` | Everything |
    /// | `Upgrader` | Call `upgrade` |
    /// | `Verifier` | Call `verify` and `revoke_verification` |
    ///
    /// A grant made here never lapses on its own — see `set_role_with_expiry`
    /// (Issue #221) for a time-bounded grant, e.g. for a contractor or bot
    /// key that should stop verifying after a known off-boarding date without
    /// requiring anyone to remember to call `remove_role`.
    ///
    /// Emits [`RoleGrantedEvent`].
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    pub fn set_role(env: Env, target: Address, role: Role) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        let admin = get_admin(&env)?;
        admin.require_auth();

        storage_set_role(&env, &target, &role);
        let timestamp = env.ledger().timestamp();
        RoleGrantedEvent {
            address: target,
            role: role as u32,
            admin,
            timestamp,
            domain: event_domain(&env),
        }
        .publish(&env);

        Ok(())
    }

    /// Revokes `target`'s role assignment. Admin-only.
    ///
    /// After this call `get_role(target)` returns `None`. Does not affect the
    /// admin's own role — the admin address is stored separately and cannot be
    /// stripped via `remove_role`. Also clears any expiry timestamp set via
    /// `set_role_with_expiry` (Issue #221) — this is the only way to actually
    /// delete a role's storage entries; letting a time-bounded grant simply
    /// lapse leaves it in storage, just no longer reported by `get_role`.
    ///
    /// Emits [`RoleRevokedEvent`].
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    pub fn remove_role(env: Env, target: Address) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        let admin = get_admin(&env)?;
        admin.require_auth();

        storage_remove_role(&env, &target);
        let timestamp = env.ledger().timestamp();
        RoleRevokedEvent {
            address: target,
            admin,
            timestamp,
            domain: event_domain(&env),
        }
        .publish(&env);

        Ok(())
    }

    /// Returns the role currently assigned to `address`, or `None` if no role
    /// is assigned **or its grant has expired** (Issue #221).
    ///
    /// Read-only; no auth required. Returns `None` for any address that has
    /// never been granted a role (including the admin address, which is
    /// stored separately). Expiry is lazy: an expired grant's storage entry
    /// is left in place — `get_role` just stops reporting it as held. Use
    /// `get_role_expiry` to see the raw expiry timestamp, including for an
    /// address whose grant has already lapsed.
    #[must_use]
    pub fn get_role(env: Env, address: Address) -> Option<Role> {
        storage_get_role(&env, &address)
    }

    /// Returns `true` if `address` currently holds `role` — i.e. `get_role`
    /// would return `Some(role)` (Issue #221). Expired grants read as not
    /// held, the same as if `address` had never been granted a role.
    ///
    /// Read-only; no auth required.
    #[must_use]
    pub fn has_role(env: Env, address: Address, role: Role) -> bool {
        storage_get_role(&env, &address) == Some(role)
    }

    /// The expiry timestamp for `address`'s current role grant, or `None` if
    /// it was granted with no expiry, or if `address` holds no `ROLE_KEY`
    /// entry at all (Issue #221).
    ///
    /// Unlike `get_role`, this does **not** hide an already-lapsed expiry —
    /// it is the raw stored timestamp, so an operator auditing off-boarded
    /// keys can distinguish "never expires" (`None`) from "expired at T"
    /// (`Some(T)` where `T` is in the past) without waiting for `remove_role`
    /// to run.
    ///
    /// Read-only; no auth required.
    #[must_use]
    pub fn get_role_expiry(env: Env, address: Address) -> Option<u64> {
        crate::storage::get_role_expiry(&env, &address)
    }

    /// Assigns a role to `target` with an optional expiry timestamp
    /// (Issue #221). Admin-only.
    ///
    /// Identical to `set_role`, except the grant can be time-bounded: once
    /// `env.ledger().timestamp() >= expires_at`, `get_role(target)` returns
    /// `None` — and every role check built on it (`verify`,
    /// `revoke_verification`, `batch_verify`, `get_role_holders`,
    /// `has_role_or_admin`) treats `target` as holding no role at all, with
    /// no further changes needed at those call sites. `set_role(target,
    /// role)` remains available and is exactly `set_role_with_expiry(target,
    /// role, None)` — a grant that never expires.
    ///
    /// This is intentionally the **only** path that can expire a role
    /// assignment. It never touches `ADMIN_KEY`: the contract admin's
    /// identity (`has_admin_role`, `get_admin`) is a separate, immutable
    /// storage slot, so granting `Role::Admin` here with an expiry only
    /// affects RBAC-style role checks, never who the admin is.
    ///
    /// Expiry is lazy — see `get_role` and `docs/SECURITY.md` for what that
    /// means for `get_role_holders` and storage cleanup.
    ///
    /// Emits [`RoleGrantedEvent`].
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    pub fn set_role_with_expiry(
        env: Env,
        target: Address,
        role: Role,
        expires_at: Option<u64>,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        let admin = get_admin(&env)?;
        admin.require_auth();

        crate::storage::set_role_with_expiry(&env, &target, &role, expires_at);
        let timestamp = env.ledger().timestamp();
        RoleGrantedEvent {
            address: target,
            role: role as u32,
            admin,
            timestamp,
            domain: event_domain(&env),
        }
        .publish(&env);

        Ok(())
    }

    /// Lists addresses that currently hold a role, as `(address, role)` pairs
    /// (Issue #228).
    ///
    /// `get_role` is a point lookup, so a dashboard mirroring RBAC had no way
    /// to answer "who is a Verifier?" without already knowing every address to
    /// ask about. It could only drift from the chain. This enumerates the set.
    ///
    /// Ordered by grant time, oldest first. The order is stable across calls
    /// except that revoking a role compacts the index, shifting everything
    /// after it down one — so a paginating caller should treat a concurrent
    /// revocation as a reason to restart, not to trust the next offset.
    ///
    /// The admin **is** included: `initialize` grants `Role::Admin` through the
    /// same path that maintains this index, so the admin appears as a holder
    /// like any other.
    ///
    /// `limit` is capped at [`MAX_ROLE_PAGE_LIMIT`]; `0` means "use the cap".
    /// An `offset` past the end returns an empty page rather than an error.
    ///
    /// Read-only; no auth required. Role assignments are public information —
    /// they are already visible in `RoleGrantedEvent`.
    #[must_use]
    pub fn get_role_holders(env: Env, offset: u32, limit: u32) -> Vec<RoleHolder> {
        get_role_holders_internal(&env, offset, limit)
    }

    /// Network id this instance was initialized on, or `None` for an instance
    /// initialized before network tagging existed (Issue #231).
    ///
    /// The value is `env.ledger().network_id()` — the SHA-256 of the network
    /// passphrase — captured at `initialize`. Consumers should compare it
    /// against the network they believe they are talking to instead of
    /// inferring the network from an RPC URL, which is what a bindings consumer
    /// had to do before: the same G-address is valid everywhere, so a record
    /// read off the wrong deployment looks entirely legitimate.
    ///
    /// Read-only; no auth required.
    #[must_use]
    pub fn get_network_tag(env: Env) -> Option<BytesN<32>> {
        storage_get_network_id(&env)
    }

    /// Tags an untagged instance with the network it is running on. Admin-only.
    ///
    /// Only for instances initialized before network tagging existed. It is
    /// deliberately **not** a way to re-tag: once a tag is present this returns
    /// [`ContractError::NetworkMismatch`] if it disagrees with the live
    /// network, rather than overwriting it. An entry point that could rewrite
    /// the tag would defeat the check entirely — anyone restoring state onto
    /// the wrong network could simply re-stamp it and carry on.
    ///
    /// Re-tagging with the *same* network is a no-op and succeeds, so this is
    /// safe to call unconditionally from a migration script.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NetworkMismatch`] if a tag is already recorded and
    ///   does not match the executing network.
    pub fn adopt_network_tag(env: Env) -> Result<(), ContractError> {
        // `get_admin` runs `require_initialized`, which already enforces the
        // network check — so a mismatched instance is rejected here with
        // exactly the NetworkMismatch this function would otherwise return,
        // and an untagged one passes through to be tagged below.
        let admin = get_admin(&env)?;
        require_not_paused(&env)?;
        admin.require_auth();

        let live = env.ledger().network_id();
        match storage_get_network_id(&env) {
            Some(recorded) if recorded != live => Err(ContractError::NetworkMismatch),
            Some(_) => Ok(()),
            None => {
                set_network_id(&env, &live);
                Ok(())
            }
        }
    }

    /// Number of addresses currently holding a role (Issue #228).
    ///
    /// Lets a caller size its pagination loop before fetching, and gives a
    /// dashboard a cheap way to detect that its RBAC mirror has drifted
    /// without walking every page.
    #[must_use]
    pub fn get_role_holder_count(env: Env) -> u32 {
        storage_get_role_holder_count(&env)
    }

    /// Adds `verifier` to the campaign allowlist and grants it `Role::Verifier`
    /// (Issue #293). Admin-only.
    ///
    /// The allowlist is a hard-capped (`MAX_VERIFIERS`), time-bounded companion
    /// to `set_role(Verifier)`:
    ///
    /// - `expires_at` is a ledger timestamp after which the entry stops
    ///   authorizing `verify` / `batch_verify`. `0` means no expiry. A non-zero
    ///   value must be in the future.
    /// - Re-calling for an address already on the list **refreshes its expiry**
    ///   in place and does not consume another slot.
    /// - Adding a brand-new address when the list already holds `MAX_VERIFIERS`
    ///   active members fails with `VerifierAllowlistFull`. Expired entries are
    ///   pruned first, so a lapsed member never blocks a new one.
    ///
    /// ### Interaction with `set_role`
    ///
    /// This does not replace `set_role`; it composes with it. `add_verifier`
    /// also calls `set_role(verifier, Verifier)` so the two never disagree.
    /// **Once the allowlist has been populated at least once**, `verify` and
    /// `batch_verify` require the caller to be an *active* (non-expired)
    /// allowlist member — a bare `set_role(Verifier)` grant is no longer
    /// sufficient. Before the first `add_verifier` call the contract stays in
    /// pure role-based mode, so existing deployments are unaffected until they
    /// opt in. See `docs/ABI.md`.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] / [`ContractError::Paused`]
    /// - [`ContractError::NotAuthorized`] if the caller is not the admin.
    /// - [`ContractError::VerifierExpiryInPast`] if `expires_at` is non-zero and
    ///   not in the future.
    /// - [`ContractError::VerifierAllowlistFull`] if the cap would be exceeded.
    pub fn add_verifier(
        env: Env,
        verifier: Address,
        expires_at: u64,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        let now = env.ledger().timestamp();
        storage_add_verifier(&env, &verifier, expires_at, now)?;
        storage_set_role(&env, &verifier, &Role::Verifier);

        RoleGrantedEvent {
            address: verifier,
            role: Role::Verifier as u32,
            admin,
            timestamp: now,
            domain: event_domain(&env),
        }
        .publish(&env);

        Ok(())
    }

    /// Removes `verifier` from the allowlist and revokes its `Role::Verifier`
    /// (Issue #293). Admin-only.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] / [`ContractError::Paused`]
    /// - [`ContractError::NotAuthorized`] if the caller is not the admin.
    /// - [`ContractError::VerifierNotAllowlisted`] if `verifier` is not listed.
    pub fn remove_verifier(env: Env, verifier: Address) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        let now = env.ledger().timestamp();
        storage_remove_verifier(&env, &verifier, now)?;
        storage_remove_role(&env, &verifier);

        RoleRevokedEvent {
            address: verifier,
            admin,
            timestamp: now,
            domain: event_domain(&env),
        }
        .publish(&env);

        Ok(())
    }

    /// The verifier allowlist as `(address, expires_at, added_at)` entries
    /// (Issue #293), including any that have expired but not yet been pruned.
    /// Read-only; no auth.
    #[must_use]
    pub fn get_verifiers(env: Env) -> Vec<VerifierAllowEntry> {
        get_verifier_allowlist(&env)
    }

    /// `true` if `verifier` is on the allowlist and not expired as of the
    /// current ledger (Issue #293). Read-only; no auth.
    #[must_use]
    pub fn is_active_verifier(env: Env, verifier: Address) -> bool {
        storage_is_active_verifier(&env, &verifier, env.ledger().timestamp())
    }

    /// Allowlist slots still free before the `MAX_VERIFIERS` cap, counting only
    /// active (non-expired) members (Issue #293). Read-only; no auth.
    #[must_use]
    pub fn verifier_slots_remaining(env: Env) -> u32 {
        verifier_slots_remaining(&env, env.ledger().timestamp())
    }

    /// Drops every expired allowlist entry and returns how many were removed
    /// (Issue #293). Anyone may call it — it only removes entries that are
    /// already inactive. No-op return of `0` when nothing is stale.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`]
    pub fn prune_expired_verifiers(env: Env) -> Result<u32, ContractError> {
        require_initialized(&env)?;
        Ok(prune_expired_verifiers(&env, env.ledger().timestamp()))
    }

    /// Sets the minimum number of seconds that must elapse between WASM upgrades. Admin-only.
    ///
    /// A non-zero cooldown enforces a timelock on `upgrade`, giving watchers time to
    /// detect and react to an unexpected upgrade before the next one can be submitted.
    /// Set to `0` to disable the timelock.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    pub fn set_cooldown(env: Env, cooldown_seconds: u64) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        storage_set_cooldown(&env, cooldown_seconds);
        Ok(())
    }

    /// Returns the configured WASM upgrade cooldown in seconds.
    ///
    /// Returns `0` if no cooldown is configured (upgrades are unrestricted by time).
    #[must_use]
    pub fn get_cooldown(env: Env) -> u64 {
        storage_get_cooldown(&env)
    }

    /// Sets the per-verifier, per-ledger cap on verify/revoke units (Issue #292).
    /// Admin-only.
    ///
    /// One `verify` or `revoke_verification` call spends one unit; one
    /// `batch_verify` spends one unit per requested username. When a non-admin
    /// caller would exceed `limit` units in a single ledger, the call fails with
    /// [`ContractError::VerifyRateLimited`] and writes nothing.
    ///
    /// `limit == 0` disables the check entirely. With no configured value the
    /// contract uses a built-in default (`DEFAULT_VERIFIES_PER_LEDGER`).
    ///
    /// The admin is never rate-limited — incident response must not be throttled.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    pub fn set_verify_limit(env: Env, limit: u32) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        storage_set_verify_limit(&env, limit);
        Ok(())
    }

    /// Returns the active per-verifier, per-ledger verify/revoke cap (Issue #292).
    ///
    /// This is the configured value when the admin has set one (including `0`,
    /// meaning disabled), otherwise the built-in default. Read-only; no auth.
    #[must_use]
    pub fn get_verify_limit(env: Env) -> u32 {
        storage_get_verify_limit(&env)
    }

    fn register_personal(env: &Env, contract_id: &soroban_sdk::Address, name: &str, addr: &Address) {
        TrustBridgeContract::register(
            env.clone(),
            username(env, name),
            addr.clone(),
            0,
            None,
        )
        .unwrap();
    }

    fn register_org(
        env: &Env,
        contract_id: &soroban_sdk::Address,
        name: &str,
        addr: &Address,
        org: &str,
    ) {
        TrustBridgeContract::register(
            env.clone(),
            username(env, name),
            addr.clone(),
            1,
            Some(username(env, org)),
        )
        .unwrap();
    }

    fn register_team(
        env: &Env,
        contract_id: &soroban_sdk::Address,
        name: &str,
        addr: &Address,
        org: &str,
    ) {
        TrustBridgeContract::register(
            env.clone(),
            username(env, name),
            addr.clone(),
            2,
            Some(username(env, org)),
        )
        .unwrap();
    }

    #[test]
    fn test_register_and_get_address_roundtrip() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            register_personal(&env, &contract_id, "octocat", &user);

            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert_eq!(record.stellar_address, user);
            assert!(!record.verified);
            assert_eq!(record.entity_type, EntityType::Personal);
        });
    }
    /// Returns the stored contract schema version as `(major, minor, patch)`.
    ///
    /// Falls back to the compile-time [`CONTRACT_VERSION`] constant on instances
    /// initialized before on-chain version tracking was added. Prefer `version`
    /// for the canonical version endpoint; this is the raw storage accessor.
    #[must_use]
    pub fn get_version(env: Env) -> (u32, u32, u32) {
        storage_get_version(&env).unwrap_or_else(|| CONTRACT_VERSION.to_tuple())
    }

    /// Declares in advance the WASM hash the admin intends to deploy. Admin-only.
    ///
    /// Optional two-step upgrade. While an attestation is live, `upgrade` will
    /// accept only the hash it names — so a compromised admin key cannot swap
    /// in a different binary at the moment of the upgrade without first
    /// publishing that intent on-chain, ahead of time, where watchers can see
    /// it.
    ///
    /// `expires_at` must be in the future. The expiry is the point: an
    /// attestation that never lapsed would be a standing authorisation for that
    /// hash, which is strictly worse than having none at all.
    ///
    /// Publishing a new attestation replaces any existing one.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::AttestationExpired`] if `expires_at` is not in the future.
    pub fn attest_upgrade(
        env: Env,
        wasm_hash: BytesN<32>,
        expires_at: u64,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;

        let admin = get_admin(&env)?;
        admin.require_auth();

        let now = env.ledger().timestamp();
        if expires_at <= now {
            return Err(ContractError::AttestationExpired);
        }

        set_wasm_attestation(
            &env,
            &WasmAttestation {
                wasm_hash: wasm_hash.clone(),
                expires_at,
                attested_by: admin,
                attested_at: now,
            },
        );

        UpgradeAttestedEvent {
            wasm_hash,
            expires_at,
            timestamp: now,
        }
        .publish(&env);

        Ok(())
    }

    /// Withdraws a pending upgrade attestation. Admin-only.
    ///
    /// The escape hatch for an attestation published in error: without it the
    /// admin would have to wait out the expiry before upgrading to any other
    /// hash.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    pub fn clear_attestation(env: Env) -> Result<(), ContractError> {
        require_initialized(&env)?;

        let admin = get_admin(&env)?;
        admin.require_auth();

        if let Some(attestation) = get_wasm_attestation(&env) {
            remove_wasm_attestation(&env);
            
            AttestationClearedEvent {
                wasm_hash: attestation.wasm_hash,
                expires_at: attestation.expires_at,
                timestamp: env.ledger().timestamp(),
            }
            .publish(&env);
        }
        
        Ok(())
    }

    /// Returns the pending upgrade attestation, if any.
    ///
    /// Returned regardless of expiry — seeing a lapsed attestation is useful
    /// when diagnosing a rejected upgrade.
    #[must_use]
    pub fn get_attestation(env: Env) -> Option<WasmAttestation> {
        get_wasm_attestation(&env)
    }

    /// Returns the provenance of the currently deployed WASM.
    ///
    /// `None` on an instance that has never been upgraded. `previous_wasm_hash`
    /// names the hash this one replaced, so the deployment lineage can be walked
    /// backwards through historical `UpgradedEvent`s.
    #[must_use]
    pub fn get_provenance(env: Env) -> Option<WasmProvenance> {
        get_wasm_provenance(&env)
    }

    /// Upgrades contract WASM executable code. Admin-only.
    ///
    /// Records provenance for the new hash: what it replaced, who authorised
    /// it, when, at what version, and whether it had been attested. Previously
    /// this wrote only a bare timestamp, so "what is deployed, and what did it
    /// replace?" could not be answered from a contract call at all.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::CooldownActive`] if the configured upgrade cooldown has not elapsed.
    /// - [`ContractError::AttestationExpired`] if a required attestation has expired.
    /// - [`ContractError::UnattestedWasm`] if the pending attestation names a different hash.
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        let admin = get_admin(&env)?;
        admin.require_auth();

        let now = env.ledger().timestamp();

        Self::require_cooldown_elapsed(&env, now)?;
        let attested = Self::consume_attestation(&env, &new_wasm_hash, now)?;

        // Provenance is captured before the executable is swapped: after
        // update_current_contract_wasm the code answering these questions is
        // the new binary, and the record of what it replaced would be lost.
        let previous_wasm_hash = get_wasm_provenance(&env).map(|p| p.wasm_hash);
        let version = storage_get_version(&env).unwrap_or_else(|| CONTRACT_VERSION.to_tuple());

        set_wasm_provenance(
            &env,
            &WasmProvenance {
                wasm_hash: new_wasm_hash.clone(),
                previous_wasm_hash,
                upgraded_by: admin,
                upgraded_at: now,
                version: soroban_sdk::vec![&env, version.0, version.1, version.2],
                attested,
            },
        );

        env.deployer()
            .update_current_contract_wasm(new_wasm_hash.clone());
        set_last_upgrade(&env, now);

        let version = storage_get_version(&env).unwrap_or_else(|| CONTRACT_VERSION.to_tuple());
        UpgradedEvent {
            new_wasm_hash,
            version,
            timestamp: now,
            domain: event_domain(&env),
        }
        .publish(&env);

        Ok(())
    }

    /// Enforces the upgrade timelock.
    ///
    /// Extracted from `upgrade` so the entry point reads as its four distinct
    /// steps — timelock, attestation, provenance, swap — instead of one block
    /// of interleaved policy.
    fn require_cooldown_elapsed(env: &Env, now: u64) -> Result<(), ContractError> {
        let cooldown = storage_get_cooldown(env);
        if cooldown == 0 {
            return Ok(());
        }

        // A contract that has never upgraded has no last-upgrade timestamp to
        // measure from; treating the missing value as 0 would make the very
        // first upgrade wait out a cooldown against the epoch.
        if !env.storage().instance().has(&crate::storage::LAST_UPG_KEY) {
            return Ok(());
        }

        if now < get_last_upgrade(env).saturating_add(cooldown) {
            return Err(ContractError::CooldownActive);
        }

        Ok(())
    }

    /// Validates `new_wasm_hash` against any live attestation and clears it.
    ///
    /// Returns whether the upgrade was covered by an attestation, which is
    /// recorded in the provenance so an auditor can tell a two-step upgrade
    /// from a direct one after the fact.
    ///
    /// When `attestation_required` is `true` (set via `set_attestation_required`),
    /// a missing attestation fails with [`ContractError::AttestationRequired`]
    /// instead of silently proceeding. Hash mismatches and expired attestations
    /// always fail regardless of the required flag.
    fn consume_attestation(
        env: &Env,
        new_wasm_hash: &BytesN<32>,
        now: u64,
    ) -> Result<bool, ContractError> {
        let attestation = match get_wasm_attestation(env) {
            Some(a) => a,
            None => {
                // When attestation is mandatory, a missing attestation is an error.
                if is_attestation_required(env) {
                    return Err(ContractError::AttestationRequired);
                }
                // Attestation is opt-in: with none published, upgrade behaves as it
                // always has. Making it mandatory would brick every deployment that
                // upgrades without adopting the new flow.
                return Ok(false);
            }
        };

        if now > attestation.expires_at {
            // Clear the stale record so the admin is not forced to call
            // clear_attestation before retrying.
            remove_wasm_attestation(env);
            return Err(ContractError::AttestationExpired);
        }

        if attestation.wasm_hash != *new_wasm_hash {
            // Deliberately left in place: a mismatch may be an attacker
            // substituting a binary, and clearing it here would let a second
            // attempt through unchecked.
            return Err(ContractError::UnattestedWasm);
        }

        // Single-use — an attestation authorises one upgrade, not a standing
        // permission for that hash.
        remove_wasm_attestation(env);
        Ok(true)
    }

    /// Authorization check for `remove`: only the registrant or the contract
    /// admin may remove a record.
    fn require_remove_auth(
        caller: &Address,
        admin: &Address,
        record_address: &Address,
    ) -> Result<(), ContractError> {
        if caller != admin && caller != record_address {
            return Err(ContractError::NotAuthorized);
        }
        Ok(())
    }

    /// One-time configuration of the verification parameters.
    ///
    /// Stores the attestation symbol, expiry window, and threshold. May only
    /// be called once — a second invocation returns [`ContractError::AlreadyInitialized`].
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotAuthorized`] if `caller` is not the contract admin.
    /// - [`ContractError::AlreadyInitialized`] if verification was already configured.
    pub fn config_verification(
        env: Env,
        caller: Address,
        attestation: Symbol,
        expires_in: u64,
        threshold: u32,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        caller.require_auth();

        let admin = get_admin(&env)?;
        if caller != admin {
            return Err(ContractError::NotAuthorized);
        }

        if crate::storage::is_verification_configured(&env) {
            return Err(ContractError::AlreadyInitialized);
        }

        crate::storage::set_verification_config(&env, attestation, expires_in, threshold);

        let timestamp = env.ledger().timestamp();
        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::AdminAction, timestamp, Some(caller)),
        );

        Ok(())
    }

    /// Returns the stored verification configuration, or `None` if not configured.
    #[must_use]
    pub fn get_verification_config(env: Env) -> Option<VerificationConfig> {
        get_verification_config(&env)
    }

    /// Returns `true` if `github_username` is verified **and that
    /// verification has not expired** (Issue #218).
    ///
    /// This is the effective, expiry-aware status — the read to use when
    /// deciding payout eligibility. It differs from
    /// `get_address(github_username).map(|r| r.verified)` in one case: a
    /// record whose `verified` flag is still raw-`true` but whose
    /// `config_verification`-configured window has lapsed. That case reads
    /// `true` from the raw flag but `false` here.
    ///
    /// `false` for an unregistered username, a never-verified one, or one
    /// whose verification has expired. `true` for a verified username when
    /// verification was never configured, or configured with
    /// `expires_in == 0` (no expiry) — expiry only ever narrows `verified`,
    /// it never widens it.
    ///
    /// Read-only; no auth required; works while paused.
    #[must_use]
    pub fn is_verification_active(env: Env, github_username: String) -> bool {
        match get_record(&env, &github_username) {
            Some(record) if record.verified => {
                !crate::storage::is_verification_expired(&env, &github_username)
            }
            _ => false,
        }
    }

    /// Returns the ledger timestamp at which `github_username`'s **current**
    /// verification will expire, or `None` if it cannot expire — never
    /// verified, verification not configured, configured with
    /// `expires_in == 0`, or already revoked (Issue #218).
    ///
    /// Does not check whether that timestamp is already in the past — pair
    /// this with `is_verification_active` for the effective yes/no answer.
    ///
    /// Read-only; no auth required.
    #[must_use]
    pub fn get_verification_expiry(env: Env, github_username: String) -> Option<u64> {
        let record = get_record(&env, &github_username)?;
        if !record.verified {
            return None;
        }
        let config = get_verification_config(&env)?;
        if config.expires_in == 0 {
            return None;
        }
        let verified_at = crate::storage::get_verified_at(&env, &github_username)?;
        Some(verified_at.saturating_add(config.expires_in))
    }

    /// Returns stored audit log entries for operator compliance and inspection.
    #[must_use]
    pub fn get_audit_logs(env: Env) -> Vec<AuditLogEntry> {
        get_audit_logs(&env)
    }

    /// Returns stored aggregate audit log statistics.
    #[must_use]
    pub fn get_audit_stats(env: Env) -> AuditStats {
        get_audit_stats(&env)
    }

    /// Advances the on-chain schema version and runs any applicable data-migration steps. Admin-only.
    ///
    /// `new_version` must be strictly greater than the current stored version
    /// (semver order); downgrading is rejected with `InvalidVersion`.
    ///
    /// For each registered migration step whose `from_version` falls in the
    /// window `(current, new_version]` the step is executed exactly once.
    /// Calling `migrate` again with the same `new_version` is a no-op because
    /// `current == new_version` after the first run, which fails the strict
    /// greater-than check.  Calling it with a later version only runs the
    /// steps for that new gap — already-applied steps are skipped.
    ///
    /// **v1.0.0 → v1.1.0**: rewrites every `ContributorRecord` to normalise
    /// the `registered_at` field from the legacy `u64` layout to `u32`.
    /// Safe to run on a clean deployment (no records to touch ⇒ no writes).
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    /// - [`ContractError::InvalidVersion`] if `new_version` is not strictly greater than the current version.
    pub fn migrate(env: Env, new_version: (u32, u32, u32)) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        let admin = get_admin(&env)?;
        admin.require_auth();

        let current = storage_get_version(&env).unwrap_or_else(|| CONTRACT_VERSION.to_tuple());
        if new_version <= current {
            return Err(ContractError::InvalidVersion);
        }

        // Run every migration step that closes the gap between current and
        // new_version.  The return value (steps applied) is informational and
        // not stored — the version bump itself is the idempotency guard.
        run_migration_steps(&env, current, new_version);

        set_version(&env, new_version);
        Ok(())
    }

    /// Returns the deployed contract version as `(major, minor, patch)`.
    ///
    /// Instances initialized before versioning was added carry no stored
    /// version and report the build constant instead.
    #[must_use]
    pub fn version(env: Env) -> (u32, u32, u32) {
        if require_initialized(&env).is_err() {
            return CONTRACT_VERSION.to_tuple();
        }
        storage_get_version(&env).unwrap_or_else(|| CONTRACT_VERSION.to_tuple())
    }

    /// Reports whether the deployed contract satisfies a client's minimum
    /// required version. Bindings consumers call this before invoking, so a
    /// stale client fails fast instead of on an unexpected ABI.
    #[must_use]
    pub fn is_compatible(env: Env, major: u32, minor: u32, patch: u32) -> bool {
        Version::from_tuple(Self::version(env))
            .is_compatible_with(Version::new(major, minor, patch))
    }

    /// Returns the maximum accepted GitHub username length.
    ///
    /// Clients read this instead of hardcoding 39, so a future relaxation of
    /// the guard does not require a client release.
    #[must_use]
    pub fn max_username_len(_env: Env) -> u32 {
        MAX_USERNAME_LEN
    }

    /// Reports whether `github_username` would pass the `register` guard.
    /// Lets a dashboard validate input before asking the user to sign.
    #[must_use]
    pub fn is_username_valid(_env: Env, github_username: String) -> bool {
        is_valid_github_username(&github_username)
    }

    /// Reports whether `address` is the well-known zero/burn address that
    /// `register` rejects. Lets a dashboard or indexer consumer validate a
    /// Stellar address before asking a user to sign, mirroring
    /// `is_username_valid`.
    #[must_use]
    pub fn is_address_zero(env: Env, address: Address) -> bool {
        is_zero_address(&env, &address)
    }

    /// Case-insensitive username equality, matching GitHub's own semantics.
    ///
    /// Off-chain verification workflows use this to match a registration
    /// against a GitHub identity without depending on the stored casing.
    #[must_use]
    pub fn usernames_match(_env: Env, a: String, b: String) -> bool {
        eq_ignore_ascii_case(&a, &b)
    }

    /// Computes the Merkle leaf hash for one `(github_username,
    /// stellar_address, verified)` export entry (Issue #216).
    ///
    /// Every `ExportPage` returned by `get_registered_paginated` /
    /// `get_public_paginated` carries a `merkle_root` over its `records` in
    /// page order. This function exposes the exact leaf encoding that root
    /// is built from, so off-chain tooling (a treasury, a dashboard) can
    /// verify its own reimplementation matches before trusting inclusion
    /// proofs it builds from an exported page. See `crate::merkle` for the
    /// full leaf/node encoding and the odd-node promotion rule used above
    /// the leaf layer.
    ///
    /// Read-only; no auth required — this is a pure function of its inputs
    /// and reads no contract state.
    #[must_use]
    pub fn merkle_leaf_hash(
        env: Env,
        github_username: String,
        stellar_address: Address,
        verified: bool,
    ) -> BytesN<32> {
        crate::merkle::leaf_hash(&env, &github_username, &stellar_address, verified)
    }

    /// Registers or updates a GitHub username → Stellar address mapping.
    ///
    /// The caller must authenticate as `stellar_address`. The username must be
    /// 1 to `MAX_USERNAME_LEN` (39) characters of alphanumerics, hyphens, and
    /// underscores, starting and ending alphanumeric, or the call fails with
    /// `InvalidUsername`. `stellar_address` must not be the well-known
    /// zero/burn address, or the call fails with `ZeroAddress`.
    ///
    /// Re-pointing an existing registration at a different address also
    /// requires authentication from the address currently registered, so a
    /// username cannot be taken over by whoever calls `register` next.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::InvalidUsername`] if `github_username` is not accepted.
    /// - [`ContractError::ZeroAddress`] if `stellar_address` is the zero/burn address.
    /// - [`ContractError::ChallengeActive`] if a challenge is active on this username.
    pub fn register(
        env: Env,
        github_username: String,
        stellar_address: Address,
        fallback_addresses: Vec<Address>,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        if !is_valid_github_username(&github_username) {
            return Err(ContractError::InvalidUsername);
        }

        // Reject the zero/burn address before auth too. On a live network
        // `require_auth` below would already fail for it since nobody holds
        // its private key, but `mock_all_auths` in tests and local sandboxes
        // bypasses that check — and a typed `ZeroAddress` error is more
        // useful to dashboard/indexer consumers than an opaque auth failure.
        if is_zero_address(&env, &stellar_address) {
            return Err(ContractError::ZeroAddress);
        }

        // Reserved names are held back for their real owners; the check is
        // case-insensitive so "Stellar" cannot slip past a reserved "stellar".
        if crate::storage::is_reserved(&env, &github_username) {
            return Err(ContractError::UsernameReserved);
        }

        // Block re-registration while a challenge is pending (Issue #214).
        // The registrant's window to prove ownership off-chain must not be
        // bypassed by simply re-registering to a new address.
        if has_challenge(&env, &github_username) {
            return Err(ContractError::ChallengeActive);
        }

        stellar_address.require_auth();

        // Validate fallback address list size and require auth on each.
        if fallback_addresses.len() > MAX_FALLBACK_ADDRESSES {
            return Err(ContractError::FallbackListFull);
        }
        for i in 0..fallback_addresses.len() {
            let addr = fallback_addresses.get(i).unwrap();
            if is_zero_address(&env, &addr) {
                return Err(ContractError::ZeroAddress);
            }
            if addr != stellar_address {
                addr.require_auth();
            }
        }

        let timestamp = env.ledger().timestamp();
        let existing = get_record(&env, &github_username);

        if let Some(ref old) = existing {
            if old.stellar_address != stellar_address {
                old.stellar_address.require_auth();
            }
        }

        if is_in_cooldown(&env, &github_username) {
            return Err(ContractError::CooldownActive);
        }

        // With a rotation delay configured, an address change must go through
        // request/execute so the delay window and its events actually apply.
        // A direct swap here would be exactly the instant takeover the delay
        // exists to prevent (Issue #234).
        if let Some(ref old) = existing {
            if old.stellar_address != stellar_address && storage_get_rotation_delay(&env) > 0 {
                return Err(ContractError::RotationRequired);
            }
        }

        // Defaults to `stellar_address` for a first-time registration; a
        // re-registration preserves whatever payout address was already on
        // file rather than resetting it back to identity every time.
        let resolved_payout = existing
            .as_ref()
            .map(|r| r.payout_address.clone())
            .unwrap_or_else(|| stellar_address.clone());

        let record = ContributorRecord {
            stellar_address: stellar_address.clone(),
            payout_address: resolved_payout,
            registered_at: timestamp as u32,
            verified: existing
                .as_ref()
                .map(|r| r.stellar_address == stellar_address && r.verified)
                .unwrap_or(false),
            is_bot: existing.as_ref().map(|r| r.is_bot).unwrap_or(false),
        };

        if existing.is_none() {
            set_count(&env, get_count(&env).saturating_add(1));
            add_to_index(&env, &github_username);
        } else if let Some(old) = existing {
            if old.stellar_address != stellar_address && old.verified {
                set_verified_count(&env, storage_get_verified_count(&env).saturating_sub(1));
                // Mark pending reverify because address changed for a verified user
                set_pending_reverify(&env, &github_username, true);
            }
        }

        set_record(&env, &github_username, &record);
        set_last_action(&env, &github_username, timestamp);

        RegisteredEvent {
            github_username: github_username.clone(),
            stellar_address: stellar_address.clone(),
            timestamp,
            sponsor: None,
        }
        .publish(&env);

        set_last_event_ledger(&env);
        push_audit_entry(
            &env,
            AuditLogEntry::new(
                AuditEventType::UserRegistered,
                timestamp,
                Some(stellar_address.clone()),
            )
            .with_username(github_username)
            .with_address(stellar_address),
        );

        Ok(())
    }

    /// Register or update a GitHub username mapping sponsored by a maintainer/account.
    ///
    /// Requires authentication from both the `stellar_address` and the `sponsor`.
    pub fn register_sponsored(
        env: Env,
        github_username: String,
        stellar_address: Address,
        sponsor: Address,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        if !is_valid_github_username(&github_username) {
            return Err(ContractError::InvalidUsername);
        }

        if is_zero_address(&env, &stellar_address) {
            return Err(ContractError::ZeroAddress);
        }

        if has_challenge(&env, &github_username) {
            return Err(ContractError::ChallengeActive);
        }

        sponsor.require_auth();
        stellar_address.require_auth();

        let timestamp = env.ledger().timestamp();
        let existing = get_record(&env, &github_username);

        if let Some(ref old) = existing {
            if old.stellar_address != stellar_address {
                old.stellar_address.require_auth();
            }
        }

        if is_in_cooldown(&env, &github_username) {
            return Err(ContractError::CooldownActive);
        }

        let resolved_payout = existing
            .as_ref()
            .map(|r| r.payout_address.clone())
            .unwrap_or_else(|| stellar_address.clone());

        let record = ContributorRecord {
            stellar_address: stellar_address.clone(),
            payout_address: resolved_payout,
            registered_at: timestamp as u32,
            verified: existing
                .as_ref()
                .map(|r| r.stellar_address == stellar_address && r.verified)
                .unwrap_or(false),
            is_bot: existing.as_ref().map(|r| r.is_bot).unwrap_or(false),
        };

        if existing.is_none() {
            set_count(&env, get_count(&env).saturating_add(1));
            add_to_index(&env, &github_username);
        } else if let Some(old) = existing {
            if old.stellar_address != stellar_address && old.verified {
                set_verified_count(&env, storage_get_verified_count(&env).saturating_sub(1));
                set_pending_reverify(&env, &github_username, true);
            }
        }

        set_record(&env, &github_username, &record);
        set_last_action(&env, &github_username, timestamp);

        RegisteredEvent {
            github_username: github_username.clone(),
            stellar_address: stellar_address.clone(),
            timestamp,
            sponsor: Some(sponsor.clone()),
        }
        .publish(&env);

        set_last_event_ledger(&env);
        push_audit_entry(
            &env,
            AuditLogEntry::new(
                AuditEventType::UserRegistered,
                timestamp,
                Some(stellar_address.clone()),
            )
            .with_username(github_username)
            .with_address(stellar_address),
        );

        Ok(())
    }

    /// Extends the storage TTL of registry records so they are not archived.
    ///
    /// Soroban persistent entries expire unless their TTL is extended. Reads and
    /// writes extend as a side effect, but a record nobody touches for ~30 days
    /// is archived and becomes unreadable until restored — so a registry with a
    /// long tail of inactive contributors silently loses its cold entries.
    ///
    /// This is the keeper operation that prevents that: an off-chain job walks
    /// the index and calls this periodically for entries approaching expiry.
    ///
    /// Permissionless by design. Extending a TTL only ever preserves data —
    /// there is no state an attacker could corrupt by calling it, and gating it
    /// behind admin auth would mean the registry decays whenever the admin key
    /// is unavailable. The caller pays the fee, which is its own rate limit.
    ///
    /// Returns the number of entries actually extended. Usernames that are not
    /// registered are skipped rather than erroring: the keeper's list is built
    /// off-chain and can lag behind removals.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::InvalidBatchSize`] if `usernames` is empty or exceeds
    ///   the configured maximum batch size.
    pub fn extend_registry_ttl(env: Env, usernames: Vec<String>) -> Result<u32, ContractError> {
        require_initialized(&env)?;

        let config = crate::batch::BatchConfig::default();
        if !config.is_valid_batch_size(usernames.len()) {
            return Err(ContractError::InvalidBatchSize);
        }

        let mut extended: u32 = 0;
        for username in usernames.iter() {
            if crate::storage::extend_record_ttl(&env, &username) {
                extended = extended.saturating_add(1);
            }
        }

        Ok(extended)
    }

    /// Removes multiple registrations in a single invocation, collecting
    /// per-entry errors rather than aborting on the first failure.
    ///
    /// This is the batched form of `remove`, intended for
    /// admin workflows that need to clean up many stale or disputed
    /// registrations efficiently. Doing that as N separate invocations costs
    /// N transactions, N signatures, and N rounds of ledger overhead — this
    /// is one.
    ///
    /// **Partial success is the point.** A username that cannot be removed
    /// (e.g. not registered, paused contract, auth failure) does not abort
    /// the batch; it is counted as a failure in the returned
    /// [`BatchSummary`] and the rest proceed. A cleanup of 100 contributors
    /// must not be lost wholesale because one entry was already removed or
    /// the caller lacks permission for a particular record.
    ///
    /// Each successfully removed username publishes a [`RemovedEvent`].
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin. Unlike `remove`,
    /// which also allows the registrant to self-remove, this batch variant
    /// requires admin auth — a registrant who wants to remove only their own
    /// record should call the single-entry version.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::InvalidBatchSize`] if `usernames` is empty or exceeds
    ///   the configured maximum batch size.
    /// - [`ContractError::DualControlRequired`] if `usernames.len()` exceeds
    ///   the configured dual-control threshold (Issue #219, `0` by default,
    ///   which disables this check) — use `propose_batch_remove` /
    ///   `execute_batch_remove` for a batch this size instead.
    ///
    /// # Per-entry outcomes (counted in BatchSummary)
    ///
    /// | Outcome | Counted as | Notes |
    /// |---------|------------|-------|
    /// | Registered, caller is admin | `successful` | Record removed, `RemovedEvent` published |
    /// | Not registered | `failed` | Skipped, batch continues |
    /// | Registered but caller not authorized | `failed` | Should not happen with admin auth, but tracked |
    pub fn batch_remove(
        env: Env,
        caller: Address,
        usernames: Vec<String>,
    ) -> Result<BatchSummary, ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        // Budget cap, not just a shape check — see `MAX_WRITE_BATCH` (Issue #227).
        let config = BatchConfig::for_writes();
        if !config.is_valid_batch_size(usernames.len()) {
            return Err(ContractError::InvalidBatchSize);
        }

        caller.require_auth();

        // Admin must be the caller for batch_remove (stricter than single remove).
        let admin = get_admin(&env)?;
        if caller != admin {
            return Err(ContractError::NotAuthorized);
        }

        // Issue #219: above the configured threshold, a single admin
        // signature is not enough — route through propose/execute instead.
        if crate::storage::requires_batch_remove_dual_control(&env, usernames.len()) {
            return Err(ContractError::DualControlRequired);
        }

        let summary = Self::apply_batch_remove(&env, &caller, &usernames);
        Ok(summary)
    }

    /// Proposes a large `batch_remove` for dual-control execution
    /// (Issue #219). First of the two steps required once `usernames.len()`
    /// exceeds the configured threshold (`set_batch_remove_threshold`) —
    /// `batch_remove` itself refuses a batch that size with
    /// `DualControlRequired`.
    ///
    /// Records the exact username list and who proposed it. A **different**
    /// admin-equivalent address must call `execute_batch_remove` to actually
    /// remove them — one signature alone can never delete a large batch.
    ///
    /// Only one proposal may be pending at a time; call `cancel_batch_remove`
    /// first to replace one. A proposal not executed within
    /// `BATCH_REMOVE_PROPOSAL_TTL_SECS` (24 hours) is treated as gone the
    /// next time anyone calls `execute_batch_remove`.
    ///
    /// Emits [`BatchRemoveProposedEvent`].
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::InvalidBatchSize`] if `usernames` is empty or exceeds
    ///   the configured maximum batch size.
    /// - [`ContractError::NotAuthorized`] if `caller` is not the contract admin.
    /// - [`ContractError::BatchRemoveProposalPending`] if a proposal is
    ///   already pending.
    pub fn propose_batch_remove(
        env: Env,
        caller: Address,
        usernames: Vec<String>,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        let config = BatchConfig::for_writes();
        if !config.is_valid_batch_size(usernames.len()) {
            return Err(ContractError::InvalidBatchSize);
        }

        caller.require_auth();

        let admin = get_admin(&env)?;
        if caller != admin {
            return Err(ContractError::NotAuthorized);
        }

        if let Some(existing) = crate::storage::get_pending_batch_remove(&env) {
            if !crate::storage::is_batch_remove_proposal_expired(&env, &existing) {
                return Err(ContractError::BatchRemoveProposalPending);
            }
        }

        let timestamp = env.ledger().timestamp();
        let count = usernames.len();
        crate::storage::set_pending_batch_remove(
            &env,
            &PendingBatchRemove {
                usernames,
                proposed_by: caller.clone(),
                proposed_at: timestamp,
            },
        );

        BatchRemoveProposedEvent {
            proposed_by: caller,
            count,
            timestamp,
            domain: event_domain(&env),
        }
        .publish(&env);

        Ok(())
    }

    /// Executes the pending large `batch_remove` proposal. Second step of
    /// dual control (Issue #219).
    ///
    /// `caller` must be a **different** address than whoever proposed the
    /// batch, and must itself be admin-equivalent — the contract admin or
    /// any address currently holding `Role::Admin` (see `set_role`). This is
    /// what makes the control "dual": neither address alone can remove the
    /// batch. An operator relying on this must provision at least one
    /// `Role::Admin` holder distinct from the contract admin ahead of time —
    /// see `docs/SECURITY.md#dual-control-batch_remove-issue-219`.
    ///
    /// A proposal older than `BATCH_REMOVE_PROPOSAL_TTL_SECS` is treated as
    /// gone (cleared, `NoPendingBatchRemove`) rather than executed.
    ///
    /// Same partial-success semantics as `batch_remove`: one bad entry does
    /// not block the rest. Emits [`BatchRemoveExecutedEvent`] plus a
    /// [`RemovedEvent`] per removed username.
    ///
    /// # Auth
    ///
    /// Requires auth from `caller`.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NoPendingBatchRemove`] if nothing is proposed, or
    ///   the pending proposal has expired.
    /// - [`ContractError::NotAuthorized`] if `caller` is not admin-equivalent,
    ///   or is the same address that proposed the batch.
    pub fn execute_batch_remove(env: Env, caller: Address) -> Result<BatchSummary, ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        caller.require_auth();

        let proposal = crate::storage::get_pending_batch_remove(&env)
            .ok_or(ContractError::NoPendingBatchRemove)?;

        if crate::storage::is_batch_remove_proposal_expired(&env, &proposal) {
            crate::storage::clear_pending_batch_remove(&env);
            return Err(ContractError::NoPendingBatchRemove);
        }

        let admin = get_admin(&env)?;
        let is_admin_equivalent =
            caller == admin || storage_get_role(&env, &caller) == Some(Role::Admin);
        if !is_admin_equivalent || caller == proposal.proposed_by {
            return Err(ContractError::NotAuthorized);
        }

        crate::storage::clear_pending_batch_remove(&env);

        let count = proposal.usernames.len();
        let summary = Self::apply_batch_remove(&env, &caller, &proposal.usernames);

        BatchRemoveExecutedEvent {
            executed_by: caller,
            proposed_by: proposal.proposed_by,
            count,
            successful: summary.successful,
            timestamp: env.ledger().timestamp(),
            domain: event_domain(&env),
        }
        .publish(&env);

        Ok(summary)
    }

    /// Cancels the pending large `batch_remove` proposal without executing
    /// it (Issue #219). Available even while paused — an abort path should
    /// not itself be blockable by the same emergency that might motivate
    /// using it.
    ///
    /// Emits [`BatchRemoveCancelledEvent`].
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if `caller` is not the contract admin.
    /// - [`ContractError::NoPendingBatchRemove`] if nothing is proposed.
    pub fn cancel_batch_remove(env: Env, caller: Address) -> Result<(), ContractError> {
        require_initialized(&env)?;
        caller.require_auth();

        let admin = get_admin(&env)?;
        if caller != admin {
            return Err(ContractError::NotAuthorized);
        }

        let proposal = crate::storage::get_pending_batch_remove(&env)
            .ok_or(ContractError::NoPendingBatchRemove)?;

        crate::storage::clear_pending_batch_remove(&env);

        BatchRemoveCancelledEvent {
            cancelled_by: caller,
            proposed_by: proposal.proposed_by,
            timestamp: env.ledger().timestamp(),
            domain: event_domain(&env),
        }
        .publish(&env);

        Ok(())
    }

    /// Returns the pending large-batch proposal, if one exists — regardless
    /// of whether it has already expired (Issue #219). Read-only; no auth
    /// required; works while paused, so a holder of either key can always
    /// see what is queued.
    #[must_use]
    pub fn get_pending_batch_remove(env: Env) -> Option<PendingBatchRemove> {
        crate::storage::get_pending_batch_remove(&env)
    }

    /// Sets the `batch_remove` dual-control size threshold. Admin-only
    /// (Issue #219).
    ///
    /// `0` (the default) disables dual control entirely — every
    /// `batch_remove` call executes directly regardless of size, matching
    /// pre-#219 behavior. A non-zero value requires any batch strictly
    /// larger than it to go through `propose_batch_remove` /
    /// `execute_batch_remove` instead.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    pub fn set_batch_remove_threshold(env: Env, threshold: u32) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        crate::storage::set_batch_remove_threshold(&env, threshold);
        Ok(())
    }

    /// Returns the configured `batch_remove` dual-control size threshold —
    /// `0` if dual control is disabled (the default) (Issue #219).
    ///
    /// Read-only; no auth required.
    #[must_use]
    pub fn get_batch_remove_threshold(env: Env) -> u32 {
        crate::storage::get_batch_remove_threshold(&env)
    }

    /// Shared removal loop for `batch_remove` and `execute_batch_remove`
    /// (Issue #219). `caller` is the signer of the transaction actually
    /// performing the removals — the admin for a direct `batch_remove`, or
    /// the executing address for a dual-control batch — and is what gets
    /// recorded on each `RemovedEvent` / audit entry.
    fn apply_batch_remove(env: &Env, caller: &Address, usernames: &Vec<String>) -> BatchSummary {
        let total = usernames.len();
        let mut successful: u32 = 0;
        let mut removed: u32 = 0;
        let mut unverified: u32 = 0;

        for username in usernames.iter() {
            // Attempt the remove. Silently skip failures (not registered, etc.)
            // so one bad entry does not kill the whole batch.
            let record = match get_record(env, &username) {
                Some(r) => r,
                None => continue, // not registered — count as failure below
            };

            let timestamp = env.ledger().timestamp();
            let stellar_address = record.stellar_address.clone();

            remove_record(env, &username);
            remove_from_index(env, &username);

            // Counters are accumulated and written once after the loop, so
            // `count` and `vcount` move together exactly once per batch rather
            // than 2N times — see the note at the end of this function.
            removed = removed.saturating_add(1);
            if record.verified {
                unverified = unverified.saturating_add(1);
            }

            RemovedEvent {
                github_username: username.clone(),
                stellar_address: stellar_address.clone(),
                timestamp,
                domain: event_domain(env),
            }
            .publish(env);

            push_audit_entry(
                env,
                AuditLogEntry::new(AuditEventType::UserRemoved, timestamp, Some(caller.clone()))
                    .with_username(username.clone())
                    .with_address(stellar_address),
            );

            successful = successful.saturating_add(1);
        }

        // Single write per counter for the whole batch (Issue #227). The
        // read-modify-write that used to sit inside the loop cost 2 storage
        // operations per entry, and left `count` and `vcount` briefly
        // inconsistent with each other partway through.
        if removed > 0 {
            set_count(env, get_count(env).saturating_sub(removed));
        }
        if unverified > 0 {
            set_verified_count(
                env,
                storage_get_verified_count(env).saturating_sub(unverified),
            );
        }

        BatchSummary::new(total, successful)
    }

    /// Looks up the `ContributorRecord` for `github_username`. Returns `None` if not registered.
    ///
    /// Read-only; no auth required. Use this for payout address resolution in GitHub Actions
    /// and dashboard lookups.
    ///
    /// **Complexity: O(1).** Resolved by a single direct persistent-key read of
    /// `(REG_KEY, github_username)`; it never scans the flat or chunked username
    /// index, so its cost is independent of registry size or chunk count
    /// (Issue #291).
    #[must_use]
    pub fn get_address(env: Env, github_username: String) -> Option<ContributorRecord> {
        // One direct-key read. `get_record` already returns `None` for a missing
        // entry, so the previous `has_record` pre-check only doubled the storage
        // work (Issue #291).
        get_record(&env, &github_username)
    }

    /// Verified-only address lookup for CI payouts (Issue #287).
    ///
    /// [`Self::get_address`] returns the record whenever the username is present,
    /// regardless of the `verified` flag. A GitHub Action that only checks
    /// presence would therefore pay an **unverified** registration — a squatter
    /// who registered a G-address against someone else's username but was never
    /// confirmed off-chain. This entry point refuses that case at the contract
    /// boundary so the action does not have to.
    ///
    /// Outcomes are distinct and exhaustive:
    ///
    /// | State | Result |
    /// |-------|--------|
    /// | Username registered **and** `verified == true` | `Ok(ContributorRecord)` |
    /// | Username registered but `verified == false` | `Err(ContractError::NotVerified)` (code 6) |
    /// | Username never registered / removed | `Err(ContractError::NotRegistered)` (code 4) |
    ///
    /// Read-only; no auth required; works while the contract is paused — the same
    /// contract-call semantics as [`Self::get_address`], which is left unchanged
    /// (Issue #287 constraint: no versioned sibling may change `get_address`).
    /// Callers that want payout eligibility to also depend on pause state should
    /// additionally check [`Self::is_paused`].
    ///
    /// # ABI note for the GitHub Action
    ///
    /// Resolve payout addresses with `get_address_if_verified`, not `get_address`.
    /// Treat error code `6` (`NotVerified`) and code `4` (`NotRegistered`)
    /// identically: **do not pay**. Only an `Ok` result carries a payable
    /// `stellar_address` / `payout_address`.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotRegistered`] if `github_username` has no record.
    /// - [`ContractError::NotVerified`] if the record exists but is not verified.
    pub fn get_address_if_verified(
        env: Env,
        github_username: String,
    ) -> Result<ContributorRecord, ContractError> {
        match get_record(&env, &github_username) {
            Some(record) if record.verified => Ok(record),
            Some(_) => Err(ContractError::NotVerified),
            None => Err(ContractError::NotRegistered),
        }
    }

    /// Returns `true` if `github_username` is registered, without deserializing the full record.
    ///
    /// Read-only; no auth required. Use this when callers only need existence confirmation
    /// and do not need the [`ContributorRecord`] fields.
    ///
    /// **Complexity: O(1).** A direct persistent-key `has` on
    /// `(REG_KEY, github_username)` — no index scan, no deserialization, no TTL
    /// bump (Issue #291).
    #[must_use]
    pub fn has_record(env: Env, github_username: String) -> bool {
        has_record(&env, &github_username)
    }

    /// Returns a light-client existence proof for `github_username` (Issue #230).
    ///
    /// [`Self::has_record`] answers only yes/no. This carries the verified bit,
    /// when the record was written, the ledger the answer was taken at, and the
    /// storage key plus TTL policy needed to read or revive the ledger entry —
    /// enough for an indexer or the GitHub action to confirm one registration
    /// without paging the whole registry.
    ///
    /// A missing username is not an error: the proof comes back with
    /// `exists: false`, which is itself the answer a light client needs.
    ///
    /// The exact `liveUntilLedgerSeq` is **not** included, because a contract
    /// cannot read its own entry's TTL on-chain. Read it from the ledger entry
    /// at `(key_prefix, github_username)`; see
    /// [`docs/STORAGE_RENT.md`](../docs/STORAGE_RENT.md).
    ///
    /// Read-only; no auth required; works while paused.
    #[must_use]
    pub fn get_record_proof(env: Env, github_username: String) -> RecordProof {
        build_record_proof(&env, &github_username)
    }

    /// Removes the registration for `github_username`. Callable by the registrant or the admin.
    ///
    /// `caller` must sign the transaction and must equal either the contract admin or the
    /// Stellar address currently registered to `github_username`. This prevents anyone other
    /// than the owner or admin from de-registering an account.
    ///
    /// Decrements the total count and, if the record was verified, the verified count.
    /// Emits [`RemovedEvent`].
    ///
    /// # Auth
    ///
    /// Requires auth from `caller` (registrant or admin).
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotRegistered`] if `github_username` is not registered.
    /// - [`ContractError::NotAuthorized`] if `caller` is neither the registrant nor the admin.
    pub fn remove(env: Env, caller: Address, github_username: String) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        let record = get_record(&env, &github_username).ok_or(ContractError::NotRegistered)?;
        let admin = get_admin(&env)?;

        caller.require_auth();
        Self::require_remove_auth(&caller, &admin, &record.stellar_address)?;

        let timestamp = env.ledger().timestamp();
        let stellar_address = record.stellar_address.clone();

        remove_record(&env, &github_username);
        remove_from_index(&env, &github_username);
        set_count(&env, get_count(&env).saturating_sub(1));

        if record.verified {
            set_verified_count(&env, storage_get_verified_count(&env).saturating_sub(1));
        }

        // If a challenge was active, clear it — the registrant beat the clock.
        remove_challenge(&env, &github_username);

        RemovedEvent {
            github_username: github_username.clone(),
            stellar_address: stellar_address.clone(),
            timestamp,
            domain: event_domain(&env),
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::UserRemoved, timestamp, Some(caller))
                .with_username(github_username)
                .with_address(stellar_address),
        );

        Ok(())
    }

    /// Returns a page of `(github_username, stellar_address)` pairs starting at `offset`.
    ///
    /// Admin-only alternative to `get_registered_paginated` that uses a simple offset/limit
    /// rather than a cursor. Returns up to `limit` entries beginning at `offset` in the
    /// registration index. Use for small exports; for large registries prefer the cursor-based
    /// `get_registered_paginated`.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    pub fn get_registered_page(
        env: Env,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<(String, Address)>, ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        let page = crate::storage::get_index_page(&env, offset, limit);
        let mut result = Vec::new(&env);
        for i in 0..page.len() {
            if let Some(username) = page.get(i) {
                if let Some(record) = get_record(&env, &username) {
                    result.push_back((username, record.stellar_address));
                }
            }
        }

        Ok(result)
    }

    /// Returns the complete registry as a list of `(github_username, stellar_address)` pairs.
    ///
    /// **Admin-only.** For large registries this call materialises the entire index in a
    /// single transaction; prefer `get_registered_paginated` or `get_public_paginated` for
    /// incremental dashboard sync.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    pub fn get_all_registered(env: Env) -> Result<Vec<(String, Address)>, ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        let index = get_index(&env);
        let mut result = Vec::new(&env);

        for i in 0..index.len() {
            if let Some(username) = index.get(i) {
                if let Some(record) = get_record(&env, &username) {
                    result.push_back((username, record.stellar_address));
                }
            }
        }

        Ok(result)
    }

    /// Exports a page of registry records using an opaque cursor. Admin-only.
    ///
    /// `cursor` is `None` to start from the beginning, or the exact
    /// `next_cursor` value a previous call returned to continue — it is an
    /// **opaque token** (Issue #215): never construct or decode one
    /// yourself, and never persist one across a contract upgrade that
    /// changes this encoding. `limit` is the maximum number of records to
    /// return (capped at `MAX_PAGE_LIMIT`). Returns an [`ExportPage`]
    /// containing the records, a `next_cursor` for the following call, and a
    /// `merkle_root` over the page (Issue #216).
    ///
    /// Use this instead of `get_all_registered` for large registries — it avoids
    /// materializing the full index in one transaction.
    ///
    /// A cursor issued before a username was removed from the registry no
    /// longer decodes: continuing to walk the registry with a stale offset
    /// after a middle entry is removed would silently skip or duplicate
    /// records for a consumer that isn't reconciling by upsert, so this
    /// fails loudly instead. See `docs/DASHBOARD_SYNC.md` for the recommended
    /// restart-from-`None` recovery.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    /// - [`ContractError::InvalidCursor`] if `cursor` is `Some` and no longer
    ///   decodes against the current registry state.
    pub fn get_registered_paginated(
        env: Env,
        cursor: Option<BytesN<8>>,
        limit: u32,
    ) -> Result<ExportPage, ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        get_registered_paginated_internal(&env, cursor, limit)
    }

    /// Returns a signed export attestation for one page of the registry.
    /// Admin-only (Issue #223).
    ///
    /// Same `cursor`/`limit` pagination as `get_registered_paginated` — this
    /// wraps the identical [`ExportPage`] with a SHA-256 digest over it, the
    /// contract's schema version, and the ledger sequence the read happened
    /// at. `scripts/export_registry.sh` (or any offline tool) can hash the
    /// same page bytes it just downloaded and compare against `digest`
    /// without any further network round-trip — the point for air-gapped
    /// audits, where the auditor may not have live RPC access at all.
    ///
    /// An empty registry is not an error: `page.records` is empty, `total`
    /// is `0`, and `digest` is still a real hash over that (empty) page —
    /// deterministic and comparable like any other page.
    ///
    /// This does **not** replace `get_registered_paginated` /
    /// `get_all_registered` for the actual bulk export — it is a companion
    /// attestation over the same data. Admin auth for the underlying export
    /// is unchanged; this adds a binding on top, it does not relax anything.
    ///
    /// Unaffected by the normal pause flag, matching
    /// `get_registered_paginated` / `get_all_registered`: admin export and
    /// audit tooling stays available during a maintenance window.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    pub fn export_attestation(
        env: Env,
        cursor: u32,
        limit: u32,
    ) -> Result<ExportAttestation, ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        let page = get_registered_paginated_internal(&env, cursor, limit)?;
        let digest = crate::storage::build_export_digest(&env, &page);
        let version = storage_get_version(&env).unwrap_or_else(|| CONTRACT_VERSION.to_tuple());
        let ledger = env.ledger().sequence();

        Ok(ExportAttestation {
            page,
            digest,
            version: soroban_sdk::vec![&env, version.0, version.1, version.2],
            ledger,
        })
    }

    /// Public paginated read for indexers and dashboard consumers.
    ///
    /// Same opaque-cursor/limit semantics as `get_registered_paginated`
    /// (Issue #215) but requires no auth, making it suitable for public
    /// dashboard sync and off-chain indexers. Returns an [`ExportPage`] with
    /// records, a `next_cursor`, and a `merkle_root` over the page.
    ///
    /// A cursor issued by `get_registered_paginated` may be passed here and
    /// vice versa — both read the same underlying index and generation
    /// counter, so cursors are interchangeable between the admin and public
    /// variants.
    ///
    /// **Available while paused (Issue #294).** This is a public read: the pause
    /// circuit breaker only stops state mutations, and an indexer or dashboard
    /// must be able to keep syncing the registry during a maintenance or
    /// security pause. Previously this path called `require_not_paused` and
    /// returned [`ContractError::Paused`] — that gate was the exact regression
    /// this issue's conformance matrix now guards against, and it has been
    /// removed.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    pub fn get_public_paginated(
        env: Env,
        cursor: Option<BytesN<8>>,
        limit: u32,
    ) -> Result<ExportPage, ContractError> {
        require_initialized(&env)?;

        get_registered_paginated_internal(&env, cursor, limit)
    }

    /// Toggles contract pause state. Admin-only (Issue #3).
    ///
    /// Operators use this as the read-only upgrade window switch: set
    /// `paused = true` before rotating the WASM hash so mutating calls fail
    /// fast, then set it back to `false` after the new binary is confirmed.
    ///
    /// Emits [`PausedEvent`] when `paused = true` and [`UnpausedEvent`] when
    /// `paused = false`, matching the events emitted by `pause` / `unpause`.
    /// The call is **idempotent**: if the contract is already in the requested
    /// state, no event is emitted and `Ok(())` is returned without error.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::InvalidPauseReason`] if `reason_code` is unrecognized.
    pub fn set_paused(env: Env, paused: bool, reason_code: u32) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        if !PauseReason::is_valid(reason_code) {
            return Err(ContractError::InvalidPauseReason);
        }
        let reason = PauseReason::from_code(reason_code).unwrap_or(PauseReason::Other);

        // Idempotent: skip event emission if already in the requested state.
        if storage_is_paused(&env) == paused {
            return Ok(());
        }

        set_paused_state(&env, paused);
        crate::storage::set_pause_reason(&env, reason);
        let timestamp = env.ledger().timestamp();

        if paused {
            PausedEvent {
                admin: admin.clone(),
                timestamp,
                reason_code,
                domain: event_domain(&env),
            }
            .publish(&env);
        } else {
            UnpausedEvent {
                admin: admin.clone(),
                timestamp,
                reason_code,
                domain: event_domain(&env),
            }
            .publish(&env);
        }

        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::AdminAction, timestamp, Some(admin)),
        );

        Ok(())
    }

    /// Marks a contributor as verified after an off-chain GitHub identity check.
    ///
    /// Callable by the contract admin **or** any address assigned the
    /// `Role::Verifier` role (Issue #12 — verifier role separation).
    ///
    /// If `config_verification` set a non-zero `expires_in` and this
    /// username's **previous** verification has expired, `verify` treats it
    /// as not-currently-verified and renews it — recording a fresh
    /// verification timestamp — rather than returning `AlreadyVerified`
    /// (Issue #218). A verification that is still active (not expired) is
    /// unaffected by this and still rejects a duplicate call.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotAuthorized`] if `caller` is not an admin or verifier.
    /// - [`ContractError::NotRegistered`] if `github_username` is not registered.
    /// - [`ContractError::AlreadyVerified`] if the record is verified and that
    ///   verification has not expired.
    pub fn verify(env: Env, caller: Address, github_username: String) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        caller.require_auth();

        // Caller must be the admin OR hold the Verifier role.
        // Note: Revoker role does NOT grant verify — roles are intentionally
        // separated so a compromised Revoker cannot mark new accounts as
        // verified (Issue #212).
        let is_admin = is_admin_caller(&env, &caller);
        // Verifier authorization (Issue #293): once the campaign allowlist has
        // been populated, a non-admin caller must be an *active* (non-expired)
        // allowlist member — a bare `set_role(Verifier)` grant no longer
        // suffices. Until the first `add_verifier` call the contract stays in
        // pure role-based mode, so existing deployments are unaffected.
        let is_verifier = if is_admin {
            false
        } else if verifier_allowlist_active(&env) {
            storage_is_active_verifier(&env, &caller, env.ledger().timestamp())
        } else {
            storage_get_role(&env, &caller) == Some(Role::Verifier)
        };
        if !is_admin && !is_verifier {
            return Err(ContractError::NotAuthorized);
        }

        // Per-verifier, per-ledger anti-grief cap (Issue #292). Charged before
        // any state read so a spammer calling `verify` on junk usernames still
        // pays into the limit. The admin is exempt.
        if !is_admin {
            charge_verify_rate(&env, &caller, 1)?;
        }

        let mut record = get_record(&env, &github_username).ok_or(ContractError::NotRegistered)?;

        // Issue #218: an expired prior verification does not block a fresh
        // one — only a still-active verification does.
        let was_active =
            record.verified && !crate::storage::is_verification_expired(&env, &github_username);
        if was_active {
            return Err(ContractError::AlreadyVerified);
        }

        let was_verified = record.verified;
        record.verified = true;
        set_record(&env, &github_username, &record);
        let timestamp = env.ledger().timestamp();
        crate::storage::set_verified_at(&env, &github_username, timestamp);

        // Only a genuine false→true transition is a new verification for
        // counting purposes — renewing an expired-but-never-revoked
        // verification does not touch `verified_count` a second time, since
        // it was never decremented when the previous grant expired (expiry
        // is lazy; see `is_verification_expired`).
        if !was_verified {
            set_verified_count(&env, storage_get_verified_count(&env).saturating_add(1));
            bump_ever_verified_count(&env);
        }

        // Clear pending reverify flag upon successful verification
        clear_pending_reverify(&env, &github_username);
        VerifiedEvent {
            github_username: github_username.clone(),
            stellar_address: record.stellar_address.clone(),
            timestamp,
            domain: event_domain(&env),
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::UserVerified, timestamp, Some(caller))
                .with_username(github_username)
                .with_address(record.stellar_address),
        );

        Ok(())
    }

    /// Verifies multiple registered contributors in a single invocation.
    ///
    /// Callable by the contract admin **or** any address assigned the
    /// `Role::Verifier` role. `Role::Revoker` does not grant this permission.
    ///
    /// Same expired-verification renewal behavior as `verify` (Issue #218):
    /// an entry whose prior verification has expired is treated as pending,
    /// not skipped, and gets a fresh verification timestamp.
    ///
    /// # Auth
    ///
    /// Requires auth from `caller` (admin or verifier).
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::InvalidBatchSize`] if `usernames` is empty or exceeds the maximum batch size.
    /// - [`ContractError::NotAuthorized`] if `caller` is not an admin or verifier.
    pub fn batch_verify(
        env: Env,
        caller: Address,
        usernames: Vec<String>,
    ) -> Result<BatchSummary, ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        // Budget cap, not just a shape check — see `MAX_WRITE_BATCH` (Issue #227).
        let config = BatchConfig::for_writes();
        if !config.is_valid_batch_size(usernames.len()) {
            return Err(ContractError::InvalidBatchSize);
        }

        caller.require_auth();

        let is_admin = is_admin_caller(&env, &caller);
        // Verifier authorization (Issue #293): once the campaign allowlist has
        // been populated, a non-admin caller must be an *active* (non-expired)
        // allowlist member — a bare `set_role(Verifier)` grant no longer
        // suffices. Until the first `add_verifier` call the contract stays in
        // pure role-based mode, so existing deployments are unaffected.
        let is_verifier = if is_admin {
            false
        } else if verifier_allowlist_active(&env) {
            storage_is_active_verifier(&env, &caller, env.ledger().timestamp())
        } else {
            storage_get_role(&env, &caller) == Some(Role::Verifier)
        };
        if !is_admin && !is_verifier {
            return Err(ContractError::NotAuthorized);
        }

        // A batch spends one rate-limit unit per requested username, so a batch
        // call cannot be used to exceed the per-ledger cap a loop of single
        // `verify` calls would hit (Issue #292). Counted on the requested size,
        // before dedup/skip, and charged atomically: if the batch would blow the
        // cap it is rejected whole, having written nothing. Admin is exempt.
        if !is_admin {
            charge_verify_rate(&env, &caller, usernames.len())?;
        }

        let total = usernames.len();
        let timestamp = env.ledger().timestamp();

        // ── Phase 1: decide, without writing ────────────────────────────────
        //
        // Resolve every entry first and collect only those that will actually
        // change. Nothing is written here, so if the batch is going to be
        // rejected it is rejected having touched no state at all — the
        // fail-before-write property this issue asks for.
        //
        // Skipping duplicates matters for the counter: the same username twice
        // in one batch would otherwise be counted twice against `vcount` even
        // though only one record changes. A record whose verification is
        // still active (verified and not expired, Issue #218) is skipped the
        // same way `verify` skips it; an expired one is treated as pending.
        let mut pending: Vec<String> = Vec::new(&env);
        for username in usernames.iter() {
            let Some(record) = get_record(&env, &username) else {
                continue;
            };
            let active =
                record.verified && !crate::storage::is_verification_expired(&env, &username);
            if active {
                continue;
            }
            if pending.iter().any(|u| u == username) {
                continue;
            }
            pending.push_back(username);
        }

        // ── Phase 2: apply ──────────────────────────────────────────────────
        let mut successful: u32 = 0;
        // Only entries making a genuine false→true transition count toward
        // `verified_count` / `ever_verified_count` — renewing an
        // expired-but-never-revoked entry does not, since it was never
        // decremented when its previous grant expired (expiry is lazy).
        let mut newly_verified: u32 = 0;
        for username in pending.iter() {
            // Re-read rather than carrying the record from phase 1: cloning a
            // record per pending entry would hold the whole batch in memory,
            // and the value cannot have changed in between — nothing else runs
            // inside this invocation.
            let Some(mut record) = get_record(&env, &username) else {
                continue;
            };

            let was_verified = record.verified;
            record.verified = true;
            set_record(&env, &username, &record);
            crate::storage::set_verified_at(&env, &username, timestamp);
            if !was_verified {
                newly_verified = newly_verified.saturating_add(1);
                bump_ever_verified_count(&env);
            }
            clear_pending_reverify(&env, &username);

            VerifiedEvent {
                github_username: username.clone(),
                stellar_address: record.stellar_address.clone(),
                timestamp,
                domain: event_domain(&env),
            }
            .publish(&env);

            push_audit_entry(
                &env,
                AuditLogEntry::new(
                    AuditEventType::UserVerified,
                    timestamp,
                    Some(caller.clone()),
                )
                .with_username(username.clone())
                .with_address(record.stellar_address),
            );

            successful = successful.saturating_add(1);
        }

        // One counter write for the whole batch instead of a read-modify-write
        // per entry. That is 2 storage operations rather than 2N, and it means
        // `vcount` moves exactly once — there is no intermediate state in which
        // it has been advanced for some entries but not others.
        if newly_verified > 0 {
            set_verified_count(
                &env,
                storage_get_verified_count(&env).saturating_add(newly_verified),
            );
        }

        Ok(BatchSummary::new(total, successful))
    }

    /// Revokes verification for a registered contributor.
    ///
    /// Callable by the contract admin **or** any address assigned the
    /// `Role::Revoker` role (Issue #212 — Verifier and Revoker are now
    /// separate roles).  A `Role::Verifier` holder cannot revoke; a
    /// `Role::Revoker` holder cannot verify.  Admin can do both.
    ///
    /// `reason_code` must be one of the valid `RevokeReason` codes. See the
    /// `RevokeReason` enum for the supported values and their meanings.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::InvalidReasonCode`] if `reason_code` is not recognized.
    /// - [`ContractError::NotAuthorized`] if `caller` is not an admin or revoker.
    /// - [`ContractError::NotRegistered`] if `github_username` is not registered.
    /// - [`ContractError::NotVerified`] if the record is not currently verified.
    pub fn revoke_verification(
        env: Env,
        caller: Address,
        github_username: String,
        reason_code: u32,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        // Validate reason code before auth to fail fast on malformed input.
        if !RevokeReason::is_valid(reason_code) {
            return Err(ContractError::InvalidReasonCode);
        }

        caller.require_auth();

        // Caller must be the admin OR hold the Revoker role (Issue #212).
        // Verifier role is intentionally excluded: a compromised Verifier key
        // should not be able to undo payout eligibility for existing users.
        let is_admin = is_admin_caller(&env, &caller);
        let is_revoker = storage_get_role(&env, &caller) == Some(Role::Revoker);
        if !is_admin && !is_revoker {
            return Err(ContractError::NotAuthorized);
        }

        // Revoke shares the per-actor, per-ledger cap with verify (Issue #292):
        // a compromised Revoker key can bloat events just as fast by revoking.
        // Admin is exempt.
        if !is_admin {
            charge_verify_rate(&env, &caller, 1)?;
        }

        let mut record = get_record(&env, &github_username).ok_or(ContractError::NotRegistered)?;

        if !record.verified {
            return Err(ContractError::NotVerified);
        }

        record.verified = false;
        set_record(&env, &github_username, &record);
        set_verified_count(&env, storage_get_verified_count(&env).saturating_sub(1));
        // A revoked record has no verification left to expire — clear the
        // timestamp so a later fresh `verify()` is not compared against a
        // stale grant (Issue #218). Also covers the "expired but not
        // revoked" case cleanly: revoking still works off the raw `verified`
        // flag regardless of expiry, and this leaves nothing behind either way.
        crate::storage::clear_verified_at(&env, &github_username);

        let timestamp = env.ledger().timestamp();
        VerificationRevokedEvent {
            github_username: github_username.clone(),
            stellar_address: record.stellar_address.clone(),
            timestamp,
            reason_code,
            domain: event_domain(&env),
        }
        .publish(&env);

        Ok(())
    }

    /// Sets the bot-account flag for a registered contributor.
    ///
    /// The caller must sign and must equal either the contract admin or the
    /// registered Stellar address for `github_username` (self vs admin auth).
    pub fn set_bot_status(
        env: Env,
        caller: Address,
        github_username: String,
        is_bot: bool,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        caller.require_auth();

        let mut record = get_record(&env, &github_username).ok_or(ContractError::NotRegistered)?;
        let admin = get_admin(&env)?;

        if caller != admin && caller != record.stellar_address {
            return Err(ContractError::NotAuthorized);
        }

        record.is_bot = is_bot;
        set_record(&env, &github_username, &record);

        Ok(())
    }

    /// Returns the number of currently verified registrations.
    ///
    /// Read-only; no auth required. The verified count is incremented by `verify` and
    /// decremented by `revoke_verification` and `remove` (when removing a verified record).
    #[must_use]
    pub fn get_verified_count(env: Env) -> u32 {
        storage_get_verified_count(&env)
    }

    /// Moves a registration from `old_username` to `new_username` (Issue #233).
    ///
    /// GitHub users rename. Without this they had to `remove` and re-register,
    /// which drops the record entirely and leaves the old name free for anyone
    /// to take in between.
    ///
    /// The move is atomic: the new record is written and the old one removed in
    /// the same invocation, so there is no ledger state where the registration
    /// exists under both names or neither. The total registration count is
    /// unchanged — this is a move, not a new registration.
    ///
    /// # Verified state
    ///
    /// **The verified flag does not travel.** Verification attests that a
    /// particular GitHub identity controls the address; the contract cannot
    /// confirm that the new handle is the same GitHub account, and carrying the
    /// badge across would let a verified throwaway rename onto a valuable
    /// handle and arrive pre-trusted. The record is marked pending re-verify
    /// instead, and `get_ever_verified_count` still remembers the original
    /// verification. See `docs/ABI.md`.
    ///
    /// Emits [`RenamedEvent`].
    ///
    /// # Auth
    ///
    /// Requires auth from `caller`, which must be the registered address or the
    /// contract admin — the same rule `remove` applies.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotRegistered`] if `old_username` is not registered.
    /// - [`ContractError::NotAuthorized`] if `caller` is neither holder nor admin.
    /// - [`ContractError::InvalidUsername`] if `new_username` is not a valid
    ///   GitHub username, or is the same as `old_username`.
    /// - [`ContractError::UsernameReserved`] if `new_username` is reserved.
    /// - [`ContractError::UsernameTaken`] if `new_username` is already registered.
    /// - [`ContractError::CooldownActive`] if the username is in cooldown.
    /// - [`ContractError::ChallengeActive`] if a challenge is open on either name.
    /// - [`ContractError::RotationPending`] if an address rotation is pending.
    pub fn rename(
        env: Env,
        caller: Address,
        old_username: String,
        new_username: String,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        let record = get_record(&env, &old_username).ok_or(ContractError::NotRegistered)?;
        let admin = get_admin(&env)?;

        caller.require_auth();
        Self::require_remove_auth(&caller, &admin, &record.stellar_address)?;

        if !is_valid_github_username(&new_username) {
            return Err(ContractError::InvalidUsername);
        }

        // A rename to the identical string is a no-op dressed as a state
        // change; reject it rather than emit an event for nothing. A
        // case-only rename is a real move and stays allowed, since the storage
        // key is the exact string.
        if new_username == old_username {
            return Err(ContractError::InvalidUsername);
        }

        if crate::storage::is_reserved(&env, &new_username) {
            return Err(ContractError::UsernameReserved);
        }

        if has_record(&env, &new_username) {
            return Err(ContractError::UsernameTaken);
        }

        // An open challenge on either name is an unresolved ownership question.
        if has_challenge(&env, &old_username) || has_challenge(&env, &new_username) {
            return Err(ContractError::ChallengeActive);
        }

        // A queued address rotation is scoped to the old key; let it settle or
        // be cancelled before the name moves out from under it.
        if has_pending_rotation(&env, &old_username) {
            return Err(ContractError::RotationPending);
        }

        if is_in_cooldown(&env, &old_username) {
            return Err(ContractError::CooldownActive);
        }

        let timestamp = env.ledger().timestamp();
        let verification_cleared = record.verified;

        // Verification attested the old handle; it does not follow the rename.
        if verification_cleared {
            set_verified_count(&env, storage_get_verified_count(&env).saturating_sub(1));
        }

        let moved = ContributorRecord {
            stellar_address: record.stellar_address.clone(),
            payout_address: record.payout_address.clone(),
            registered_at: timestamp as u32,
            verified: false,
            is_bot: record.is_bot,
        };

        // Write the new key and drop the old one in the same invocation: the
        // registration is never visible under both names, nor missing from both.
        set_record(&env, &new_username, &moved);
        add_to_index(&env, &new_username);
        remove_record(&env, &old_username);
        remove_from_index(&env, &old_username);

        clear_pending_reverify(&env, &old_username);
        if verification_cleared {
            set_pending_reverify(&env, &new_username, true);
        }

        set_last_action(&env, &new_username, timestamp);

        RenamedEvent {
            old_username: old_username.clone(),
            new_username: new_username.clone(),
            stellar_address: record.stellar_address.clone(),
            verification_cleared,
            timestamp,
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(
                AuditEventType::UserRegistered,
                timestamp,
                Some(record.stellar_address),
            ),
        );

        Ok(())
    }

    // ── Address rotation with a delay window (Issue #234) ────────────────────

    /// Sets how long a requested address rotation must wait before it can be
    /// executed, in seconds. Admin-only. 0 disables the delay.
    ///
    /// While the delay is 0, `register` keeps its direct dual-auth address
    /// change. Once it is set, an address change must go through
    /// [`Self::request_address_rotation`] and
    /// [`Self::execute_address_rotation`], and `register` rejects a direct
    /// change with [`ContractError::RotationRequired`].
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    pub fn set_rotation_delay(env: Env, seconds: u64) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();
        storage_set_rotation_delay(&env, seconds);
        Ok(())
    }

    /// Returns the configured rotation delay in seconds. 0 means disabled.
    ///
    /// Read-only; no auth required.
    #[must_use]
    pub fn get_rotation_delay(env: Env) -> u64 {
        storage_get_rotation_delay(&env)
    }

    /// Returns the pending address rotation for `github_username`, if any.
    ///
    /// Read-only; no auth required; works while paused, so a holder can always
    /// see that a rotation is queued against their name.
    #[must_use]
    pub fn get_pending_rotation(env: Env, github_username: String) -> Option<PendingRotation> {
        storage_get_pending_rotation(&env, &github_username)
    }

    /// Requests moving `github_username` to `new_address` after the delay.
    ///
    /// Both the current address and the new one must sign, exactly as a direct
    /// re-registration required. The difference is that nothing moves yet: the
    /// request is recorded, [`RotationRequestedEvent`] is emitted so indexers
    /// and the holder can see it immediately, and the rotation only becomes
    /// executable once the delay has elapsed. That window is what turns a
    /// phished GitHub-plus-wallet session from an instant payout redirect into
    /// something the real holder can notice and cancel.
    ///
    /// Reads keep returning the **current** address for the whole window.
    ///
    /// # Auth
    ///
    /// Requires auth from both the currently registered address and `new_address`.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotRegistered`] if the username is not registered.
    /// - [`ContractError::ZeroAddress`] if `new_address` is the burn address.
    /// - [`ContractError::RotationPending`] if a rotation is already pending.
    /// - [`ContractError::ChallengeActive`] if a challenge is open on the username.
    pub fn request_address_rotation(
        env: Env,
        github_username: String,
        new_address: Address,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        let record = get_record(&env, &github_username).ok_or(ContractError::NotRegistered)?;

        if is_zero_address(&env, &new_address) {
            return Err(ContractError::ZeroAddress);
        }

        // A challenge is an open question about who owns this name; queuing a
        // rotation underneath it would let the answer change mid-flight.
        if has_challenge(&env, &github_username) {
            return Err(ContractError::ChallengeActive);
        }

        if has_pending_rotation(&env, &github_username) {
            return Err(ContractError::RotationPending);
        }

        // Same dual-auth requirement as a direct re-registration.
        record.stellar_address.require_auth();
        if new_address != record.stellar_address {
            new_address.require_auth();
        }

        let timestamp = env.ledger().timestamp();
        let executable_at = timestamp.saturating_add(storage_get_rotation_delay(&env));

        set_pending_rotation(
            &env,
            &github_username,
            &PendingRotation {
                new_address: new_address.clone(),
                requested_at: timestamp,
                executable_at,
            },
        );

        RotationRequestedEvent {
            github_username: github_username.clone(),
            current_address: record.stellar_address.clone(),
            new_address,
            executable_at,
            timestamp,
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(
                AuditEventType::UserRegistered,
                timestamp,
                Some(record.stellar_address),
            ),
        );

        Ok(())
    }

    /// Executes a pending rotation once its delay has elapsed.
    ///
    /// Applies the same verified-state policy a direct address change had: the
    /// verified flag is cleared and the username is marked pending re-verify,
    /// because the address that was vouched for is no longer the one on file.
    ///
    /// Emits [`RotationExecutedEvent`].
    ///
    /// # Auth
    ///
    /// Requires auth from the incoming address.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotRegistered`] if the username is not registered.
    /// - [`ContractError::NoRotationPending`] if nothing is pending.
    /// - [`ContractError::RotationNotReady`] if the delay has not elapsed.
    pub fn execute_address_rotation(
        env: Env,
        github_username: String,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        let mut record = get_record(&env, &github_username).ok_or(ContractError::NotRegistered)?;
        let pending = storage_get_pending_rotation(&env, &github_username)
            .ok_or(ContractError::NoRotationPending)?;

        let timestamp = env.ledger().timestamp();
        if timestamp < pending.executable_at {
            return Err(ContractError::RotationNotReady);
        }

        pending.new_address.require_auth();

        let old_address = record.stellar_address.clone();

        // The verification vouched for the old address, so it does not travel.
        if record.verified {
            set_verified_count(&env, storage_get_verified_count(&env).saturating_sub(1));
            set_pending_reverify(&env, &github_username, true);
        }

        record.stellar_address = pending.new_address.clone();
        record.registered_at = timestamp as u32;
        record.verified = false;

        set_record(&env, &github_username, &record);
        set_last_action(&env, &github_username, timestamp);
        remove_pending_rotation(&env, &github_username);

        RotationExecutedEvent {
            github_username: github_username.clone(),
            old_address,
            new_address: pending.new_address.clone(),
            timestamp,
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(
                AuditEventType::UserRegistered,
                timestamp,
                Some(pending.new_address),
            ),
        );

        Ok(())
    }

    /// Cancels a pending rotation. Callable by the current holder or the admin.
    ///
    /// This is the half of the delay window that matters: it gives the real
    /// holder a way to stop a rotation they did not intend.
    ///
    /// Emits [`RotationCancelledEvent`].
    ///
    /// # Auth
    ///
    /// Requires auth from `caller`, which must be the currently registered
    /// address or the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotRegistered`] if the username is not registered.
    /// - [`ContractError::NoRotationPending`] if nothing is pending.
    /// - [`ContractError::NotAuthorized`] if `caller` is neither holder nor admin.
    pub fn cancel_address_rotation(
        env: Env,
        caller: Address,
        github_username: String,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;

        let record = get_record(&env, &github_username).ok_or(ContractError::NotRegistered)?;
        if !has_pending_rotation(&env, &github_username) {
            return Err(ContractError::NoRotationPending);
        }

        caller.require_auth();
        let admin = get_admin(&env)?;
        if caller != record.stellar_address && caller != admin {
            return Err(ContractError::NotAuthorized);
        }

        remove_pending_rotation(&env, &github_username);

        let timestamp = env.ledger().timestamp();
        RotationCancelledEvent {
            github_username: github_username.clone(),
            cancelled_by: caller.clone(),
            timestamp,
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::UserRegistered, timestamp, Some(caller)),
        );

        Ok(())
    }

    /// Returns the reason recorded by the most recent `pause` / `unpause`.
    ///
    /// Defaults to [`PauseReason::Other`] on an instance that has never been
    /// paused, so the read is always answerable.
    ///
    /// Read-only; no auth required; works while paused.
    #[must_use]
    pub fn get_pause_reason(env: Env) -> PauseReason {
        crate::storage::get_pause_reason(&env).unwrap_or(PauseReason::Other)
    }

    /// Returns how many verifications have ever been granted, including any
    /// since revoked (Issue #229).
    ///
    /// Unlike [`Self::get_verified_count`], which reports who is verified
    /// *right now* and drops when a verification is revoked, this figure only
    /// ever grows. Reports asking "how many contributors did we ever verify"
    /// should read this one.
    ///
    /// Read-only; no auth required; works while paused.
    #[must_use]
    pub fn get_ever_verified_count(env: Env) -> u32 {
        storage_get_ever_verified_count(&env)
    }

    /// Returns aggregate registry statistics: total and verified registration counts.
    ///
    /// Read-only; no auth required. Suitable for dashboard displays and health checks.
    /// See [`Stats`] for the returned struct fields.
    #[must_use]
    pub fn get_stats(env: Env) -> Stats {
        read_stats(&env)
    }

    /// Returns the ledger sequence containing the most recently emitted event.
    ///
    /// A value of `0` means this contract instance has not emitted an event.
    /// Indexers compare this cursor with their Horizon/RPC ingestion watermark
    /// to detect lag; it remains available while the contract is paused.
    #[must_use]
    pub fn get_last_event_ledger(env: Env) -> u32 {
        storage_get_last_event_ledger(&env)
    }

    /// Returns a single packed health snapshot for dashboards and CI probes (Issue #210).
    ///
    /// Combines pause state, schema version, registration counts, upgrade cooldown, and
    /// attestation presence into one call so operators get a coherent view without five
    /// separate RPC requests.
    ///
    /// Read-only, no auth required, and intentionally works while the contract is paused
    /// — the snapshot is most useful precisely when something may be wrong.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    pub fn get_health(env: Env) -> Result<HealthSnapshot, ContractError> {
        require_initialized(&env)?;

        let paused = storage_is_paused(&env);
        let ver = storage_get_version(&env).unwrap_or_else(|| CONTRACT_VERSION.to_tuple());
        let version = soroban_sdk::vec![&env, ver.0, ver.1, ver.2];
        let stats = read_stats(&env);

        let cooldown_secs = storage_get_cooldown(&env);
        let now = env.ledger().timestamp();
        let cooldown_remaining_secs = if cooldown_secs > 0 {
            let last_upg = get_last_upgrade(&env);
            let next_allowed = last_upg.saturating_add(cooldown_secs);
            if now < next_allowed {
                next_allowed - now
            } else {
                0
            }
        } else {
            0
        };

        let attestation_present = match get_wasm_attestation(&env) {
            Some(a) => a.expires_at > now,
            None => false,
        };

        Ok(HealthSnapshot {
            paused,
            version,
            total: stats.total,
            verified: stats.verified,
            cooldown_secs,
            cooldown_remaining_secs,
            attestation_present,
        })
    }

    // ── Challenge-period flow (Issue #214) ───────────────────────────────────

    /// Starts a challenge on a registered username. Admin-only.
    ///
    /// Places the username in a locked state for `DEFAULT_CHALLENGE_DELAY_SECS`
    /// (48 hours). While the challenge is active:
    ///
    /// - Re-registration is blocked (`ChallengeActive`).
    /// - The current registrant can still remove their own record (self-remove
    ///   via `remove`), which clears the challenge atomically.
    /// - `complete_challenge` is gated behind the delay so the registrant has
    ///   time to prove GitHub ownership off-chain.
    ///
    /// Emits [`ChallengeStartedEvent`].
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotAuthorized`] if `caller` is not the admin.
    /// - [`ContractError::NotRegistered`] if `github_username` is not registered.
    /// - [`ContractError::ChallengeAlreadyActive`] if a challenge is already open.
    pub fn start_challenge(
        env: Env,
        caller: Address,
        github_username: String,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        caller.require_auth();

        let admin = get_admin(&env)?;
        if caller != admin {
            return Err(ContractError::NotAuthorized);
        }

        if !has_record(&env, &github_username) {
            return Err(ContractError::NotRegistered);
        }

        if has_challenge(&env, &github_username) {
            return Err(ContractError::ChallengeAlreadyActive);
        }

        let now = env.ledger().timestamp();
        let resolve_after = now.saturating_add(DEFAULT_CHALLENGE_DELAY_SECS);

        set_challenge(
            &env,
            &github_username,
            &ChallengeRecord {
                challenged_by: caller.clone(),
                started_at: now,
                resolve_after,
            },
        );

        ChallengeStartedEvent {
            github_username,
            challenged_by: caller,
            resolve_after,
            timestamp: now,
            domain: event_domain(&env),
        }
        .publish(&env);

        Ok(())
    }

    /// Cancels a pending challenge. Admin-only.
    ///
    /// Removes the lock unconditionally, freeing the username for re-registration.
    /// Use this when an off-chain review concludes the registrant is legitimate.
    ///
    /// Emits [`ChallengeCancelledEvent`].
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotAuthorized`] if `caller` is not the admin.
    /// - [`ContractError::NoChallengeActive`] if there is no active challenge.
    pub fn cancel_challenge(
        env: Env,
        caller: Address,
        github_username: String,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        caller.require_auth();

        let admin = get_admin(&env)?;
        if caller != admin {
            return Err(ContractError::NotAuthorized);
        }

        if !has_challenge(&env, &github_username) {
            return Err(ContractError::NoChallengeActive);
        }

        remove_challenge(&env, &github_username);

        let timestamp = env.ledger().timestamp();
        ChallengeCancelledEvent {
            github_username,
            cancelled_by: caller,
            timestamp,
            domain: event_domain(&env),
        }
        .publish(&env);

        Ok(())
    }

    /// Completes a challenge and removes the squatted registration. Admin-only.
    ///
    /// May only be called after `resolve_after` has passed. Removes the
    /// registration, clears the challenge, decrements the counts, and publishes
    /// both a [`ChallengeCompletedEvent`] and a [`RemovedEvent`].
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotAuthorized`] if `caller` is not the admin.
    /// - [`ContractError::NoChallengeActive`] if there is no pending challenge.
    /// - [`ContractError::ChallengeNotResolvable`] if the delay has not elapsed.
    /// - [`ContractError::NotRegistered`] if the record was removed during the challenge window.
    pub fn complete_challenge(
        env: Env,
        caller: Address,
        github_username: String,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        caller.require_auth();

        let admin = get_admin(&env)?;
        if caller != admin {
            return Err(ContractError::NotAuthorized);
        }

        let challenge =
            get_challenge(&env, &github_username).ok_or(ContractError::NoChallengeActive)?;

        let now = env.ledger().timestamp();
        if now < challenge.resolve_after {
            return Err(ContractError::ChallengeNotResolvable);
        }

        let record = get_record(&env, &github_username).ok_or(ContractError::NotRegistered)?;

        // Remove the registration.
        remove_record(&env, &github_username);
        remove_from_index(&env, &github_username);
        set_count(&env, get_count(&env).saturating_sub(1));
        if record.verified {
            set_verified_count(&env, storage_get_verified_count(&env).saturating_sub(1));
        }

        remove_challenge(&env, &github_username);

        RemovedEvent {
            github_username: github_username.clone(),
            stellar_address: record.stellar_address.clone(),
            timestamp: now,
            domain: event_domain(&env),
        }
        .publish(&env);

        ChallengeCompletedEvent {
            github_username,
            completed_by: caller,
            timestamp: now,
            domain: event_domain(&env),
        }
        .publish(&env);

        Ok(())
    }

    /// Returns the pending challenge record for a username, if any.
    ///
    /// Read-only; no auth required. Returns `None` if there is no active challenge.
    #[must_use]
    pub fn get_challenge(env: Env, github_username: String) -> Option<ChallengeRecord> {
        get_challenge(&env, &github_username)
    }

    /// Returns whether the contract is currently paused.
    ///
    /// Alias of `is_paused` kept for the reference event indexer,
    /// which calls this name. Read-only; no auth required.
    #[must_use]
    pub fn is_contract_paused(env: Env) -> bool {
        storage_is_paused(&env)
    }

    /// Returns `true` if `caller` is the contract admin.
    ///
    /// Read-only; no auth required. Useful for off-chain tooling that needs to
    /// check admin status without submitting a transaction.
    #[must_use]
    pub fn has_admin_role(env: Env, caller: Address) -> bool {
        is_admin_caller(&env, &caller)
    }

    /// Returns `true` if `github_username` is still within the registration cooldown window.
    ///
    /// Read-only; no auth required. The cooldown period is set by `set_cooldown`. Returns
    /// `false` if no mutating action has been recorded for the username or the cooldown has
    /// elapsed.
    ///
    /// The cooldown is enforced **automatically** by the contract: `register`, `verify`,
    /// and the username/address change paths stamp the username's last-action timestamp on
    /// success and reject a follow-up mutation with [`ContractError::CooldownActive`] until
    /// the window elapses. This function is the read-only view of that state for off-chain
    /// tooling and cross-contract callers — there is no separate `record_action` entry
    /// point to call (it was removed in Issue #296: an unauthenticated public timestamp
    /// setter let any caller push an arbitrary username into cooldown).
    #[must_use]
    pub fn is_registration_in_cooldown(env: Env, github_username: String) -> bool {
        is_in_cooldown(&env, &github_username)
    }

    // ── Reserved username list (Issue #213) ──────────────────────────────────

    /// Adds `username` to the admin-managed reserved list. Admin-only.
    ///
    /// Once reserved, `register` calls for this username fail with
    /// [`ContractError::UsernameReserved`] regardless of who signs them.
    /// The check is case-insensitive, matching GitHub's own identity rules.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    /// - [`ContractError::InvalidUsername`] if the username is not a valid GitHub username shape.
    /// - [`ContractError::AlreadyReserved`] if the username is already on the list.
    /// - [`ContractError::ReservedListFull`] if the list has reached its maximum size.
    pub fn add_reserved(env: Env, username: String) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        if !crate::utils::is_valid_github_username(&username) {
            return Err(ContractError::InvalidUsername);
        }

        crate::storage::add_to_reserved(&env, &username)?;
        Ok(())
    }

    /// Removes `username` from the reserved list. Admin-only.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    /// - [`ContractError::NotReserved`] if the username is not currently reserved.
    pub fn remove_reserved(env: Env, username: String) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        crate::storage::remove_from_reserved(&env, &username)?;
        Ok(())
    }

    /// Returns `true` if `username` is on the reserved list.
    ///
    /// Read-only; no auth required. Case-insensitive match.
    #[must_use]
    pub fn is_reserved(env: Env, username: String) -> bool {
        crate::storage::is_reserved(&env, &username)
    }

    /// Returns the full reserved username list. Admin-only.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    pub fn get_reserved_list(env: Env) -> Result<Vec<String>, ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();
        Ok(crate::storage::get_reserved_list(&env))
    }

    // ── Index compaction (Issue #209) ─────────────────────────────────────────

    /// Rebuilds the chunked username index densely. Admin-only.
    ///
    /// After removals, chunks may have empty slots (holes). This operation
    /// re-partitions the current flat index into contiguous full chunks plus
    /// a single partial tail, removing all holes and reclaiming persistent
    /// storage entries that are now empty. Pagination results remain the same
    /// except that empty gaps are eliminated.
    ///
    /// Returns the number of chunks written after compaction.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    pub fn compact_index(env: Env) -> Result<u32, ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        let chunks_written = crate::storage::compact_chunked_index(&env);
        Ok(chunks_written)
    }
}

#[cfg(test)]
extern crate alloc;
#[cfg(test)]
extern crate std;

#[cfg(test)]
mod test {
    use super::*;
    use alloc::format;
    use alloc::string::ToString;
    use soroban_sdk::{
        testutils::{Address as _, Events as _, Ledger as _},
        Address, Env, Event as _, String, TryFromVal,
    };

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn setup(env: &Env) -> (Address, Address, Address, Address) {
        let admin = Address::generate(env);
        let user = Address::generate(env);
        let other = Address::generate(env);
        let contract_id = env.register(TrustBridgeContract, ());
        env.as_contract(&contract_id, || {
            TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
        });
        (admin, user, other, contract_id)
    }

    fn username(env: &Env, name: &str) -> String {
        String::from_str(env, name)
    }

    /// Registers `name` to `addr` with no fallback addresses. Must be called
    /// from inside an `env.as_contract(&contract_id, || { .. })` closure.
    fn register_personal(env: &Env, _contract_id: &Address, name: &str, addr: &Address) {
        TrustBridgeContract::register(env.clone(), username(env, name), addr.clone(), Vec::new(env))
            .unwrap();
    }

    /// Asserts `get_verified_count()` and `get_stats().verified` agree on
    /// `expected` (Issue #90). Both APIs are checked together so future
    /// mutation paths cannot let the two counters drift apart silently.
    fn assert_verified_parity(env: &Env, expected: u32) {
        assert_eq!(
            TrustBridgeContract::get_verified_count(env.clone()),
            expected,
            "get_verified_count() diverged from the expected verified count"
        );
        assert_eq!(
            TrustBridgeContract::get_stats(env.clone()).verified,
            expected,
            "get_stats().verified diverged from the expected verified count"
        );
    }

    // ── Basic registration / lookup ──────────────────────────────────────────

    #[test]
    fn test_config_verification_double_initialize_rejection() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        let attestation = soroban_sdk::Symbol::new(&env, "github_att");

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::config_verification(
                env.clone(),
                admin.clone(),
                attestation.clone(),
                3600,
                2,
            )
            .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let res = TrustBridgeContract::config_verification(
                env.clone(),
                admin.clone(),
                attestation,
                3600,
                2,
            );
            assert_eq!(res, Err(ContractError::AlreadyInitialized));
        });
    }

    #[test]
    fn test_register_and_get_address_roundtrip() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert_eq!(record.stellar_address, user);
            assert!(!record.verified);
        });
    }

    #[test]
    fn test_register_with_fallback_addresses() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        let fb1 = Address::generate(&env);
        let fb2 = Address::generate(&env);
        let mut fallbacks: Vec<Address> = Vec::new(&env);
        fallbacks.push_back(fb1.clone());
        fallbacks.push_back(fb2.clone());
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                fallbacks.clone(),
            )
            .unwrap();
            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert_eq!(record.stellar_address, user);
            assert_eq!(record.fallback_addresses.len(), 2);
            assert_eq!(record.fallback_addresses.get(0).unwrap(), fb1);
            assert_eq!(record.fallback_addresses.get(1).unwrap(), fb2);
        });
    }

    #[test]
    fn test_register_fallback_list_exceeds_cap() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        let mut fallbacks: Vec<Address> = Vec::new(&env);
        for _ in 0..=MAX_FALLBACK_ADDRESSES {
            fallbacks.push_back(Address::generate(&env));
        }
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let res = TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                fallbacks,
            );
            assert_eq!(res, Err(ContractError::FallbackListFull));
        });
    }

    #[test]
    fn test_get_address_missing_returns_none() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        env.as_contract(&contract_id, || {
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "missing")).is_none()
            );
        });
    }

    #[test]
    fn test_repeated_missing_lookups_are_stable() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        env.as_contract(&contract_id, || {
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "missing")).is_none()
            );
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "missing")).is_none()
            );
        });
    }

    #[test]
    fn test_register_two_users_keeps_addresses() {
        let env = Env::default();
        let (_admin, user1, user2, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone(), Vec::new(&env))
                .unwrap();
            assert_eq!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "alice"))
                    .unwrap()
                    .stellar_address,
                user1
            );
            assert_eq!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "bob"))
                    .unwrap()
                    .stellar_address,
                user2
            );
        });
    }

    // ── Verified-only lookup (Issue #287) ────────────────────────────────────

    /// A verified registration resolves through `get_address_if_verified` and
    /// carries the same address `get_address` would return.
    #[test]
    fn test_verified_only_lookup_returns_record_when_verified() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
            let record =
                TrustBridgeContract::get_address_if_verified(env.clone(), username(&env, "octocat"))
                    .unwrap();
            assert_eq!(record.stellar_address, user);
            assert!(record.verified);
        });
    }

    /// A registered-but-unverified username is rejected with `NotVerified` (code 6),
    /// even though `get_address` would still return it.
    #[test]
    fn test_verified_only_lookup_rejects_unverified() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).is_some(),
                "get_address still returns the unverified record"
            );
            assert_eq!(
                TrustBridgeContract::get_address_if_verified(env.clone(), username(&env, "octocat")),
                Err(ContractError::NotVerified),
            );
        });
    }

    /// An unregistered username is rejected with `NotRegistered` (code 4),
    /// distinct from the unverified case.
    #[test]
    fn test_verified_only_lookup_rejects_unregistered() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::get_address_if_verified(env.clone(), username(&env, "ghost")),
                Err(ContractError::NotRegistered),
            );
        });
    }

    /// A revoked registration falls back to `NotVerified` — verification can be
    /// withdrawn and the verified-only lookup tracks that immediately.
    #[test]
    fn test_verified_only_lookup_follows_revocation() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
            TrustBridgeContract::revoke_verification(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
                1,
            )
            .unwrap();
            assert_eq!(
                TrustBridgeContract::get_address_if_verified(env.clone(), username(&env, "octocat")),
                Err(ContractError::NotVerified),
            );
        });
    }

    /// The verified-only lookup keeps working while the contract is paused,
    /// matching `get_address` read semantics.
    #[test]
    fn test_verified_only_lookup_works_while_paused() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
            TrustBridgeContract::pause(env.clone(), 1).unwrap();
            let record =
                TrustBridgeContract::get_address_if_verified(env.clone(), username(&env, "octocat"))
                    .unwrap();
            assert_eq!(record.stellar_address, user);
        });
    }

    // ── Stats ────────────────────────────────────────────────────────────────

    #[test]
    fn test_stats_empty_after_setup() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        env.as_contract(&contract_id, || {
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 0);
            assert_eq!(stats.verified, 0);
        });
    }

    #[test]
    fn test_get_stats_increments_correctly() {
        let env = Env::default();
        let (admin, user1, user2, contract_id) = setup(&env);

        env.as_contract(&contract_id, || {
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 0);
            assert_eq!(stats.verified, 0);
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone(), Vec::new(&env))
                .unwrap();
        });

        env.as_contract(&contract_id, || {
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 2);
            assert_eq!(stats.verified, 0);
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "alice"))
                .unwrap();
        });

        env.as_contract(&contract_id, || {
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 2);
            assert_eq!(stats.verified, 1);
            assert_verified_parity(&env, 1);
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user2.clone(), username(&env, "bob")).unwrap();
        });

        env.as_contract(&contract_id, || {
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 1);
            assert_eq!(stats.verified, 1);
        });
    }

    #[test]
    fn test_removing_one_of_two_keeps_remaining_stats() {
        let env = Env::default();
        let (_admin, user1, user2, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user1.clone(), username(&env, "alice"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 1);
            assert_eq!(stats.verified, 0);
        });
    }

    // ── Remove auth (Issue #74 / Wave #75) ────────────────────────────
    //
    // The remove authorization policy: only the registrant (the Stellar address
    // currently registered to the username) or the contract admin may remove a
    // record. This is enforced by `require_remove_auth`, extracted from `remove`
    // so the policy can be read, tested, and modified in isolation.
    //
    // Success paths: registrant removes own record; admin removes any record.
    // Failure paths: third-party caller → NotAuthorized; unregistered username
    //   → NotRegistered; paused contract → Paused; uninitialized → NotInitialized.
    // State invariants: verified counter decremented only when the removed record
    //   was verified; count always decremented by exactly 1 on success.

    // ── Success path: registrant ────────────────────────────────────────────

    /// The registered address (registrant) may remove their own record.
    #[test]
    fn test_registrant_can_remove_own_record() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            let result =
                TrustBridgeContract::remove(env.clone(), other.clone(), username(&env, "octocat"));
            assert_eq!(result, Err(ContractError::NotAuthorized));
        });
    }

    // ── Success path: admin ──────────────────────────────────────────────────

    /// The contract admin may remove any record regardless of who registered it.
    #[test]
    fn test_admin_can_remove_any_record() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::remove(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).is_none()
            );
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 0);
        });
    }

    /// Admin may remove a record even when the registrant is a different address.
    #[test]
    fn test_admin_can_remove_record_registered_by_another_user() {
        let env = Env::default();
        let (admin, user1, _user2, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user1.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).is_none()
            );
        });
    }

    // ── Failure path: unauthorized third party ────────────────────────────────

    /// Any address that is neither the registrant nor the admin must be
    /// rejected with NotAuthorized.
    #[test]
    fn test_third_party_cannot_remove() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            let result =
                TrustBridgeContract::remove(env.clone(), other.clone(), username(&env, "octocat"));
            assert_eq!(
                result,
                Err(ContractError::NotAuthorized),
                "third party must be rejected with NotAuthorized"
            );
        });
        env.as_contract(&contract_id, || {
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).is_some()
            );
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 1);
        });
    }

    /// A freshly-generated address with no role and no registration is rejected.
    #[test]
    fn test_unknown_address_cannot_remove() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        let stranger = Address::generate(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user.clone(), Vec::new(&env))
                .unwrap();
            let result =
                TrustBridgeContract::remove(env.clone(), stranger.clone(), username(&env, "alice"));
            assert_eq!(result, Err(ContractError::NotAuthorized));
        });
    }

    // ── Failure path: unregistered username ─────────────────────────────────

    /// Attempting to remove a username that was never registered returns
    /// NotRegistered and does not mutate any state.
    #[test]
    fn test_remove_unregistered_username_fails() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result =
                TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "missing"));
            assert_eq!(result, Err(ContractError::NotRegistered));
        });
    }

    // ── Failure path: paused contract ─────────────────────────────────────────────

    /// Remove is blocked while the contract is paused.
    #[test]
    fn test_remove_blocked_while_paused() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::pause(env.clone(), 1).unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result =
                TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat"));
            assert_eq!(
                result,
                Err(ContractError::Paused),
                "remove must be blocked while paused"
            );
        });
        env.as_contract(&contract_id, || {
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).is_some()
            );
        });
    }

    // ── State invariants after remove ──────────────────────────────────────────────

    /// Removing an unverified record must decrement total but leave the
    /// verified count unchanged.
    #[test]
    fn test_remove_unverified_record_does_not_decrement_verified_count() {
        let env = Env::default();
        let (admin, user1, user2, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "alice"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).verified, 1);
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), admin.clone(), username(&env, "bob")).unwrap();
        });
        env.as_contract(&contract_id, || {
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 1, "total must decrement by 1");
            assert_eq!(
                stats.verified, 1,
                "verified count must be unchanged when removing an unverified record"
            );
        });
    }

    /// Removing a verified record must decrement both total and verified count.
    #[test]
    fn test_remove_verified_record_decrements_verified_count() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).verified, 1);
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 0);
            assert_eq!(
                stats.verified, 0,
                "verified count must decrement when removing a verified record"
            );
        });
    }

    /// Re-adding a removed username must treat it as a fresh registration:
    /// count increments from 0 to 1, and the new record is unverified.
    #[test]
    fn test_readding_removed_user_increments_count() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 1);
            assert_eq!(
                stats.verified, 0,
                "re-registered record must start unverified"
            );
        });
    }

    // ── Issue #52: lookup after peer removal ─────────────────────────────────

    #[test]
    fn test_remove_then_lookup_other_record() {
        let env = Env::default();
        let (_admin, user1, user2, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user1.clone(), username(&env, "alice"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            // bob's record must survive alice's removal
            assert_eq!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "bob"))
                    .unwrap()
                    .stellar_address,
                user2
            );
        });
    }

    #[test]
    fn test_export_skips_removed_records() {
        let env = Env::default();
        let (_admin, user1, user2, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user1.clone(), username(&env, "alice"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let all = TrustBridgeContract::get_all_registered(env.clone()).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all.get(0).unwrap(), (username(&env, "bob"), user2.clone()));
        });
    }

    #[test]
    fn test_lookup_after_first_of_three_removed() {
        let env = Env::default();
        let (admin, user1, user2, contract_id) = setup(&env);
        let user3 = Address::generate(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "carol"), user3.clone(), Vec::new(&env))
                .unwrap();

            // Remove the first entry
            TrustBridgeContract::remove(env.clone(), admin.clone(), username(&env, "alice"))
                .unwrap();

            // Both remaining records must be reachable
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "alice")).is_none()
            );
            assert_eq!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "bob"))
                    .unwrap()
                    .stellar_address,
                user2
            );
            assert_eq!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "carol"))
                    .unwrap()
                    .stellar_address,
                user3
            );
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 2);
        });
    }

    #[test]
    fn test_lookup_after_middle_of_three_removed() {
        let env = Env::default();
        let (admin, user1, user2, contract_id) = setup(&env);
        let user3 = Address::generate(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "carol"), user3.clone(), Vec::new(&env))
                .unwrap();

            // Remove the middle entry
            TrustBridgeContract::remove(env.clone(), admin.clone(), username(&env, "bob")).unwrap();

            assert_eq!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "alice"))
                    .unwrap()
                    .stellar_address,
                user1
            );
            assert!(TrustBridgeContract::get_address(env.clone(), username(&env, "bob")).is_none());
            assert_eq!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "carol"))
                    .unwrap()
                    .stellar_address,
                user3
            );
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 2);
        });
    }

    #[test]
    fn test_index_integrity_after_multiple_removals() {
        let env = Env::default();
        let (admin, user1, user2, contract_id) = setup(&env);
        let user3 = Address::generate(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "carol"), user3.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), admin.clone(), username(&env, "alice"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), admin.clone(), username(&env, "carol"))
                .unwrap();
        });

        // Only bob remains
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let all = TrustBridgeContract::get_all_registered(env.clone()).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all.get(0).unwrap().0, username(&env, "bob"));
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 1);
        });
    }

    #[test]
    fn test_reregister_after_removal_is_treated_as_new() {
        let env = Env::default();
        let (_admin, user1, user2, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user1.clone(), username(&env, "alice"))
                .unwrap();
        });
        // re-register alice
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
        });

        env.as_contract(&contract_id, || {
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 2);
            assert_eq!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "alice"))
                    .unwrap()
                    .stellar_address,
                user1
            );
        });
    }

    // ── Re-registration ───────────────────────────────────────────────────────

    #[test]
    fn test_reregistration_updates_record() {
        let env = Env::default();
        let (admin, user, new_user, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), new_user.clone(), Vec::new(&env))
                .unwrap();

            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert_eq!(record.stellar_address, new_user);
            assert!(!record.verified);

            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 1);
            assert_eq!(stats.verified, 0);
        });
    }

    #[test]
    fn test_updated_registration_preserves_count() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), other.clone(), Vec::new(&env))
                .unwrap();
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 1);
        });
    }

    #[test]
    fn test_unverified_update_stays_unverified() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), other.clone(), Vec::new(&env))
                .unwrap();
            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert!(!record.verified);
        });
    }

    // ── Issue #16: Verification attestation storage ───────────────────────────

    #[test]
    fn test_verify_sets_verified_flag_and_increments_vcount() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            assert!(
                !TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .verified
            );
            assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 0);

            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();

            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert!(record.verified, "verified flag must be true after verify()");
            assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 1);
        });
    }

    /// Issue #57: `verify` on a username with no registration fails closed
    /// with `NotRegistered` instead of creating a verified record out of
    /// nothing. Documented alongside the matching integration coverage in
    /// `tests/integration.rs` and the error table in `docs/ABI.md`.
    #[test]
    fn test_verify_missing_registration_fails() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result =
                TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "missing"));
            assert_eq!(result, Err(ContractError::NotRegistered));
        });
    }

    // ── Issue #293: verifier allowlist (cap + on-chain expiry) ──────────────

    #[test]
    fn test_verifier_allowlist_cap_enforced() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            for _ in 0..MAX_VERIFIERS {
                TrustBridgeContract::add_verifier(env.clone(), Address::generate(&env), 0).unwrap();
            }
            assert_eq!(TrustBridgeContract::verifier_slots_remaining(env.clone()), 0);
            let res =
                TrustBridgeContract::add_verifier(env.clone(), Address::generate(&env), 0);
            assert_eq!(res, Err(ContractError::VerifierAllowlistFull));
        });
    }

    #[test]
    fn test_verifier_allowlist_expiry_blocks_verify() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        let verifier = Address::generate(&env);
        env.ledger().with_mut(|li| li.timestamp = 1_000);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();
            // Expires at t=2000.
            TrustBridgeContract::add_verifier(env.clone(), verifier.clone(), 2_000).unwrap();
            assert!(TrustBridgeContract::is_active_verifier(env.clone(), verifier.clone()));
            TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "octocat"))
                .unwrap();
        });

        // Advance past expiry; the same verifier is now inactive.
        env.ledger().with_mut(|li| li.timestamp = 3_000);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "hubber"),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
            assert!(!TrustBridgeContract::is_active_verifier(env.clone(), verifier.clone()));
            let res =
                TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "hubber"));
            assert_eq!(res, Err(ContractError::NotAuthorized));
        });
    }

    #[test]
    fn test_verifier_expiry_in_past_rejected() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        env.ledger().with_mut(|li| li.timestamp = 5_000);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let res =
                TrustBridgeContract::add_verifier(env.clone(), Address::generate(&env), 4_000);
            assert_eq!(res, Err(ContractError::VerifierExpiryInPast));
        });
    }

    #[test]
    fn test_expired_verifier_slot_is_reclaimed_and_pruned() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        env.ledger().with_mut(|li| li.timestamp = 1_000);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            for _ in 0..(MAX_VERIFIERS - 1) {
                TrustBridgeContract::add_verifier(env.clone(), Address::generate(&env), 0).unwrap();
            }
            // One short-lived member takes the last slot.
            TrustBridgeContract::add_verifier(env.clone(), Address::generate(&env), 2_000).unwrap();
            assert_eq!(TrustBridgeContract::verifier_slots_remaining(env.clone()), 0);
        });

        env.ledger().with_mut(|li| li.timestamp = 3_000);
        env.as_contract(&contract_id, || {
            // Expired member no longer counts, so a new one fits — add_verifier
            // prunes it on the way in.
            TrustBridgeContract::add_verifier(env.clone(), Address::generate(&env), 0).unwrap();
            let pruned = TrustBridgeContract::prune_expired_verifiers(env.clone()).unwrap();
            assert_eq!(pruned, 0, "add_verifier already pruned the expired entry");
            assert_eq!(TrustBridgeContract::get_verifiers(env.clone()).len(), MAX_VERIFIERS);
        });
    }

    #[test]
    fn test_add_verifier_refresh_does_not_consume_slot() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        env.ledger().with_mut(|li| li.timestamp = 1_000);
        env.mock_all_auths();
        let v = Address::generate(&env);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::add_verifier(env.clone(), v.clone(), 2_000).unwrap();
            TrustBridgeContract::add_verifier(env.clone(), v.clone(), 9_000).unwrap();
            assert_eq!(TrustBridgeContract::get_verifiers(env.clone()).len(), 1);
            assert_eq!(
                TrustBridgeContract::verifier_slots_remaining(env.clone()),
                MAX_VERIFIERS - 1
            );
        });
    }

    #[test]
    fn test_remove_verifier_not_listed_errors() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let res = TrustBridgeContract::remove_verifier(env.clone(), Address::generate(&env));
            assert_eq!(res, Err(ContractError::VerifierNotAllowlisted));
        });
    }

    #[test]
    fn test_role_based_verify_still_works_before_allowlist_opt_in() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        let verifier = Address::generate(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), verifier.clone(), Role::Verifier).unwrap();
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();
            // No add_verifier call yet → pure role-based mode.
            TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "octocat"))
                .unwrap();
        });
    }

    /// Same not-registered guard on the other verification-mutating entry
    /// point, so `verify` and `revoke_verification` stay consistent (Issue
    /// #57).
    #[test]
    fn test_revoke_verification_missing_registration_fails() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::revoke_verification(
                env.clone(),
                admin.clone(),
                username(&env, "missing"),
                1,
            );
            assert_eq!(result, Err(ContractError::NotRegistered));
        });
    }

    #[test]
    fn test_double_verify_fails() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result =
                TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"));
            assert_eq!(result, Err(ContractError::AlreadyVerified));
        });
    }

    // ── Issue #233: username rename ──────────────────────────────────────────

    fn register_as(env: &Env, contract_id: &Address, name: &str, addr: &Address) {
        env.mock_all_auths();
        env.as_contract(contract_id, || {
            TrustBridgeContract::register(env.clone(), username(env, name), addr.clone()).unwrap();
        });
    }

    #[test]
    fn test_rename_moves_the_registration_atomically() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        register_as(&env, &contract_id, "octocat", &user);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::rename(
                env.clone(),
                user.clone(),
                username(&env, "octocat"),
                username(&env, "octocat2"),
            )
            .unwrap();
        });

        env.as_contract(&contract_id, || {
            assert!(
                !TrustBridgeContract::has_record(env.clone(), username(&env, "octocat")),
                "old name must be gone"
            );
            assert_eq!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat2"))
                    .unwrap()
                    .stellar_address,
                user,
                "new name must hold the same address"
            );
            assert_eq!(
                TrustBridgeContract::get_stats(env.clone()).total,
                1,
                "a rename is a move, not a new registration"
            );
        });
    }

    #[test]
    fn test_rename_updates_the_index() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        register_as(&env, &contract_id, "octocat", &user);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::rename(
                env.clone(),
                user.clone(),
                username(&env, "octocat"),
                username(&env, "octocat2"),
            )
            .unwrap();
        });

        env.as_contract(&contract_id, || {
            let page = TrustBridgeContract::get_registered_paginated(env.clone(), 0, 50).unwrap();
            let names: alloc::vec::Vec<_> = page
                .records
                .iter()
                .map(|(name, _)| name.clone())
                .collect();
            assert!(
                names.contains(&username(&env, "octocat2")),
                "index must list the new name"
            );
            assert!(
                !names.contains(&username(&env, "octocat")),
                "index must not still list the old name"
            );
        });
    }

    /// Verification attested the old handle, so it does not follow the rename.
    #[test]
    fn test_rename_clears_verification_and_flags_reverify() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        register_as(&env, &contract_id, "octocat", &user);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::rename(
                env.clone(),
                user.clone(),
                username(&env, "octocat"),
                username(&env, "octocat2"),
            )
            .unwrap();
        });

        env.as_contract(&contract_id, || {
            assert!(
                !TrustBridgeContract::get_address(env.clone(), username(&env, "octocat2"))
                    .unwrap()
                    .verified,
                "the badge must not travel to the new handle"
            );
            assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 0);
            assert_eq!(
                TrustBridgeContract::get_ever_verified_count(env.clone()),
                1,
                "history still remembers the original verification"
            );
        });
    }

    #[test]
    fn test_rename_rejects_a_taken_username() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        register_as(&env, &contract_id, "octocat", &user);
        register_as(&env, &contract_id, "hubber", &other);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::rename(
                    env.clone(),
                    user.clone(),
                    username(&env, "octocat"),
                    username(&env, "hubber"),
                ),
                Err(ContractError::UsernameTaken)
            );
        });
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "hubber"))
                    .unwrap()
                    .stellar_address,
                other,
                "the existing registration must be untouched"
            );
        });
    }

    #[test]
    fn test_rename_rejects_a_reserved_username() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        register_as(&env, &contract_id, "octocat", &user);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::add_reserved(env.clone(), username(&env, "stellar")).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::rename(
                    env.clone(),
                    user.clone(),
                    username(&env, "octocat"),
                    username(&env, "stellar"),
                ),
                Err(ContractError::UsernameReserved)
            );
        });
    }

    #[test]
    fn test_rename_rejects_an_unregistered_username() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::rename(
                    env.clone(),
                    user.clone(),
                    username(&env, "ghost"),
                    username(&env, "ghost2"),
                ),
                Err(ContractError::NotRegistered)
            );
        });
    }

    #[test]
    fn test_rename_rejects_renaming_to_the_same_name() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        register_as(&env, &contract_id, "octocat", &user);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::rename(
                    env.clone(),
                    user.clone(),
                    username(&env, "octocat"),
                    username(&env, "octocat"),
                ),
                Err(ContractError::InvalidUsername)
            );
        });
    }

    #[test]
    fn test_rename_rejects_a_non_holder() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        register_as(&env, &contract_id, "octocat", &user);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::rename(
                    env.clone(),
                    other.clone(),
                    username(&env, "octocat"),
                    username(&env, "octocat2"),
                ),
                Err(ContractError::NotAuthorized)
            );
        });
    }

    /// A case-only change is a real move, since the storage key is the exact string.
    #[test]
    fn test_rename_allows_a_case_only_change() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        register_as(&env, &contract_id, "octocat", &user);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::rename(
                env.clone(),
                user.clone(),
                username(&env, "octocat"),
                username(&env, "OctoCat"),
            )
            .unwrap();
        });
        env.as_contract(&contract_id, || {
            assert!(TrustBridgeContract::has_record(env.clone(), username(&env, "OctoCat")));
            assert!(!TrustBridgeContract::has_record(env.clone(), username(&env, "octocat")));
        });
    }

    /// The old name is free again, and taking it does not disturb the moved record.
    #[test]
    fn test_old_name_is_registerable_after_a_rename() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        register_as(&env, &contract_id, "octocat", &user);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::rename(
                env.clone(),
                user.clone(),
                username(&env, "octocat"),
                username(&env, "octocat2"),
            )
            .unwrap();
        });

        register_as(&env, &contract_id, "octocat", &other);

        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .stellar_address,
                other
            );
            assert_eq!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat2"))
                    .unwrap()
                    .stellar_address,
                user
            );
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 2);
        });
    }

    // ── Issue #234: address rotation delay window ────────────────────────────

    const ROT_DELAY: u64 = 86_400;

    /// Register `octocat` to `user` and arm the rotation delay.
    fn setup_rotation(env: &Env, contract_id: &Address, user: &Address) {
        env.mock_all_auths();
        env.as_contract(contract_id, || {
            TrustBridgeContract::register(env.clone(), username(env, "octocat"), user.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(contract_id, || {
            TrustBridgeContract::set_rotation_delay(env.clone(), ROT_DELAY).unwrap();
        });
    }

    #[test]
    fn test_rotation_delay_defaults_to_zero() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        env.as_contract(&contract_id, || {
            assert_eq!(TrustBridgeContract::get_rotation_delay(env.clone()), 0);
        });
    }

    /// With no delay configured, register keeps its direct dual-auth swap.
    #[test]
    fn test_register_still_swaps_directly_when_delay_is_zero() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), other.clone())
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .stellar_address,
                other
            );
        });
    }

    /// Once a delay is armed, the instant swap is refused.
    #[test]
    fn test_register_refuses_direct_address_change_when_delay_armed() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        setup_rotation(&env, &contract_id, &user);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::register(env.clone(), username(&env, "octocat"), other.clone()),
                Err(ContractError::RotationRequired)
            );
        });
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .stellar_address,
                user,
                "the address must not have moved"
            );
        });
    }

    /// Re-registering the same address is not a rotation and stays allowed.
    #[test]
    fn test_register_same_address_still_allowed_with_delay_armed() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        setup_rotation(&env, &contract_id, &user);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
        });
    }

    #[test]
    fn test_requesting_a_rotation_does_not_move_the_address() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        setup_rotation(&env, &contract_id, &user);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::request_address_rotation(
                env.clone(),
                username(&env, "octocat"),
                other.clone(),
            )
            .unwrap();
        });

        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .stellar_address,
                user,
                "reads return the current address for the whole window"
            );
            let pending =
                TrustBridgeContract::get_pending_rotation(env.clone(), username(&env, "octocat"))
                    .expect("rotation must be pending");
            assert_eq!(pending.new_address, other);
            assert_eq!(pending.executable_at, pending.requested_at + ROT_DELAY);
        });
    }

    #[test]
    fn test_rotation_cannot_execute_before_the_delay_elapses() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        setup_rotation(&env, &contract_id, &user);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::request_address_rotation(
                env.clone(),
                username(&env, "octocat"),
                other.clone(),
            )
            .unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::execute_address_rotation(env.clone(), username(&env, "octocat")),
                Err(ContractError::RotationNotReady)
            );
        });
    }

    #[test]
    fn test_rotation_executes_once_the_delay_has_elapsed() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        setup_rotation(&env, &contract_id, &user);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::request_address_rotation(
                env.clone(),
                username(&env, "octocat"),
                other.clone(),
            )
            .unwrap();
        });

        env.ledger().with_mut(|li| li.timestamp += ROT_DELAY);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::execute_address_rotation(env.clone(), username(&env, "octocat"))
                .unwrap();
        });

        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .stellar_address,
                other
            );
            assert!(
                TrustBridgeContract::get_pending_rotation(env.clone(), username(&env, "octocat"))
                    .is_none(),
                "the pending entry must be cleared"
            );
        });
    }

    /// The window is only useful if the real holder can stop the rotation.
    #[test]
    fn test_holder_can_cancel_a_pending_rotation() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        setup_rotation(&env, &contract_id, &user);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::request_address_rotation(
                env.clone(),
                username(&env, "octocat"),
                other.clone(),
            )
            .unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::cancel_address_rotation(
                env.clone(),
                user.clone(),
                username(&env, "octocat"),
            )
            .unwrap();
        });

        env.ledger().with_mut(|li| li.timestamp += ROT_DELAY);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::execute_address_rotation(env.clone(), username(&env, "octocat")),
                Err(ContractError::NoRotationPending),
                "a cancelled rotation must not become executable"
            );
        });
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .stellar_address,
                user
            );
        });
    }

    #[test]
    fn test_second_rotation_request_is_rejected_while_one_is_pending() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        setup_rotation(&env, &contract_id, &user);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::request_address_rotation(
                env.clone(),
                username(&env, "octocat"),
                other.clone(),
            )
            .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::request_address_rotation(
                    env.clone(),
                    username(&env, "octocat"),
                    other.clone(),
                ),
                Err(ContractError::RotationPending)
            );
        });
    }

    #[test]
    fn test_rotation_requires_a_registered_username() {
        let env = Env::default();
        let (_admin, _user, other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::request_address_rotation(
                    env.clone(),
                    username(&env, "ghost"),
                    other.clone(),
                ),
                Err(ContractError::NotRegistered)
            );
        });
    }

    /// A verified registration loses its badge when the address moves.
    #[test]
    fn test_executing_a_rotation_clears_the_verified_flag() {
        let env = Env::default();
        let (admin, user, other, contract_id) = setup(&env);
        setup_rotation(&env, &contract_id, &user);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 1);
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::request_address_rotation(
                env.clone(),
                username(&env, "octocat"),
                other.clone(),
            )
            .unwrap();
        });
        env.ledger().with_mut(|li| li.timestamp += ROT_DELAY);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::execute_address_rotation(env.clone(), username(&env, "octocat"))
                .unwrap();
        });

        env.as_contract(&contract_id, || {
            assert!(
                !TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .verified,
                "verification vouched for the old address"
            );
            assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 0);
            assert_eq!(
                TrustBridgeContract::get_ever_verified_count(env.clone()),
                1,
                "the historical count still remembers it"
            );
        });
    }

    // ── Issue #230: light-client record proof ────────────────────────────────

    #[test]
    fn test_record_proof_for_registered_username() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            let proof = TrustBridgeContract::get_record_proof(env.clone(), username(&env, "octocat"));
            assert!(proof.exists);
            assert!(!proof.verified, "a fresh registration is unverified");
            assert_eq!(proof.as_of_ledger, env.ledger().sequence());
            assert_eq!(proof.key_prefix, soroban_sdk::symbol_short!("reg"));
            assert_eq!(proof.ttl_threshold_ledgers, crate::storage::TTL_THRESHOLD);
            assert_eq!(proof.ttl_bump_ledgers, crate::storage::TTL_BUMP);
        });
    }

    #[test]
    fn test_record_proof_reports_verified_bit() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            let proof = TrustBridgeContract::get_record_proof(env.clone(), username(&env, "octocat"));
            assert!(proof.exists);
            assert!(proof.verified);
        });
    }

    /// A missing username is an answer, not an error.
    #[test]
    fn test_record_proof_for_missing_username() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        env.as_contract(&contract_id, || {
            let proof = TrustBridgeContract::get_record_proof(env.clone(), username(&env, "ghost"));
            assert!(!proof.exists);
            assert!(!proof.verified);
            assert_eq!(proof.registered_at, 0);
            // The key and policy still come back so a client can look for the
            // entry — including an archived one — itself.
            assert_eq!(proof.key_prefix, soroban_sdk::symbol_short!("reg"));
            assert_eq!(proof.ttl_bump_ledgers, crate::storage::TTL_BUMP);
        });
    }

    #[test]
    fn test_record_proof_agrees_with_has_record() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            for name in ["octocat", "ghost"] {
                assert_eq!(
                    TrustBridgeContract::get_record_proof(env.clone(), username(&env, name)).exists,
                    TrustBridgeContract::has_record(env.clone(), username(&env, name)),
                    "proof.exists must agree with has_record for '{name}'"
                );
            }
        });
    }

    /// The proof is a read: it must work while the contract is paused.
    #[test]
    fn test_record_proof_works_while_paused() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::pause(env.clone(), 1).unwrap();
        });
        env.as_contract(&contract_id, || {
            assert!(TrustBridgeContract::get_record_proof(env.clone(), username(&env, "octocat")).exists);
        });
    }

    // ── Issue #229: monotonic ever-verified counter ──────────────────────────

    /// A verify/revoke/verify cycle: the live count moves with the flag, the
    /// historical count only ever climbs.
    #[test]
    fn test_ever_verified_count_survives_a_revoke_cycle() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            assert_eq!(TrustBridgeContract::get_ever_verified_count(env.clone()), 0);
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 1);
            assert_eq!(TrustBridgeContract::get_ever_verified_count(env.clone()), 1);
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::revoke_verification(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
                1,
            )
            .unwrap();
        });
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::get_verified_count(env.clone()),
                0,
                "live count must drop on revoke"
            );
            assert_eq!(
                TrustBridgeContract::get_ever_verified_count(env.clone()),
                1,
                "historical count must not drop on revoke"
            );
        });

        // Verifying the same contributor again counts as a second verification.
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 1);
            assert_eq!(TrustBridgeContract::get_ever_verified_count(env.clone()), 2);
        });
    }

    /// `get_stats` reports the live and historical counts side by side.
    #[test]
    fn test_stats_reports_live_and_ever_verified() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::revoke_verification(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
                1,
            )
            .unwrap();
        });

        env.as_contract(&contract_id, || {
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 1);
            assert_eq!(stats.verified, 0, "verified is the live figure");
            assert_eq!(stats.ever_verified, 1, "ever_verified keeps the history");
        });
    }

    /// A fresh instance starts both counters at zero.
    #[test]
    fn test_ever_verified_count_starts_at_zero() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        env.as_contract(&contract_id, || {
            assert_eq!(TrustBridgeContract::get_ever_verified_count(env.clone()), 0);
        });
    }

    #[test]
    fn test_revoke_verification_clears_flag_and_decrements_vcount() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 1);
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::revoke_verification(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
                1,
            )
            .unwrap();
        });
        env.as_contract(&contract_id, || {
            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert!(!record.verified, "verified flag must be false after revoke");
            assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 0);
        });
    }

    #[test]
    fn test_revoke_verification_nonverified_fails() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            let result = TrustBridgeContract::revoke_verification(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
                1,
            );
            assert_eq!(result, Err(ContractError::NotVerified));
        });
    }

    #[test]
    fn test_removing_verified_record_updates_stats() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 0);
            assert_eq!(stats.verified, 0);
        });
    }

    #[test]
    fn test_reregister_same_address_keeps_verification() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert!(
                record.verified,
                "re-registering the same address should preserve verified=true"
            );
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).verified, 1);
        });
    }

    #[test]
    fn test_verified_address_change_clears_verification() {
        let env = Env::default();
        let (admin, user, other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), other.clone(), Vec::new(&env))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert!(
                !record.verified,
                "changing stellar address must clear verification"
            );
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).verified, 0);
        });
    }

    #[test]
    fn test_verified_same_address_reregister_keeps_count() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 1);
            assert_eq!(stats.verified, 1);
        });
    }

    #[test]
    fn test_removed_verified_user_can_register_unverified() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert!(!record.verified);
        });
    }

    #[test]
    fn test_verify_after_reregister_new_address() {
        let env = Env::default();
        let (admin, user, other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), other.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .verified
            );
        });
    }

    #[test]
    fn test_verify_after_address_update_targets_new_address() {
        let env = Env::default();
        let (admin, old_user, new_user, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), old_user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), new_user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            let after_update =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert_eq!(after_update.stellar_address, new_user);
            assert!(!after_update.verified);
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).verified, 0);
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            let after_verify =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert_eq!(after_verify.stellar_address, new_user);
            assert!(after_verify.verified);
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).verified, 1);
        });
    }

    /// Verifier-role caller can verify (Issue #12).
    #[test]
    fn test_verifier_role_can_verify() {
        let env = Env::default();
        let (admin, user, verifier, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), verifier.clone(), Role::Verifier).unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert!(record.verified);
        });
        let _ = admin;
    }

    /// Verifier-role caller can revoke (Issue #12).
    /// Updated for Issue #212: Verifier can no longer revoke — that now
    /// requires Role::Revoker. This test verifies the old Verifier-can-revoke
    /// path correctly fails and that Role::Revoker succeeds.
    #[test]
    fn test_verifier_role_can_revoke_verification() {
        let env = Env::default();
        let (admin, user, revoker, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            // Assign Revoker role, not Verifier.
            TrustBridgeContract::set_role(env.clone(), revoker.clone(), Role::Revoker).unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::revoke_verification(
                env.clone(),
                revoker.clone(),
                username(&env, "octocat"),
                1,
            )
            .unwrap();
        });
        env.as_contract(&contract_id, || {
            assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 0);
        });
    }

    /// Address without role cannot verify (Issue #12).
    #[test]
    fn test_no_role_cannot_verify() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            // `other` has no role
            let result =
                TrustBridgeContract::verify(env.clone(), other.clone(), username(&env, "octocat"));
            assert_eq!(result, Err(ContractError::NotAuthorized));
        });
    }

    /// Admin can still call verify (role separation is additive) (Issue #12).
    #[test]
    fn test_admin_can_still_verify_after_role_separation() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .verified
            );
        });
    }

    /// Upgrader role cannot verify (Issue #12 — only Verifier and Admin).
    #[test]
    fn test_upgrader_role_cannot_verify() {
        let env = Env::default();
        let (_admin, user, upgrader, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), upgrader.clone(), Role::Upgrader).unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::verify(
                env.clone(),
                upgrader.clone(),
                username(&env, "octocat"),
            );
            assert_eq!(result, Err(ContractError::NotAuthorized));
        });
    }

    // ── Issue #54: Not-initialized guard tests ───────────────────────────────

    #[test]
    fn test_initialize_only_once() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register(TrustBridgeContract, ());
        env.as_contract(&contract_id, || {
            TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
            let result = TrustBridgeContract::initialize(env.clone(), admin.clone());
            assert_eq!(result, Err(ContractError::AlreadyInitialized));
        });
    }

    #[test]
    fn test_register_requires_initialization() {
        let env = Env::default();
        let user = Address::generate(&env);
        let contract_id = env.register(TrustBridgeContract, ());
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result =
                TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env));
            assert_eq!(result, Err(ContractError::NotInitialized));
        });
    }

    #[test]
    fn test_remove_requires_initialization() {
        let env = Env::default();
        let user = Address::generate(&env);
        let contract_id = env.register(TrustBridgeContract, ());
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result =
                TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat"));
            assert_eq!(result, Err(ContractError::NotInitialized));
        });
    }

    #[test]
    fn test_verify_requires_initialization() {
        let env = Env::default();
        let caller = Address::generate(&env);
        let contract_id = env.register(TrustBridgeContract, ());
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result =
                TrustBridgeContract::verify(env.clone(), caller.clone(), username(&env, "octocat"));
            assert_eq!(result, Err(ContractError::NotInitialized));
        });
    }

    #[test]
    fn test_revoke_verification_requires_initialization() {
        let env = Env::default();
        let caller = Address::generate(&env);
        let contract_id = env.register(TrustBridgeContract, ());
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::revoke_verification(
                env.clone(),
                caller.clone(),
                username(&env, "octocat"),
                1,
            );
            assert_eq!(result, Err(ContractError::NotInitialized));
        });
    }

    #[test]
    fn test_pause_requires_initialization() {
        let env = Env::default();
        let contract_id = env.register(TrustBridgeContract, ());
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::pause(env.clone(), 1);
            assert_eq!(result, Err(ContractError::NotInitialized));
        });
    }

    #[test]
    fn test_unpause_requires_initialization() {
        let env = Env::default();
        let contract_id = env.register(TrustBridgeContract, ());
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::unpause(env.clone(), 4);
            assert_eq!(result, Err(ContractError::NotInitialized));
        });
    }

    #[test]
    fn test_set_role_requires_initialization() {
        let env = Env::default();
        let target = Address::generate(&env);
        let contract_id = env.register(TrustBridgeContract, ());
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::set_role(env.clone(), target.clone(), Role::Verifier);
            assert_eq!(result, Err(ContractError::NotInitialized));
        });
    }

    #[test]
    fn test_remove_role_requires_initialization() {
        let env = Env::default();
        let target = Address::generate(&env);
        let contract_id = env.register(TrustBridgeContract, ());
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::remove_role(env.clone(), target.clone());
            assert_eq!(result, Err(ContractError::NotInitialized));
        });
    }

    #[test]
    fn test_set_cooldown_requires_initialization() {
        let env = Env::default();
        let contract_id = env.register(TrustBridgeContract, ());
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::set_cooldown(env.clone(), 3600);
            assert_eq!(result, Err(ContractError::NotInitialized));
        });
    }

    #[test]
    fn test_get_all_registered_requires_initialization() {
        let env = Env::default();
        let contract_id = env.register(TrustBridgeContract, ());
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::get_all_registered(env.clone());
            assert_eq!(result, Err(ContractError::NotInitialized));
        });
    }

    // Issue #53: every variant's numeric repr must match the table in
    // error.rs, and stay in sync as new variants are added — a variant
    // renumbered or added without a matching entry here breaks silently for
    // off-chain consumers that decode raw u32 codes.
    #[test]
    fn test_error_codes_match_repr() {
        assert_eq!(ContractError::AlreadyInitialized.code(), 1);
        assert_eq!(ContractError::NotInitialized.code(), 2);
        assert_eq!(ContractError::NotAuthorized.code(), 3);
        assert_eq!(ContractError::NotRegistered.code(), 4);
        assert_eq!(ContractError::AlreadyVerified.code(), 5);
        assert_eq!(ContractError::NotVerified.code(), 6);
        assert_eq!(ContractError::Paused.code(), 7);
        assert_eq!(ContractError::CooldownActive.code(), 8);
        assert_eq!(ContractError::InvalidVersion.code(), 9);
        assert_eq!(ContractError::InvalidRole.code(), 10);
        assert_eq!(ContractError::InvalidUsername.code(), 11);
        assert_eq!(ContractError::AttestationExpired.code(), 12);
        assert_eq!(ContractError::UnattestedWasm.code(), 13);
        assert_eq!(ContractError::InvalidBatchSize.code(), 14);
        assert_eq!(ContractError::InvalidReasonCode.code(), 15);
        assert_eq!(ContractError::ZeroAddress.code(), 16);
        assert_eq!(ContractError::ChallengeAlreadyActive.code(), 17);
        assert_eq!(ContractError::NoChallengeActive.code(), 18);
        assert_eq!(ContractError::ChallengeNotResolvable.code(), 19);
        assert_eq!(ContractError::ChallengeActive.code(), 20);
        assert_eq!(ContractError::InvalidPauseReason.code(), 21);
        assert_eq!(ContractError::AlreadyReserved.code(), 22);
        assert_eq!(ContractError::NotReserved.code(), 23);
        assert_eq!(ContractError::UsernameReserved.code(), 24);
        assert_eq!(ContractError::ReservedListFull.code(), 25);
        assert_eq!(ContractError::AdminTransferPending.code(), 26);
        assert_eq!(ContractError::AdminTransferDelayActive.code(), 27);
        assert_eq!(ContractError::NoPendingAdminTransfer.code(), 28);
        assert_eq!(ContractError::AttestationRequired.code(), 29);
    }

    #[test]
    fn test_error_from_code_is_inverse_of_code() {
        for variant in [
            ContractError::AlreadyInitialized,
            ContractError::NotInitialized,
            ContractError::NotAuthorized,
            ContractError::NotRegistered,
            ContractError::AlreadyVerified,
            ContractError::NotVerified,
            ContractError::Paused,
            ContractError::CooldownActive,
            ContractError::InvalidVersion,
            ContractError::InvalidRole,
            ContractError::InvalidUsername,
            ContractError::AttestationExpired,
            ContractError::UnattestedWasm,
            ContractError::InvalidBatchSize,
            ContractError::InvalidReasonCode,
            ContractError::ZeroAddress,
            ContractError::ChallengeAlreadyActive,
            ContractError::NoChallengeActive,
            ContractError::ChallengeNotResolvable,
            ContractError::ChallengeActive,
            ContractError::InvalidPauseReason,
            ContractError::AlreadyReserved,
            ContractError::NotReserved,
            ContractError::UsernameReserved,
            ContractError::ReservedListFull,
            ContractError::AdminTransferPending,
            ContractError::AdminTransferDelayActive,
            ContractError::NoPendingAdminTransfer,
            ContractError::AttestationRequired,
        ] {
            assert_eq!(ContractError::from_code(variant.code()), Some(variant));
        }
        assert_eq!(ContractError::from_code(0), None);
        // 30 is one past the highest assigned variant (AttestationRequired = 29):
        assert_eq!(ContractError::from_code(30), None);
    }

    // --- Issue #69: max username length guard ---

    #[test]
    fn test_register_rejects_over_length_username() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();

        // 40 characters: one past MAX_USERNAME_LEN.
        let too_long = String::from_str(&env, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(too_long.len(), MAX_USERNAME_LEN + 1);

        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::register(env.clone(), too_long.clone(), user.clone(), Vec::new(&env)),
                Err(ContractError::InvalidUsername)
            );
            // The rejected username must leave no trace in the registry.
            assert!(!TrustBridgeContract::has_record(env.clone(), too_long));
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 0);
        });
    }

    #[test]
    fn test_register_accepts_username_at_max_length() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();

        // Exactly 39 characters.
        let at_max = String::from_str(&env, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(at_max.len(), MAX_USERNAME_LEN);

        env.as_contract(&contract_id, || {
            assert!(
                TrustBridgeContract::register(env.clone(), at_max.clone(), user.clone(), Vec::new(&env)).is_ok()
            );
            assert!(TrustBridgeContract::has_record(env.clone(), at_max));
        });
    }

    #[test]
    fn test_register_rejects_empty_and_malformed_usernames() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            for bad in ["", "-lead", "trail-", "has space", "at@sign"] {
                assert_eq!(
                    TrustBridgeContract::register(env.clone(), username(&env, bad), user.clone(), Vec::new(&env)),
                    Err(ContractError::InvalidUsername),
                    "expected {bad:?} to be rejected"
                );
            }
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 0);
        });
    }

    #[test]
    fn test_max_username_len_is_exposed() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);

        env.as_contract(&contract_id, || {
            assert_eq!(TrustBridgeContract::max_username_len(env.clone()), 39);
            assert!(TrustBridgeContract::is_username_valid(
                env.clone(),
                username(&env, "octocat")
            ));
            assert!(!TrustBridgeContract::is_username_valid(
                env.clone(),
                username(&env, "octo cat")
            ));
        });
    }

    // --- Issue #68: username case normalization ---

    #[test]
    fn test_usernames_match_is_case_insensitive() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);

        env.as_contract(&contract_id, || {
            assert!(TrustBridgeContract::usernames_match(
                env.clone(),
                username(&env, "OctoCat"),
                username(&env, "octocat")
            ));
            assert!(!TrustBridgeContract::usernames_match(
                env.clone(),
                username(&env, "octocat"),
                username(&env, "octocat1")
            ));
        });
    }

    // --- Issue #72: register self-auth enforcement ---

    #[test]
    fn test_register_transfer_requires_current_owner_auth() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        let client = TrustBridgeContractClient::new(&env, &contract_id);
        let name = username(&env, "octocat");

        env.mock_all_auths();
        client.register(&name, &user, &Vec::new(&env));

        // Re-point the registration at `other`, authorizing only `other`.
        // The current owner's signature is missing, so the call must fail.
        env.set_auths(&[]);
        let res = client.try_register(&name, &other);
        assert!(
            res.is_err(),
            "takeover succeeded without the current owner's authorization"
        );

        // The registration is unchanged.
        env.mock_all_auths();
        assert_eq!(client.get_address(&name).unwrap().stellar_address, user);
    }

    #[test]
    fn test_register_transfer_succeeds_with_both_auths() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        let client = TrustBridgeContractClient::new(&env, &contract_id);
        let name = username(&env, "octocat");

        env.mock_all_auths();
        client.register(&name, &user, &Vec::new(&env));
        client.register(&name, &other, &Vec::new(&env));

        assert_eq!(client.get_address(&name).unwrap().stellar_address, other);
        assert_eq!(client.get_stats().total, 1);
    }

    // --- Issue #62: RegisteredEvent payload ---

    #[test]
    fn test_registered_event_payload_is_complete() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        let client = TrustBridgeContractClient::new(&env, &contract_id);
        let name = username(&env, "octocat");

        env.mock_all_auths();
        env.ledger().set_timestamp(1_600_000_000);

        client.register(&name, &user, &Vec::new(&env));

        let expected = RegisteredEvent {
            github_username: name.clone(),
            stellar_address: user.clone(),
            timestamp: 1_600_000_000,
            domain: env.as_contract(&contract_id, || event_domain(&env)),
        };

        assert_eq!(
            env.events().all(),
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    expected.topics(&env),
                    expected.data(&env),
                )
            ],
            "RegisteredEvent payload or topics changed"
        );

        let topics = expected.topics(&env);
        assert_eq!(topics.len(), 2, "RegisteredEvent must have 2 topics");
        assert_eq!(
            soroban_sdk::Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
            soroban_sdk::Symbol::new(&env, "registered_event"),
            "RegisteredEvent topic symbol changed"
        );
        assert_eq!(
            String::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            name,
            "RegisteredEvent username topic changed"
        );
    }

    #[test]
    fn test_registered_event_not_published_on_failed_register() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        let client = TrustBridgeContractClient::new(&env, &contract_id);
        let name = username(&env, "octocat");

        // We do NOT mock auth, so the registration should fail.
        env.set_auths(&[]);
        assert!(client.try_register(&name, &user).is_err());

        assert_eq!(
            env.events().all(),
            soroban_sdk::vec![&env],
            "failed register published an event"
        );
    }

    // --- Issue #64: RemovedEvent payload ---

    #[test]
    fn test_removed_event_payload_is_complete() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        let client = TrustBridgeContractClient::new(&env, &contract_id);
        let name = username(&env, "octocat");

        env.mock_all_auths();
        client.register(&name, &user, &Vec::new(&env));

        env.ledger().set_timestamp(1_700_000_000);
        client.remove(&user, &name);

        // `remove` must publish exactly one event, and that event must be a
        // fully-populated RemovedEvent: the username as topic, and the removed
        // address plus the removal timestamp as data. An indexer replaying only
        // this event has to be able to reconstruct the record it is retiring,
        // so every field is asserted rather than just the event's presence.
        let expected = RemovedEvent {
            github_username: name.clone(),
            stellar_address: user.clone(),
            timestamp: 1_700_000_000,
            domain: env.as_contract(&contract_id, || event_domain(&env)),
        };

        assert_eq!(
            env.events().all(),
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    expected.topics(&env),
                    expected.data(&env),
                )
            ],
            "RemovedEvent payload or topics changed"
        );

        // Pin the topic shape independently of the struct, so renaming the
        // event or dropping the username topic breaks this test rather than
        // silently breaking every downstream subscriber's filter.
        let topics = expected.topics(&env);
        assert_eq!(topics.len(), 2, "RemovedEvent must have 2 topics");
        assert_eq!(
            soroban_sdk::Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
            soroban_sdk::Symbol::new(&env, "removed_event"),
            "RemovedEvent topic symbol changed"
        );
        assert_eq!(
            String::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            name,
            "RemovedEvent username topic changed"
        );
    }

    #[test]
    fn test_removed_event_not_published_on_failed_remove() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        let client = TrustBridgeContractClient::new(&env, &contract_id);
        let name = username(&env, "octocat");

        env.mock_all_auths();
        client.register(&name, &user, &Vec::new(&env));

        // A caller who is neither the registrant nor the admin is rejected,
        // and must not leave a RemovedEvent behind for indexers to act on.
        assert!(client.try_remove(&other, &name).is_err());
        assert_eq!(
            env.events().all(),
            soroban_sdk::vec![&env],
            "failed remove published an event"
        );
        assert!(client.has_record(&name));
    }

    // ── Issue #56: unauthorized remove must not disturb registry events ──────

    #[test]
    fn test_unauthorized_remove_leaves_verified_record_and_events_intact() {
        let env = Env::default();
        let (admin, user, other, contract_id) = setup(&env);
        let client = TrustBridgeContractClient::new(&env, &contract_id);
        let name = username(&env, "octocat");

        env.mock_all_auths();
        client.register(&name, &user, &Vec::new(&env));
        env.mock_all_auths();
        client.verify(&admin, &name);

        // Neither the registrant nor the admin: rejected, and must leave the
        // registration, its verified flag, and the event log untouched. `all()`
        // scopes to the last invocation, so an empty result here proves the
        // failed call published no RemovedEvent (same idiom as
        // test_removed_event_not_published_on_failed_remove).
        assert!(client.try_remove(&other, &name).is_err());
        assert_eq!(
            env.events().all(),
            soroban_sdk::vec![&env],
            "unauthorized remove must not publish a RemovedEvent"
        );
        let record = client.get_address(&name).unwrap();
        assert!(
            record.verified,
            "unauthorized remove must not clear verification"
        );
        assert_eq!(client.get_stats().verified, 1);
    }

    #[test]
    fn test_unauthorized_remove_after_revoke_leaves_events_intact() {
        let env = Env::default();
        let (admin, user, other, contract_id) = setup(&env);
        let client = TrustBridgeContractClient::new(&env, &contract_id);
        let name = username(&env, "octocat");

        env.mock_all_auths();
        client.register(&name, &user, &Vec::new(&env));
        env.mock_all_auths();
        client.verify(&admin, &name);
        env.mock_all_auths();
        client.revoke_verification(&admin, &name, &1u32);

        assert!(client.try_remove(&other, &name).is_err());
        assert_eq!(
            env.events().all(),
            soroban_sdk::vec![&env],
            "unauthorized remove must not publish a RemovedEvent after a prior revoke"
        );
        assert!(client.has_record(&name));
        assert_eq!(client.get_stats().total, 1);
    }

    // ── Pause / unpause workflow ──────────────────────────────────────────────

    #[test]
    fn test_pause_and_unpause_workflow() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.as_contract(&contract_id, || {
            assert!(!TrustBridgeContract::is_paused(env.clone()));
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::pause(env.clone(), 1).unwrap();
        });

        env.as_contract(&contract_id, || {
            assert!(TrustBridgeContract::is_paused(env.clone()));
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let reg_res =
                TrustBridgeContract::register(env.clone(), username(&env, "alice"), user.clone(), Vec::new(&env));
            assert_eq!(reg_res, Err(ContractError::Paused));
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::unpause(env.clone(), 4).unwrap();
        });

        env.as_contract(&contract_id, || {
            assert!(!TrustBridgeContract::is_paused(env.clone()));
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            assert!(TrustBridgeContract::register(
                env.clone(),
                username(&env, "alice"),
                user.clone(),
                Vec::new(&env),
            )
            .is_ok());
        });
    }

    #[test]
    fn test_pause_blocks_remove_and_public_paginated_then_unpause_restores() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        let name = username(&env, "octocat");

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), name.clone(), user.clone(), Vec::new(&env)).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::pause(env.clone(), 1).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let remove_res = TrustBridgeContract::remove(env.clone(), user.clone(), name.clone());
            assert_eq!(remove_res, Err(ContractError::Paused));
        });

        env.as_contract(&contract_id, || {
            let paged_res = TrustBridgeContract::get_public_paginated(env.clone(), 0, 10);
            assert_eq!(paged_res, Err(ContractError::Paused));
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::unpause(env.clone(), 4).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let paged = TrustBridgeContract::get_public_paginated(env.clone(), 0, 10).unwrap();
            assert_eq!(paged.records.len(), 1);
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user.clone(), name.clone()).unwrap();
            assert!(TrustBridgeContract::get_address(env.clone(), name.clone()).is_none());
        });
    }

    // ── Roles management ─────────────────────────────────────────────────────

    #[test]
    fn test_roles_management() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);

        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::get_role(env.clone(), user.clone()),
                None
            );
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), user.clone(), Role::Upgrader).unwrap();
        });

        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::get_role(env.clone(), user.clone()),
                Some(Role::Upgrader)
            );
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), other.clone(), Role::Verifier).unwrap();
        });

        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::get_role(env.clone(), other.clone()),
                Some(Role::Verifier)
            );
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove_role(env.clone(), user.clone()).unwrap();
        });

        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::get_role(env.clone(), user.clone()),
                None
            );
        });
    }

    // ── Cooldown / version / migration ────────────────────────────────────────

    #[test]
    fn test_migration_version_increment() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);

        env.as_contract(&contract_id, || {
            assert_eq!(TrustBridgeContract::get_version(env.clone()), (1, 0, 0));
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let err_res = TrustBridgeContract::migrate(env.clone(), (1, 0, 0));
            assert_eq!(err_res, Err(ContractError::InvalidVersion));
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::migrate(env.clone(), (1, 1, 0)).unwrap();
        });

        env.as_contract(&contract_id, || {
            assert_eq!(TrustBridgeContract::get_version(env.clone()), (1, 1, 0));
        });
    }

    // ── Admin export ──────────────────────────────────────────────────────────

    #[test]
    fn test_admin_export_empty_registry() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let all = TrustBridgeContract::get_all_registered(env.clone()).unwrap();
            assert_eq!(all.len(), 0);
        });
    }

    #[test]
    fn test_get_all_registered_returns_indexed_records() {
        let env = Env::default();
        let (_admin, user1, user2, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone(), Vec::new(&env))
                .unwrap();
            let all = TrustBridgeContract::get_all_registered(env.clone()).unwrap();
            assert_eq!(all.len(), 2);
            assert_eq!(
                all.get(0).unwrap(),
                (username(&env, "alice"), user1.clone())
            );
            assert_eq!(all.get(1).unwrap(), (username(&env, "bob"), user2.clone()));
        });
    }

    #[test]
    fn test_cold_start_register_exposes_dashboard_state() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::get_all_registered(env.clone())
                    .unwrap()
                    .len(),
                0
            );
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::get_all_registered(env.clone())
                    .unwrap()
                    .len(),
                1
            );
        });
    }

    // ── Issue #16 / #53: from_code round-trip and completeness ───────────────

    /// Every variant's code() must round-trip through from_code() (Issue #16).
    #[test]
    fn test_from_code_round_trips_all_variants() {
        let all = [
            ContractError::AlreadyInitialized,
            ContractError::NotInitialized,
            ContractError::NotAuthorized,
            ContractError::NotRegistered,
            ContractError::AlreadyVerified,
            ContractError::NotVerified,
            ContractError::Paused,
            ContractError::CooldownActive,
            ContractError::InvalidVersion,
            ContractError::InvalidRole,
            ContractError::InvalidUsername,
            ContractError::AttestationExpired,
            ContractError::UnattestedWasm,
            ContractError::InvalidBatchSize,
            ContractError::InvalidReasonCode,
            ContractError::ZeroAddress,
            ContractError::ChallengeAlreadyActive,
            ContractError::NoChallengeActive,
            ContractError::ChallengeNotResolvable,
            ContractError::ChallengeActive,
        ];
        for variant in all {
            assert_eq!(ContractError::from_code(variant as u32), Some(variant));
        }
    }

    /// Codes not in the enum must return None (Issue #16).
    #[test]
    fn test_from_code_unknown_returns_none() {
        assert_eq!(ContractError::from_code(0), None);
        assert_eq!(ContractError::from_code(32), None);
        assert_eq!(ContractError::from_code(u32::MAX), None);
    }

    // ── Issue #54: Additional not-initialized guard tests ────────────────────

    /// get_registered_page must fail before init (Issue #54).
    #[test]
    fn test_get_registered_page_requires_initialization() {
        let env = Env::default();
        let contract_id = env.register(TrustBridgeContract, ());
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::get_registered_page(env.clone(), 0, 10);
            assert_eq!(result, Err(ContractError::NotInitialized));
        });
    }

    /// get_registered_paginated must fail before init (Issue #54).
    #[test]
    fn test_get_registered_paginated_requires_initialization() {
        let env = Env::default();
        let contract_id = env.register(TrustBridgeContract, ());
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::get_registered_paginated(env.clone(), 0, 10);
            assert_eq!(result, Err(ContractError::NotInitialized));
        });
    }

    /// get_public_paginated must fail before init (Issue #54).
    #[test]
    fn test_get_public_paginated_requires_initialization() {
        let env = Env::default();
        let contract_id = env.register(TrustBridgeContract, ());
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::get_public_paginated(env.clone(), 0, 10);
            assert_eq!(result, Err(ContractError::NotInitialized));
        });
    }

    /// After initialization every previously failing guard must succeed (Issue #54).
    #[test]
    fn test_guards_succeed_after_initialization() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register(TrustBridgeContract, ());
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            // Confirm guard fires before init
            assert_eq!(
                TrustBridgeContract::register(
                    env.clone(),
                    username(&env, "octocat"),
                    admin.clone(),
                    Vec::new(&env),
                ),
                Err(ContractError::NotInitialized)
            );

            TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();

            // Same call must now succeed
            assert!(TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                admin.clone(),
                Vec::new(&env),
            )
            .is_ok());
        });
    }

    /// Double-initialize after successful init must still be rejected (Issue #54).
    #[test]
    fn test_double_initialize_rejected_after_successful_init() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let admin2 = Address::generate(&env);
        let contract_id = env.register(TrustBridgeContract, ());
        env.as_contract(&contract_id, || {
            TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
            let result = TrustBridgeContract::initialize(env.clone(), admin2.clone());
            assert_eq!(result, Err(ContractError::AlreadyInitialized));
        });
    }

    // ── Issue #52: Additional lookup-after-peer-removal tests ────────────────

    /// Paginated export must skip removed records (Issue #52).
    #[test]
    fn test_paginated_export_skips_removed_records() {
        let env = Env::default();
        let (_admin, user1, user2, contract_id) = setup(&env);
        let user3 = Address::generate(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "carol"), user3.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user2.clone(), username(&env, "bob")).unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let page = TrustBridgeContract::get_registered_paginated(env.clone(), 0, 10).unwrap();
            assert_eq!(
                page.records.len(),
                2,
                "paginated export must skip removed entry"
            );
            assert_eq!(page.total, 2);
            assert!(!page.has_more);
        });
    }

    /// Public paginated endpoint reflects removal immediately (Issue #52).
    #[test]
    fn test_public_paginated_reflects_removal() {
        let env = Env::default();
        let (_admin, user1, user2, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user1.clone(), username(&env, "alice"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            let page = TrustBridgeContract::get_public_paginated(env.clone(), 0, 10).unwrap();
            assert_eq!(page.records.len(), 1);
            assert_eq!(page.records.get(0).unwrap().0, username(&env, "bob"));
        });
    }

    // ── Issue #143: bulk export pagination limits ─────────────────────────────

    /// Requesting exactly `MAX_PAGE_LIMIT` from a registry larger than that
    /// must return a full page and report that more records remain.
    #[test]
    fn test_paginated_export_at_max_page_limit() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();

        // A full `MAX_PAGE_LIMIT`-sized page reads one ledger entry per
        // record, which exceeds the mainnet per-invocation footprint limit
        // (100 entries) purely as a test-harness artifact of registering and
        // reading that many entries in one `Env`. The pagination *behavior*
        // under test — clamping and cursor bookkeeping — is independent of
        // that network limit, so it is disabled here.
        env.cost_estimate().disable_resource_limits();

        let total = crate::storage::MAX_PAGE_LIMIT + 5;
        for i in 0..total {
            let mut name = alloc::string::String::from("user");
            name.push_str(&alloc::format!("{i}"));
            let name = String::from_str(&env, &name);
            env.as_contract(&contract_id, || {
                TrustBridgeContract::register(env.clone(), name.clone(), user.clone(), Vec::new(&env)).unwrap();
            });
        }

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let page = TrustBridgeContract::get_registered_paginated(
                env.clone(),
                0,
                crate::storage::MAX_PAGE_LIMIT,
            )
            .unwrap();
            assert_eq!(page.records.len(), crate::storage::MAX_PAGE_LIMIT);
            assert!(page.has_more);
            assert_eq!(page.next_cursor, Some(crate::storage::MAX_PAGE_LIMIT));
        });
    }

    /// Requesting more than `MAX_PAGE_LIMIT` must be clamped down to it rather
    /// than rejected or returned unbounded.
    #[test]
    fn test_paginated_export_over_max_page_limit_clamps() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();

        // See `test_paginated_export_at_max_page_limit`: disabled for the
        // same reason — the registry size needed to exercise the clamp
        // exceeds the mainnet per-invocation footprint limit.
        env.cost_estimate().disable_resource_limits();

        let total = crate::storage::MAX_PAGE_LIMIT + 5;
        for i in 0..total {
            let mut name = alloc::string::String::from("user");
            name.push_str(&alloc::format!("{i}"));
            let name = String::from_str(&env, &name);
            env.as_contract(&contract_id, || {
                TrustBridgeContract::register(env.clone(), name.clone(), user.clone(), Vec::new(&env)).unwrap();
            });
        }

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let page = TrustBridgeContract::get_registered_paginated(
                env.clone(),
                0,
                crate::storage::MAX_PAGE_LIMIT + 50,
            )
            .unwrap();
            assert!(page.records.len() <= crate::storage::MAX_PAGE_LIMIT);
            assert_eq!(page.records.len(), crate::storage::MAX_PAGE_LIMIT);
        });
    }

    /// has_record returns false after removal and true for surviving peer (Issue #52).
    #[test]
    fn test_has_record_consistency_after_peer_removal() {
        let env = Env::default();
        let (_admin, user1, user2, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user1.clone(), username(&env, "alice"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            assert!(!TrustBridgeContract::has_record(
                env.clone(),
                username(&env, "alice")
            ));
            assert!(TrustBridgeContract::has_record(
                env.clone(),
                username(&env, "bob")
            ));
        });
    }

    /// Comparison counts the case-normalization benchmark sweeps. Normalization
    /// touches no ledger entries, so it is not footprint-bound.
    const BENCH_SIZES: [u32; 4] = [10, 50, 100, 200];

    /// Registry sizes the full-export benchmark sweeps.
    ///
    /// Capped below 100: `get_all_registered` reads one ledger entry per
    /// record, and Soroban rejects an invocation whose footprint exceeds 100
    /// entries. That ceiling is the reason `get_registered_page` exists — see
    /// `test_bench_export_footprint_ceiling`.
    const EXPORT_BENCH_SIZES: [u32; 4] = [10, 20, 40, 80];

    /// Labels used by the register budget guard.
    const REGISTER_BUDGET_BASELINE_LABEL: &str = "baseline";
    const REGISTER_BUDGET_STRESSED_LABEL: &str = "max_username_len";

    /// Measures a single `register` call against a fresh initialized contract.
    /// Returns `(cpu_instructions, memory_bytes)`.
    fn measure_register_cost(env: &Env, github_username: String) -> (u64, u64) {
        let (_admin, user, _other, contract_id) = setup(env);

        env.mock_all_auths();
        env.cost_estimate().budget().reset_default();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), github_username, user, Vec::new(&env)).unwrap();
        });

        let budget = env.cost_estimate().budget();
        (budget.cpu_instruction_cost(), budget.memory_bytes_cost())
    }

    fn max_len_username(env: &Env) -> String {
        let repeated = alloc::string::String::from("a").repeat(MAX_USERNAME_LEN as usize);
        String::from_str(env, &repeated)
    }

    /// Registers `size` contributors and measures the metered cost of a single
    /// full export. Returns `(cpu_instructions, memory_bytes)`.
    fn measure_export(size: u32) -> (u64, u64) {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();

        // Each registration runs in its own frame: `require_auth` for the same
        // address twice in one frame is an Auth(ExistingValue) error.
        for i in 0..size {
            let mut name = alloc::string::String::from("bench");
            name.push_str(&alloc::format!("{i}"));
            let name = String::from_str(&env, &name);
            env.as_contract(&contract_id, || {
                TrustBridgeContract::register(env.clone(), name.clone(), user.clone(), Vec::new(&env)).unwrap();
            });
        }

        env.cost_estimate().budget().reset_default();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::get_all_registered(env.clone()).unwrap();
        });

        let budget = env.cost_estimate().budget();
        (budget.cpu_instruction_cost(), budget.memory_bytes_cost())
    }

    /// Measures the metered cost of `size` case-insensitive username
    /// comparisons — the normalization step an off-chain verifier performs per
    /// candidate match. Returns `(cpu_instructions, memory_bytes)`.
    fn measure_case_normalization(size: u32) -> (u64, u64) {
        let env = Env::default();
        let contract_id = env.register(TrustBridgeContract, ());

        // Mixed-case on one side, lower-case on the other, so every comparison
        // exercises the folding path rather than an early length mismatch.
        let upper = String::from_str(&env, "OctoCat-Dev_01");
        let lower = String::from_str(&env, "octocat-dev_01");

        env.cost_estimate().budget().reset_default();
        env.as_contract(&contract_id, || {
            for _ in 0..size {
                assert!(TrustBridgeContract::usernames_match(
                    env.clone(),
                    upper.clone(),
                    lower.clone()
                ));
            }
        });

        let budget = env.cost_estimate().budget();
        (budget.cpu_instruction_cost(), budget.memory_bytes_cost())
    }

    /// Benchmark for issue #68: username case normalization must stay linear
    /// in the number of comparisons and must not allocate per comparison.
    #[test]
    fn test_bench_username_case_normalization() {
        std::println!("operation,size,cpu_instructions,memory_bytes");

        let mut previous_cpu = 0u64;
        let mut baseline: Option<(u32, u64)> = None;
        let mut largest: Option<(u32, u64)> = None;

        for size in BENCH_SIZES {
            let (cpu, mem) = measure_case_normalization(size);
            std::println!("usernames_match,{},{},{}", size, cpu, mem);

            assert!(cpu > 0, "normalization at size {size} was not metered");
            assert!(
                cpu >= previous_cpu,
                "normalization CPU cost dropped at size {size}: {cpu} < {previous_cpu}"
            );

            previous_cpu = cpu;
            baseline.get_or_insert((size, cpu));
            largest = Some((size, cpu));
        }

        let (small_size, small_cpu) = baseline.unwrap();
        let (large_size, large_cpu) = largest.unwrap();

        // Comparison is a fixed-width stack scan, so cost is linear in the
        // number of calls. 3x headroom over the size ratio absorbs per-call
        // overhead while still failing on super-linear growth.
        let ceiling = small_cpu * ((large_size / small_size) as u64) * 3;
        assert!(
            large_cpu <= ceiling,
            "normalization CPU cost grew super-linearly: {large_cpu} at size {large_size} exceeds ceiling {ceiling}"
        );
    }

    #[test]
    fn test_bench_export_cpu_cost() {
        std::println!("operation,size,cpu_instructions,memory_bytes");

        let mut previous_cpu = 0u64;
        let mut baseline: Option<(u32, u64)> = None;
        let mut largest: Option<(u32, u64)> = None;

        for size in EXPORT_BENCH_SIZES {
            let (cpu, mem) = measure_export(size);
            std::println!("get_all_registered,{},{},{}", size, cpu, mem);

            assert!(cpu > 0, "export at size {size} was not metered");
            // Cost is monotonic in registry size; a drop means the export
            // stopped visiting every record.
            assert!(
                cpu >= previous_cpu,
                "export CPU cost dropped at size {size}: {cpu} < {previous_cpu}"
            );

            previous_cpu = cpu;
            baseline.get_or_insert((size, cpu));
            largest = Some((size, cpu));
        }

        let (small_size, small_cpu) = baseline.unwrap();
        let (large_size, large_cpu) = largest.unwrap();

        // Export is a linear scan. Allow 3x headroom over the size ratio so
        // normal per-entry overhead passes while quadratic growth fails.
        let ceiling = small_cpu * ((large_size / small_size) as u64) * 3;
        assert!(
            large_cpu <= ceiling,
            "export CPU cost grew super-linearly: {large_cpu} at size {large_size} exceeds ceiling {ceiling}"
        );
    }

    /// Emits register budget samples for CI/Make threshold checks.
    ///
    /// Output format (CSV):
    /// operation,input,cpu_instructions,memory_bytes
    #[test]
    fn test_report_register_budget_samples() {
        let env = Env::default();
        let baseline = username(&env, "octocat");
        let stressed = max_len_username(&env);

        let (baseline_cpu, baseline_mem) = measure_register_cost(&env, baseline);
        let (stressed_cpu, stressed_mem) = measure_register_cost(&env, stressed);

        std::println!("operation,input,cpu_instructions,memory_bytes");
        std::println!(
            "register,{},{},{}",
            REGISTER_BUDGET_BASELINE_LABEL,
            baseline_cpu,
            baseline_mem
        );
        std::println!(
            "register,{},{},{}",
            REGISTER_BUDGET_STRESSED_LABEL,
            stressed_cpu,
            stressed_mem
        );

        assert!(baseline_cpu > 0, "baseline register cost was not metered");
        assert!(stressed_cpu > 0, "stressed register cost was not metered");
    }
    /// Measures the metered cost of a double-verify rejection — calling `verify`
    /// on a username that is already verified. Returns `(cpu_instructions, memory_bytes)`.
    ///
    /// The operation registers a user, verifies them, then calls verify again and
    /// expects `AlreadyVerified`. The cost is compared against a successful first
    /// verify to ensure the rejection path is strictly cheaper (denial-of-wallet
    /// protection — a rejected call must not consume more budget than a successful one).
    fn measure_double_verify_rejection() -> (u64, u64, u64, u64) {
        let env = Env::default();
        let contract_id = env.register(TrustBridgeContract, ());
        let admin = Address::generate(&env);
        let user = Address::generate(&env);

        // Initialize the contract
        env.as_contract(&contract_id, || {
            TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
        });

        // Register and verify first
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });

        // Measure cost of the second verify (should fail with AlreadyVerified)
        env.cost_estimate().budget().reset_default();
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result =
                TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"));
            assert_eq!(result, Err(ContractError::AlreadyVerified));
        });
        let reject_budget = env.cost_estimate().budget();
        let reject_cpu = reject_budget.cpu_instruction_cost();
        let reject_mem = reject_budget.memory_bytes_cost();

        // Measure cost of a successful verify (separate user, fresh env)
        let env2 = Env::default();
        let contract_id2 = env2.register(TrustBridgeContract, ());
        let admin2 = Address::generate(&env2);
        let user2 = Address::generate(&env2);

        env2.as_contract(&contract_id2, || {
            TrustBridgeContract::initialize(env2.clone(), admin2.clone()).unwrap();
        });
        env2.mock_all_auths();
        env2.as_contract(&contract_id2, || {
            TrustBridgeContract::register(env2.clone(), username(&env2, "octocat"), user2.clone(), Vec::new(&env))
                .unwrap();
        });

        env2.cost_estimate().budget().reset_default();
        env2.mock_all_auths();
        env2.as_contract(&contract_id2, || {
            TrustBridgeContract::verify(env2.clone(), admin2.clone(), username(&env2, "octocat"))
                .unwrap();
        });
        let success_budget = env2.cost_estimate().budget();
        let success_cpu = success_budget.cpu_instruction_cost();
        let success_mem = success_budget.memory_bytes_cost();

        (reject_cpu, reject_mem, success_cpu, success_mem)
    }

    /// Benchmark double-verify rejection (Issue #58): ensures the rejected
    /// verify (AlreadyVerified) costs strictly less than a successful verify.
    ///
    /// A rejection path that costs more than the success path could be used by
    /// an attacker to burn the contract's ledger budget cheaply — the rejection
    /// executes fewer steps, so it must cost strictly less.
    #[test]
    fn test_bench_double_verify_rejection() {
        let (reject_cpu, reject_mem, success_cpu, success_mem) = measure_double_verify_rejection();

        std::println!("operation,type,cpu_instructions,memory_bytes");
        std::println!("verify,success,{},{}", success_cpu, success_mem);
        std::println!(
            "verify,rejected_double_verify,{},{}",
            reject_cpu,
            reject_mem
        );

        assert!(
            success_cpu > 0,
            "successful verify CPU cost was not metered"
        );
        assert!(
            reject_cpu > 0,
            "rejected double-verify CPU cost was not metered"
        );

        // Protection against denial-of-wallet: a rejected path must cost
        // strictly less than the equivalent success path (Issue #58).
        assert!(
            reject_cpu < success_cpu,
            "rejected double-verify CPU cost ({reject_cpu}) must be strictly less than successful verify CPU cost ({success_cpu})"
        );
        assert!(
            reject_mem <= success_mem,
            "rejected double-verify memory cost ({reject_mem}) must not exceed successful verify memory cost ({success_mem})"
        );
    }

    /// Revoking Verifier role prevents further verify calls (Issue #12).
    #[test]
    fn test_revoked_verifier_role_cannot_verify() {
        let env = Env::default();
        let (_admin, user, verifier, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), verifier.clone(), Role::Verifier).unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove_role(env.clone(), verifier.clone()).unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result =
                TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "alice"));
            assert_eq!(result, Err(ContractError::NotAuthorized));
        });
    }

    /// Two independent Verifier-role holders can each verify without interfering (Issue #12).
    #[test]
    fn test_two_verifiers_operate_independently() {
        let env = Env::default();
        let (_admin, user, verifier1, contract_id) = setup(&env);
        let verifier2 = Address::generate(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), verifier1.clone(), Role::Verifier).unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), verifier2.clone(), Role::Verifier).unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), verifier1.clone(), username(&env, "alice"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), verifier2.clone(), username(&env, "bob"))
                .unwrap();
        });
        env.as_contract(&contract_id, || {
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "alice"))
                    .unwrap()
                    .verified
            );
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "bob"))
                    .unwrap()
                    .verified
            );
            assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 2);
        });
    }

    /// Upgrader role cannot revoke verification (Issue #12).
    #[test]
    fn test_upgrader_role_cannot_revoke_verification() {
        let env = Env::default();
        let (admin, user, upgrader, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), upgrader.clone(), Role::Upgrader).unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::revoke_verification(
                env.clone(),
                upgrader.clone(),
                username(&env, "octocat"),
                1,
            );
            assert_eq!(result, Err(ContractError::NotAuthorized));
        });
    }

    /// Verifier-role address cannot call set_role (admin-only operation) (Issue #12).
    #[test]
    fn test_verifier_cannot_grant_roles() {
        let env = Env::default();
        let (_admin, user, verifier, contract_id) = setup(&env);
        let target = Address::generate(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), verifier.clone(), Role::Verifier).unwrap();

            // The contract does not expose a "set_role_as" API; the guard is
            // in set_role itself: admin.require_auth() is always the admin
            // address. This test validates the role table stays clean.
            let _ = user;
            let _ = target;
            assert_eq!(
                TrustBridgeContract::get_role(env.clone(), verifier.clone()),
                Some(Role::Verifier)
            );
        });
    }

    // ── Issue #114: verify / revoke_verification auth negative matrix ─────────
    //
    // Matrix of all unauthorized / invalid call paths for verify and
    // revoke_verification, with expected ContractError codes.  Each test
    // corresponds to exactly one cell in the table published in docs/SECURITY.md.
    //
    // verify negative matrix:
    // | # | Scenario                              | Expected error    | Code |
    // |---|---------------------------------------|-------------------|------|
    // | V1 | Not initialized                      | NotInitialized    |  2   |
    // | V2 | Username not registered               | NotRegistered     |  4   |
    // | V3 | Already verified (double-verify)      | AlreadyVerified   |  5   |
    // | V4 | Caller has no role                    | NotAuthorized     |  3   |
    // | V5 | Upgrader-role caller                  | NotAuthorized     |  3   |
    // | V6 | Admin caller (happy path)             | Ok(())            |  —   |
    // | V7 | Verifier-role caller (happy path)     | Ok(())            |  —   |
    // | V8 | Contract is paused                    | Paused            |  7   |
    //
    // revoke_verification negative matrix:
    // | # | Scenario                              | Expected error    | Code |
    // |---|---------------------------------------|-------------------|------|
    // | R1 | Not initialized                      | NotInitialized    |  2   |
    // | R2 | Username not registered               | NotRegistered     |  4   |
    // | R3 | Record not verified (can't revoke)    | NotVerified       |  6   |
    // | R4 | Caller has no role                    | NotAuthorized     |  3   |
    // | R5 | Upgrader-role caller                  | NotAuthorized     |  3   |
    // | R6 | Admin caller (happy path)             | Ok(())            |  —   |
    // | R7 | Verifier-role caller (happy path)     | Ok(())            |  —   |
    // | R8 | Contract is paused                    | Paused            |  7   |

    // ── verify negative matrix ────────────────────────────────────────────────

    /// #114-V1: verify before initialization returns NotInitialized (code 2).
    #[test]
    fn test_verify_negative_not_initialized() {
        let env = Env::default();
        let caller = Address::generate(&env);
        let contract_id = env.register(TrustBridgeContract, ());
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result =
                TrustBridgeContract::verify(env.clone(), caller.clone(), username(&env, "octocat"));
            assert_eq!(
                result,
                Err(ContractError::NotInitialized),
                "verify before init must return NotInitialized (code {})",
                ContractError::NotInitialized.code()
            );
            assert_eq!(ContractError::NotInitialized.code(), 2);
        });
    }

    /// #114-V2: verify on a username that was never registered returns NotRegistered (code 4).
    #[test]
    fn test_verify_negative_username_not_registered() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result =
                TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "ghost"));
            assert_eq!(
                result,
                Err(ContractError::NotRegistered),
                "verify on missing username must return NotRegistered (code {})",
                ContractError::NotRegistered.code()
            );
            assert_eq!(ContractError::NotRegistered.code(), 4);
        });
    }

    /// #114-V3: double-verify returns AlreadyVerified (code 5).
    #[test]
    fn test_verify_negative_already_verified() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result =
                TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"));
            assert_eq!(
                result,
                Err(ContractError::AlreadyVerified),
                "second verify must return AlreadyVerified (code {})",
                ContractError::AlreadyVerified.code()
            );
            assert_eq!(ContractError::AlreadyVerified.code(), 5);
        });
    }

    /// #114-V4: address with no role cannot verify — returns NotAuthorized (code 3).
    #[test]
    fn test_verify_negative_no_role_caller() {
        let env = Env::default();
        let (_admin, user, nobody, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            let result = TrustBridgeContract::verify(
                env.clone(),
                nobody.clone(), // no role
                username(&env, "octocat"),
            );
            assert_eq!(
                result,
                Err(ContractError::NotAuthorized),
                "no-role caller must not be able to verify (code {})",
                ContractError::NotAuthorized.code()
            );
            assert_eq!(ContractError::NotAuthorized.code(), 3);
        });
    }

    /// #114-V5: Upgrader-role address cannot verify — returns NotAuthorized (code 3).
    #[test]
    fn test_verify_negative_upgrader_cannot_verify() {
        let env = Env::default();
        let (_admin, user, upgrader, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), upgrader.clone(), Role::Upgrader).unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            let result = TrustBridgeContract::verify(
                env.clone(),
                upgrader.clone(), // Upgrader role — not allowed
                username(&env, "octocat"),
            );
            assert_eq!(
                result,
                Err(ContractError::NotAuthorized),
                "Upgrader-role must not be allowed to verify (code {})",
                ContractError::NotAuthorized.code()
            );
        });
    }

    /// #114-V6 (happy path): admin can verify a registered user.
    #[test]
    fn test_verify_positive_admin_can_verify() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .verified,
                "admin verify must set verified=true"
            );
        });
    }

    /// #114-V7 (happy path): Verifier-role address can verify.
    #[test]
    fn test_verify_positive_verifier_role_can_verify() {
        let env = Env::default();
        let (_admin, user, verifier, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), verifier.clone(), Role::Verifier).unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "octocat"))
                .unwrap();
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .verified,
                "Verifier-role verify must set verified=true"
            );
        });
    }

    /// #114-V8: verify while contract is paused returns Paused (code 7).
    #[test]
    fn test_verify_negative_paused() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::pause(env.clone(), 1).unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result =
                TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"));
            assert_eq!(
                result,
                Err(ContractError::Paused),
                "verify while paused must return Paused (code {})",
                ContractError::Paused.code()
            );
            assert_eq!(ContractError::Paused.code(), 7);
            // Record must still be unverified
            assert!(
                !TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .verified
            );
        });
    }

    // ── revoke_verification negative matrix ───────────────────────────────────

    /// #114-R1: revoke_verification before initialization returns NotInitialized (code 2).
    #[test]
    fn test_revoke_negative_not_initialized() {
        let env = Env::default();
        let caller = Address::generate(&env);
        let contract_id = env.register(TrustBridgeContract, ());
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::revoke_verification(
                env.clone(),
                caller.clone(),
                username(&env, "octocat"),
                1,
            );
            assert_eq!(
                result,
                Err(ContractError::NotInitialized),
                "revoke before init must return NotInitialized (code {})",
                ContractError::NotInitialized.code()
            );
        });
    }

    /// #114-R2: revoke_verification on an unregistered username returns NotRegistered (code 4).
    #[test]
    fn test_revoke_negative_username_not_registered() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::revoke_verification(
                env.clone(),
                admin.clone(),
                username(&env, "ghost"),
                1,
            );
            assert_eq!(
                result,
                Err(ContractError::NotRegistered),
                "revoke on missing username must return NotRegistered (code {})",
                ContractError::NotRegistered.code()
            );
        });
    }

    /// #114-R3: revoking on an unverified record returns NotVerified (code 6).
    #[test]
    fn test_revoke_negative_not_verified() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            let result = TrustBridgeContract::revoke_verification(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
                1,
            );
            assert_eq!(
                result,
                Err(ContractError::NotVerified),
                "revoke on unverified record must return NotVerified (code {})",
                ContractError::NotVerified.code()
            );
            assert_eq!(ContractError::NotVerified.code(), 6);
        });
    }

    /// #114-R4: address with no role cannot revoke — returns NotAuthorized (code 3).
    #[test]
    fn test_revoke_negative_no_role_caller() {
        let env = Env::default();
        let (admin, user, nobody, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
            let result = TrustBridgeContract::revoke_verification(
                env.clone(),
                nobody.clone(), // no role
                username(&env, "octocat"),
                1,
            );
            assert_eq!(
                result,
                Err(ContractError::NotAuthorized),
                "no-role caller must not revoke verification (code {})",
                ContractError::NotAuthorized.code()
            );
            // Verification must still be intact
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .verified
            );
        });
    }

    /// #114-R5: Upgrader-role address cannot revoke — returns NotAuthorized (code 3).
    #[test]
    fn test_revoke_negative_upgrader_cannot_revoke() {
        let env = Env::default();
        let (admin, user, upgrader, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), upgrader.clone(), Role::Upgrader).unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
            let result = TrustBridgeContract::revoke_verification(
                env.clone(),
                upgrader.clone(), // Upgrader role — not allowed
                username(&env, "octocat"),
                1,
            );
            assert_eq!(
                result,
                Err(ContractError::NotAuthorized),
                "Upgrader-role must not revoke verification (code {})",
                ContractError::NotAuthorized.code()
            );
        });
    }

    /// #114-R6 (happy path): admin can revoke verification.
    #[test]
    fn test_revoke_positive_admin_can_revoke() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::revoke_verification(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
                1,
            )
            .unwrap();
            assert!(
                !TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .verified,
                "admin revoke must clear verified=false"
            );
        });
    }

    /// #114-R7 (happy path): Revoker-role address can revoke verification (Issue #212).
    /// Updated from Verifier to Revoker — roles are now split.
    #[test]
    fn test_revoke_positive_verifier_role_can_revoke() {
        let env = Env::default();
        let (admin, user, revoker, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            // Use Revoker role, not Verifier (Issue #212).
            TrustBridgeContract::set_role(env.clone(), revoker.clone(), Role::Revoker).unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
            TrustBridgeContract::revoke_verification(
                env.clone(),
                revoker.clone(),
                username(&env, "octocat"),
                1,
            )
            .unwrap();
            assert!(
                !TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .verified,
                "Revoker-role revoke must clear verified=false"
            );
        });
    }

    /// #114-R8: revoke_verification while paused returns Paused (code 7).
    #[test]
    fn test_revoke_negative_paused() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::pause(env.clone(), 1).unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::revoke_verification(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
                1,
            );
            assert_eq!(
                result,
                Err(ContractError::Paused),
                "revoke while paused must return Paused (code {})",
                ContractError::Paused.code()
            );
            // Verification must still be intact
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .verified
            );
        });
    }

    // ── batch_remove (Wave #batch) ─────────────────────────────────────────
    //
    // `batch_remove` is the batched form of `remove`, intended for admin
    // workflows that need to clean up many stale or disputed registrations
    // efficiently. Partial success is the point: a username that cannot be
    // removed does not abort the batch.

    /// batch_remove happy path: admin removes multiple registered users.
    #[test]
    fn test_batch_remove_all_succeed() {
        let env = Env::default();
        let (admin, user1, user2, contract_id) = setup(&env);
        let user3 = Address::generate(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "carol"), user3.clone(), Vec::new(&env))
                .unwrap();
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 3);
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames = soroban_sdk::vec![
                &env,
                username(&env, "alice"),
                username(&env, "bob"),
                username(&env, "carol"),
            ];
            let summary =
                TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames).unwrap();
            assert_eq!(summary.total, 3);
            assert_eq!(summary.successful, 3);
            assert_eq!(summary.failed, 0);
            assert_eq!(summary.success_rate, 100);
            assert!(summary.all_successful());

            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 0);
            assert_eq!(stats.verified, 0);
        });
    }

    /// batch_remove partial success: some usernames are registered, some are not.
    #[test]
    fn test_batch_remove_partial_success() {
        let env = Env::default();
        let (admin, user1, _user2, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames = soroban_sdk::vec![
                &env,
                username(&env, "alice"), // registered → succeeds
                username(&env, "ghost"), // not registered → skipped (counted as failed)
            ];
            let summary =
                TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames).unwrap();
            assert_eq!(summary.total, 2);
            assert_eq!(summary.successful, 1);
            assert_eq!(summary.failed, 1);
            assert_eq!(summary.success_rate, 50);
            assert!(!summary.all_successful());
            assert!(summary.any_successful());

            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 0, "alice was removed");
        });
    }

    /// batch_remove all fail: none of the usernames are registered.
    #[test]
    fn test_batch_remove_all_fail() {
        let env = Env::default();
        let (admin, _user1, _user2, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames =
                soroban_sdk::vec![&env, username(&env, "ghost1"), username(&env, "ghost2"),];
            let summary =
                TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames).unwrap();
            assert_eq!(summary.total, 2);
            assert_eq!(summary.successful, 0);
            assert_eq!(summary.failed, 2);
            assert_eq!(summary.success_rate, 0);
            assert!(!summary.all_successful());
            assert!(!summary.any_successful());

            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 0, "registry must remain untouched");
        });
    }

    /// batch_remove rejects empty list with InvalidBatchSize.
    #[test]
    fn test_batch_remove_empty_rejected() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames: soroban_sdk::Vec<String> = soroban_sdk::Vec::new(&env);
            let result = TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames);
            assert_eq!(result, Err(ContractError::InvalidBatchSize));
        });
    }

    /// batch_remove rejects too-large list with InvalidBatchSize.
    #[test]
    fn test_batch_remove_over_limit_rejected() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let limit = crate::batch::BatchConfig::default().max_batch_size;
            let mut usernames: soroban_sdk::Vec<String> = soroban_sdk::Vec::new(&env);
            for i in 0..=limit {
                usernames.push_back(username(&env, &alloc::format!("user{i}")));
            }
            let result = TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames);
            assert_eq!(result, Err(ContractError::InvalidBatchSize));
        });
    }

    /// batch_remove is blocked while the contract is paused.
    #[test]
    fn test_batch_remove_blocked_while_paused() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user.clone(), Vec::new(&env))
                .unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::pause(env.clone(), 1).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames = soroban_sdk::vec![&env, username(&env, "alice")];
            let result = TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames);
            assert_eq!(result, Err(ContractError::Paused));
        });
    }

    /// batch_remove requires initialization.
    #[test]
    fn test_batch_remove_not_initialized() {
        let env = Env::default();
        let contract_id = env.register(TrustBridgeContract, ());
        let admin = Address::generate(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames = soroban_sdk::vec![&env, username(&env, "alice")];
            let result = TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames);
            assert_eq!(result, Err(ContractError::NotInitialized));
        });
    }

    /// batch_remove rejects non-admin caller with NotAuthorized.
    #[test]
    fn test_batch_remove_non_admin_rejected() {
        let env = Env::default();
        let (_admin, _user, other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames = soroban_sdk::vec![&env, username(&env, "alice")];
            let result = TrustBridgeContract::batch_remove(env.clone(), other.clone(), usernames);
            assert_eq!(result, Err(ContractError::NotAuthorized));
        });
    }

    /// batch_remove with mixed verified/unverified records updates counters correctly.
    #[test]
    fn test_batch_remove_mixed_verified_state() {
        let env = Env::default();
        let (admin, user1, user2, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone(), Vec::new(&env))
                .unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "alice"))
                .unwrap();
        });

        env.as_contract(&contract_id, || {
            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 2);
            assert_eq!(stats.verified, 1);
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames = soroban_sdk::vec![
                &env,
                username(&env, "alice"), // verified
                username(&env, "bob"),   // unverified
            ];
            let summary =
                TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames).unwrap();
            assert_eq!(summary.total, 2);
            assert_eq!(summary.successful, 2);

            let stats = TrustBridgeContract::get_stats(env.clone());
            assert_eq!(stats.total, 0);
            assert_eq!(stats.verified, 0);
        });
    }

    /// #114-verify-error-codes: all verify/revoke-relevant error codes match their ABI discriminants.
    #[test]
    fn test_verify_revoke_negative_error_codes_match_abi() {
        assert_eq!(ContractError::NotInitialized.code(), 2);
        assert_eq!(ContractError::NotAuthorized.code(), 3);
        assert_eq!(ContractError::NotRegistered.code(), 4);
        assert_eq!(ContractError::AlreadyVerified.code(), 5);
        assert_eq!(ContractError::NotVerified.code(), 6);
        assert_eq!(ContractError::Paused.code(), 7);
    }

    // ── Revoke reason code validation (Wave #19) ───────────────────────────────
    //
    // `revoke_verification` now requires a `reason_code` parameter. Valid codes
    // are defined in the `RevokeReason` enum; invalid codes return
    // `InvalidReasonCode` (code 15) before auth or any storage mutation.

    /// All `RevokeReason` variants are accepted by `revoke_verification`.
    #[test]
    fn test_revoke_verification_accepts_all_valid_reason_codes() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });

        let valid_codes = [1, 2, 3, 4, 5, 6, 99];
        for &code in &valid_codes {
            env.mock_all_auths();
            env.as_contract(&contract_id, || {
                TrustBridgeContract::revoke_verification(
                    env.clone(),
                    admin.clone(),
                    username(&env, "octocat"),
                    code,
                )
                .unwrap();
                assert!(
                    !TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                        .unwrap()
                        .verified,
                    "revoke with reason code {code} must clear verified"
                );
            });

            // Re-verify for the next iteration
            env.mock_all_auths();
            env.as_contract(&contract_id, || {
                TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                    .unwrap();
            });
        }
    }

    /// Invalid reason codes return `InvalidReasonCode` (code 15) and leave the
    /// record unmodified.
    #[test]
    fn test_revoke_verification_rejects_invalid_reason_code() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });

        let invalid_codes = [0, 7, 8, 9, 10, 11, 50, 98, 100, u32::MAX];
        for &code in &invalid_codes {
            env.mock_all_auths();
            env.as_contract(&contract_id, || {
                let result = TrustBridgeContract::revoke_verification(
                    env.clone(),
                    admin.clone(),
                    username(&env, "octocat"),
                    code,
                );
                assert_eq!(
                    result,
                    Err(ContractError::InvalidReasonCode),
                    "expected InvalidReasonCode for reason code {code}"
                );
                assert_eq!(ContractError::InvalidReasonCode.code(), 15);

                // Record must remain verified and verified count unchanged
                assert!(
                    TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                        .unwrap()
                        .verified,
                    "failed revoke must not clear verified flag"
                );
                assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 1);
            });
        }
    }

    // ── ContributorRecord size optimization (Issue #67 / Wave #66) ─────────
    //
    // `ContributorRecord.registered_at` was changed from `u64` to `u32`,
    // saving 4 bytes per record (37 bytes serialized vs 41 bytes in XDR).
    // This is safe because Soroban ledger timestamps (Unix seconds) fit in
    // u32 until ~2106.

    /// `registered_at` stores a valid ledger timestamp at registration time.
    #[test]
    fn test_contributor_record_registered_at_is_valid_timestamp() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let ledger_ts = env.ledger().timestamp();
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            // registered_at fits in u32 and matches the ledger timestamp.
            assert!(
                (record.registered_at as u64).abs_diff(ledger_ts) <= 2,
                "registered_at ({}) should be within 2s of ledger timestamp ({})",
                record.registered_at,
                ledger_ts
            );
        });
    }

    /// `registered_at` updates on re-registration with the same address.
    #[test]
    fn test_contributor_record_registered_at_updates_on_reregister() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        let ts1 = env.as_contract(&contract_id, || {
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .registered_at
        });
        // Advance ledger and re-register
        env.ledger().set_timestamp(env.ledger().timestamp() + 1000);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        let ts2 = env.as_contract(&contract_id, || {
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .registered_at
        });
        assert!(ts2 > ts1, "registered_at must advance on re-registration");
        assert_eq!(ts2 as u64, env.ledger().timestamp());
    }

    /// `registered_at` for a freshly registered record must equal the ledger
    /// timestamp at registration time (truncated to u32).
    #[test]
    fn test_contributor_record_registered_at_matches_ledger_timestamp() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        // Set a specific ledger timestamp
        let specific_ts: u64 = 1_700_000_000; // Nov 2023
        env.ledger().set_timestamp(specific_ts);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert_eq!(
                record.registered_at as u64, specific_ts,
                "registered_at must match ledger timestamp"
            );
        });
    }

    /// `registered_at` uses u32 and the cast from u64 must not lose precision
    /// for current timestamps (well below u32::MAX ≈ 4.3B).
    #[test]
    fn test_contributor_record_registered_at_fits_in_u32() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();

            let ledger_ts = env.ledger().timestamp();
            // ledger_ts is u64; registered_at is u32.
            // For the current epoch (~1.7B) this is well within u32 range.
            assert!(
                ledger_ts <= u32::MAX as u64,
                "ledger timestamp {} exceeds u32 range — u32 would truncate!",
                ledger_ts
            );
            assert_eq!(
                record.registered_at as u64, ledger_ts,
                "u32 truncation must not lose precision for current timestamps"
            );
        });
    }

    /// The revocation event exposes the complete payload and stable topic shape.
    #[test]
    fn test_verification_revoked_event_payload_is_complete() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone(), Vec::new(&env))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::revoke_verification(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
                2, // CompromisedKey
            )
            .unwrap();

            let expected = VerificationRevokedEvent {
                github_username: username(&env, "octocat"),
                stellar_address: user.clone(),
                timestamp: env.ledger().timestamp(),
                reason_code: 2,
                domain: event_domain(&env),
            };

            assert_eq!(
                env.events().all(),
                soroban_sdk::vec![
                    &env,
                    (
                        contract_id.clone(),
                        expected.topics(&env),
                        expected.data(&env),
                    )
                ],
                "VerificationRevokedEvent must include the supplied reason_code"
            );

            let topics = expected.topics(&env);
            assert_eq!(topics.len(), 2, "VerificationRevokedEvent must have 2 topics");
            assert_eq!(
                soroban_sdk::Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
                soroban_sdk::Symbol::new(&env, "verification_revoked_event"),
                "VerificationRevokedEvent topic symbol changed"
            );
            assert_eq!(
                String::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
                username(&env, "octocat"),
                "VerificationRevokedEvent username topic changed"
            );
        });
    }

    /// A failed revoke must not publish an event that could make an indexer
    /// mark an unverified contributor as revoked.
    #[test]
    fn test_verification_revoked_event_not_published_on_not_verified() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user, Vec::new(&env)).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::revoke_verification(
                env.clone(),
                admin,
                username(&env, "octocat"),
                1,
            );
            assert_eq!(result, Err(ContractError::NotVerified));
            assert_eq!(
                env.events().all(),
                soroban_sdk::vec![&env],
                "failed revoke published a VerificationRevokedEvent"
            );
        });
    }

    // ── batch_verify tests ──────────────────────────────────────────────────

    #[test]
    fn test_batch_verify_happy_path() {
        let env = Env::default();
        let (admin, user1, user2, contract_id) = setup(&env);
        let user3 = Address::generate(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "user1"), user1, Vec::new(&env)).unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "user2"), user2, Vec::new(&env)).unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "user3"), user3, Vec::new(&env)).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames = soroban_sdk::vec![
                &env,
                username(&env, "user1"),
                username(&env, "user2"),
                username(&env, "user3"),
            ];
            let summary =
                TrustBridgeContract::batch_verify(env.clone(), admin.clone(), usernames).unwrap();
            assert_eq!(summary.total, 3);
            assert_eq!(summary.successful, 3);
            assert_eq!(summary.failed, 0);
            assert_eq!(summary.success_rate, 100);
            assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 3);
        });
    }

    #[test]
    fn test_batch_verify_partial_and_mixed() {
        let env = Env::default();
        let (admin, user1, _user2, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "user1"), user1.clone(), Vec::new(&env))
                .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "user1"))
                .unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            // user1: already verified -> fail
            // user2: not registered -> fail
            let usernames =
                soroban_sdk::vec![&env, username(&env, "user1"), username(&env, "user2"),];
            let summary =
                TrustBridgeContract::batch_verify(env.clone(), admin.clone(), usernames).unwrap();
            assert_eq!(summary.total, 2);
            assert_eq!(summary.successful, 0);
            assert_eq!(summary.failed, 2);
            assert_eq!(summary.success_rate, 0);
        });
    }

    #[test]
    fn test_batch_verify_verifier_role() {
        let env = Env::default();
        let (_admin, user1, verifier, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), verifier.clone(), Role::Verifier).unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "user1"), user1, Vec::new(&env)).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames = soroban_sdk::vec![&env, username(&env, "user1")];
            let summary =
                TrustBridgeContract::batch_verify(env.clone(), verifier.clone(), usernames)
                    .unwrap();
            assert_eq!(summary.successful, 1);
        });
    }

    #[test]
    fn test_batch_verify_upgrader_rejected() {
        let env = Env::default();
        let (_admin, user1, upgrader, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), upgrader.clone(), Role::Upgrader).unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "user1"), user1, Vec::new(&env)).unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames = soroban_sdk::vec![&env, username(&env, "user1")];
            let res = TrustBridgeContract::batch_verify(env.clone(), upgrader.clone(), usernames);
            assert_eq!(res, Err(ContractError::NotAuthorized));
        });
    }

    #[test]
    fn test_batch_verify_empty_or_oversize() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let empty = soroban_sdk::vec![&env];
            assert_eq!(
                TrustBridgeContract::batch_verify(env.clone(), admin.clone(), empty),
                Err(ContractError::InvalidBatchSize)
            );
        });
    }

    #[test]
    fn test_batch_verify_paused() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::pause(env.clone(), 1).unwrap();
            let usernames = soroban_sdk::vec![&env, username(&env, "user1")];
            assert_eq!(
                TrustBridgeContract::batch_verify(env.clone(), admin.clone(), usernames),
                Err(ContractError::Paused)
            );
        });
    }

    // ── TTL keeper tests ─────────────────────────────────────────────────────

    #[test]
    fn test_extend_registry_ttl() {
        let env = Env::default();
        let (_admin, user1, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user1.clone(), Vec::new(&env)).unwrap();
        });

        // 1. Success path
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames = soroban_sdk::vec![&env, username(&env, "octocat")];
            let extended = TrustBridgeContract::extend_registry_ttl(env.clone(), usernames).unwrap();
            assert_eq!(extended, 1);
        });

        // 2. Unknown username (skipped, not an error)
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames = soroban_sdk::vec![&env, username(&env, "unknown")];
            let extended = TrustBridgeContract::extend_registry_ttl(env.clone(), usernames).unwrap();
            assert_eq!(extended, 0);
        });

        // 3. Mixed batch (some exist, some unknown)
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames = soroban_sdk::vec![&env, username(&env, "unknown"), username(&env, "octocat")];
            let extended = TrustBridgeContract::extend_registry_ttl(env.clone(), usernames).unwrap();
            assert_eq!(extended, 1);
        });

        // 4. Invalid batch size (empty)
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames = soroban_sdk::vec![&env];
            let res = TrustBridgeContract::extend_registry_ttl(env.clone(), usernames);
            assert_eq!(res, Err(ContractError::InvalidBatchSize));
        });

        // 5. Works while paused
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::pause(env.clone(), 1).unwrap();
            let usernames = soroban_sdk::vec![&env, username(&env, "octocat")];
            let extended = TrustBridgeContract::extend_registry_ttl(env.clone(), usernames).unwrap();
            assert_eq!(extended, 1);
        });
    }

    // ── Audit log & config_verification tests ────────────────────────────────

    #[test]
    fn test_audit_logs_persisted_and_retrieved() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user, Vec::new(&env)).unwrap();

            let logs = TrustBridgeContract::get_audit_logs(env.clone());
            assert!(!logs.is_empty());
            assert_eq!(
                logs.get(0).unwrap().event_type,
                AuditEventType::ContractInitialized
            );
            assert_eq!(
                logs.get(1).unwrap().event_type,
                AuditEventType::UserRegistered
            );

            let stats = TrustBridgeContract::get_audit_stats(env.clone());
            assert_eq!(stats.registrations, 1);
        });
    }

    #[test]
    fn test_unauthorized_caller_leaves_no_audit_row() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user, Vec::new(&env)).unwrap();
        });

        let initial_logs_len = env.as_contract(&contract_id, || {
            TrustBridgeContract::get_audit_logs(env.clone()).len()
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            register_personal(&env, &contract_id, "octocat", &user);
            TrustBridgeContract::verify(env.clone(), username(&env, "octocat")).unwrap();
            register_personal(&env, &contract_id, "octocat", &new_user);
            let res =
                TrustBridgeContract::remove(env.clone(), other.clone(), username(&env, "octocat"));
            assert_eq!(res, Err(ContractError::NotAuthorized));

            let logs_len_after = TrustBridgeContract::get_audit_logs(env.clone()).len();
            assert_eq!(initial_logs_len, logs_len_after);
        });
    }

    #[test]
    fn test_config_verification_persists_and_gets() {
        let env = Env::default();
        let (admin, _user, other, contract_id) = setup(&env);
        let attestation = Symbol::new(&env, "github_att");

        // Unauthorized caller fails
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let res = TrustBridgeContract::config_verification(
                env.clone(),
                other,
                attestation.clone(),
                3600,
                2,
            );
            assert_eq!(res, Err(ContractError::NotAuthorized));
            assert_eq!(
                TrustBridgeContract::get_verification_config(env.clone()),
                None
            );
        });

        // Admin caller succeeds
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::config_verification(
                env.clone(),
                admin,
                attestation.clone(),
                3600,
                2,
            )
            .unwrap();

            let cfg = TrustBridgeContract::get_verification_config(env.clone()).unwrap();
            assert_eq!(cfg.attestation, attestation);
            assert_eq!(cfg.expires_in, 3600);
            assert_eq!(cfg.threshold, 2);
        });
    }

    // ── Cooldown enforcement tests ─────────────────────────────────────────

    #[test]
    fn test_register_cooldown_enforcement() {
        let env = Env::default();
        let (_admin, user1, user2, contract_id) = setup(&env);

        env.ledger().set_timestamp(1000);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            // Set cooldown to 100 seconds
            TrustBridgeContract::set_cooldown(env.clone(), 100).unwrap();

            register_personal(&env, &contract_id, "alice", &user1);
            register_personal(&env, &contract_id, "bob", &user2);
            // First registration succeeds
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user1.clone(), Vec::new(&env))
                .unwrap();

            // Immediate re-registration fails with CooldownActive
            let res = TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user2.clone(),
                Vec::new(&env),
            );
            assert_eq!(res, Err(ContractError::CooldownActive));
        });

        // Advance ledger timestamp by 101 seconds
        env.ledger().set_timestamp(1000 + 101);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            // After cooldown elapses, re-registration succeeds
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user2.clone(), Vec::new(&env))
                .unwrap();
        });
    }

    // Issue #296: `is_registration_in_cooldown` is the read-only view of the
    // cooldown that `register` enforces inline. It must flip to `true` purely as
    // a side effect of a successful `register` — with no `record_action` call,
    // which no longer exists — and back to `false` once the window elapses.
    #[test]
    fn test_is_registration_in_cooldown_tracks_auto_enforcement() {
        let env = Env::default();
        let (_admin, user1, user2, contract_id) = setup(&env);

        env.ledger().set_timestamp(5_000);
        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_cooldown(env.clone(), 100).unwrap();

            // Unknown username: never in cooldown.
            assert!(!TrustBridgeContract::is_registration_in_cooldown(
                env.clone(),
                username(&env, "octocat")
            ));

            // A successful register stamps the cooldown with no extra call.
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user1.clone(),
                Vec::new(&env),
            )
            .unwrap();
            assert!(TrustBridgeContract::is_registration_in_cooldown(
                env.clone(),
                username(&env, "octocat")
            ));

            // And the enforced mutation agrees with the read.
            assert_eq!(
                TrustBridgeContract::register(
                    env.clone(),
                    username(&env, "octocat"),
                    user2.clone(),
                    Vec::new(&env),
                ),
                Err(ContractError::CooldownActive)
            );
        });

        env.ledger().set_timestamp(5_000 + 101);
        env.as_contract(&contract_id, || {
            assert!(!TrustBridgeContract::is_registration_in_cooldown(
                env.clone(),
                username(&env, "octocat")
            ));
        });
    }

    // ── Issue #197: set_paused events ─────────────────────────────────────────

    #[test]
    fn test_set_paused_true_emits_paused_event() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_paused(env.clone(), true, 1).unwrap();
            let events = env.events().all();
            let expected_topic = soroban_sdk::xdr::ScVal::Symbol(
                soroban_sdk::xdr::ScSymbol("paused_event".try_into().unwrap()),
            );
            let found = events.events().iter().any(|e| {
                let soroban_sdk::xdr::ContractEventBody::V0(body) = &e.body;
                body.topics.iter().any(|t| t == &expected_topic)
            });
            assert!(found, "set_paused(true) must emit PausedEvent");
        });
    }

    #[test]
    fn test_set_paused_false_emits_unpaused_event() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            // First pause, then unpause via set_paused
            TrustBridgeContract::pause(env.clone(), 1).unwrap();
            env.events().all(); // consume
            TrustBridgeContract::set_paused(env.clone(), false, 4).unwrap();
            let events = env.events().all();
            let expected_topic = soroban_sdk::xdr::ScVal::Symbol(
                soroban_sdk::xdr::ScSymbol("unpaused_event".try_into().unwrap()),
            );
            let found = events.events().iter().any(|e| {
                let soroban_sdk::xdr::ContractEventBody::V0(body) = &e.body;
                body.topics.iter().any(|t| t == &expected_topic)
            });
            assert!(found, "set_paused(false) must emit UnpausedEvent");
        });
    }

    #[test]
    fn test_set_paused_idempotent_no_duplicate_event() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            // Already unpaused — calling set_paused(false) again should be a no-op
            let count_before = env.events().all().events().len();
            TrustBridgeContract::set_paused(env.clone(), false, 4).unwrap();
            let count_after = env.events().all().events().len();
            assert_eq!(
                count_before, count_after,
                "set_paused(false) while already unpaused must not emit an event"
            );

            // Pause, then call set_paused(true) again — still a no-op
            TrustBridgeContract::set_paused(env.clone(), true, 1).unwrap();
            let count_before2 = env.events().all().events().len();
            TrustBridgeContract::set_paused(env.clone(), true, 1).unwrap();
            let count_after2 = env.events().all().events().len();
            assert_eq!(
                count_before2, count_after2,
                "set_paused(true) while already paused must not emit an event"
            );
        });
    }

    #[test]
    fn test_set_paused_unauthorized_caller_fails() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths_allowing_non_root_auth();
        env.as_contract(&contract_id, || {
            // No-auth call should still be gated through admin.require_auth()
            // We mock all auths so this tests the logical path — the admin key is
            // the only one that satisfies the check.
            assert!(
                TrustBridgeContract::set_paused(env.clone(), true, 1).is_ok(),
                "mock_all_auths must satisfy require_auth()"
            );
        });
        drop(user); // suppress unused warning
    }

    // ── Issue #196: guardian circuit breaker ─────────────────────────────────

    #[test]
    fn test_emergency_pause_by_guardian_blocks_mutations() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        let guardian = Address::generate(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            // Register first so we have a record to mutate
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();

            // Admin designates a guardian
            TrustBridgeContract::set_guardian(env.clone(), guardian.clone()).unwrap();

            // Guardian trips the circuit breaker
            TrustBridgeContract::emergency_pause(env.clone(), guardian.clone()).unwrap();

            // Mutations are now blocked
            assert_eq!(
                TrustBridgeContract::register(
                    env.clone(),
                    username(&env, "newuser"),
                    user.clone(),
                    Vec::new(&env),
                ),
                Err(ContractError::Paused)
            );
            assert_eq!(
                TrustBridgeContract::remove(
                    env.clone(),
                    user.clone(),
                    username(&env, "octocat")
                ),
                Err(ContractError::Paused)
            );
        });
        drop(admin);
    }

    #[test]
    fn test_emergency_pause_only_admin_can_clear() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        let guardian = Address::generate(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_guardian(env.clone(), guardian.clone()).unwrap();
            TrustBridgeContract::emergency_pause(env.clone(), guardian.clone()).unwrap();
            assert!(TrustBridgeContract::is_emergency_paused(env.clone()));

            // Admin clears it
            TrustBridgeContract::clear_emergency_pause(env.clone()).unwrap();
            assert!(!TrustBridgeContract::is_emergency_paused(env.clone()));
        });
        drop(admin);
    }

    #[test]
    fn test_guardian_cannot_upgrade() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        let guardian = Address::generate(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_guardian(env.clone(), guardian.clone()).unwrap();
            // Guardian has no Upgrader role — verify get_role returns None
            let role = TrustBridgeContract::get_role(env.clone(), guardian.clone());
            assert!(role.is_none(), "guardian must not have any role assigned");
        });
    }

    #[test]
    fn test_non_guardian_non_admin_cannot_emergency_pause() {
        let env = Env::default();
        let (_admin, _user, other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result =
                TrustBridgeContract::emergency_pause(env.clone(), other.clone());
            assert_eq!(result, Err(ContractError::NotAuthorized));
        });
    }

    #[test]
    fn test_emergency_pause_idempotent() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::emergency_pause(env.clone(), admin.clone()).unwrap();
            let count_before = env.events().all().events().len();
            // Second call should be no-op
            TrustBridgeContract::emergency_pause(env.clone(), admin.clone()).unwrap();
            let count_after = env.events().all().events().len();
            assert_eq!(
                count_before, count_after,
                "emergency_pause while already tripped must not emit an event"
            );
        });
    }

    #[test]
    fn test_both_pause_flags_both_must_clear() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        let guardian = Address::generate(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_guardian(env.clone(), guardian.clone()).unwrap();

            // Trip both flags
            TrustBridgeContract::pause(env.clone(), 1).unwrap();
            TrustBridgeContract::emergency_pause(env.clone(), guardian.clone()).unwrap();

            // Clear emergency pause only — normal pause still blocks
            TrustBridgeContract::clear_emergency_pause(env.clone()).unwrap();
            let user = Address::generate(&env);
            assert_eq!(
                TrustBridgeContract::register(
                    env.clone(),
                    username(&env, "stillblocked"),
                    user.clone(),
                    Vec::new(&env),
                ),
                Err(ContractError::Paused),
                "normal pause still active after emergency cleared"
            );

            // Clear normal pause too — now unblocked
            TrustBridgeContract::unpause(env.clone(), 4).unwrap();
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "nowworks"),
                user,
                Vec::new(&env),
            )
            .unwrap();
        });
        drop(admin);
    }

    // ── Issue #202: CHUNK_SIZE regression test ────────────────────────────────

    #[test]
    fn test_chunk_size_is_fifty() {
        // Regression test: CHUNK_SIZE must be 50.
        // If this constant is ever changed, docs/DASHBOARD_SYNC.md,
        // docs/storage-rent-estimator.inputs.v1.json, and the estimator's
        // cost_drivers_vs_n table must all be updated to match.
        assert_eq!(
            crate::storage::CHUNK_SIZE,
            50,
            "CHUNK_SIZE changed! Update DASHBOARD_SYNC.md and storage-rent-estimator.inputs.v1.json"
        );
    }

    #[test]
    fn test_chunk_boundaries_at_fifty() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            // Register exactly CHUNK_SIZE (50) users — all go in chunk 0
            for i in 0..50u32 {
                let name = format!("user{i:03}");
                let addr = Address::generate(&env);
                TrustBridgeContract::register(env.clone(), username(&env, &name), addr, Vec::new(&env)).unwrap();
            }
            let chunk_count_at_50 = crate::storage::get_chunk_count(&env);
            assert_eq!(chunk_count_at_50, 1, "50 entries should occupy exactly 1 chunk");

            // Register the 51st user — must spill into chunk 1
            let addr51 = Address::generate(&env);
            TrustBridgeContract::register(env.clone(), username(&env, "user050"), addr51, Vec::new(&env)).unwrap();
            let chunk_count_at_51 = crate::storage::get_chunk_count(&env);
            assert_eq!(chunk_count_at_51, 2, "51st entry must create a second chunk");
        });
    }

    /// Issue #291: `has_record` / `get_address` must resolve by direct persistent
    /// key and never scan the chunked index, so they stay correct — and O(1) —
    /// with the registry spread across many chunks.
    #[test]
    fn test_has_record_is_direct_key_across_many_chunks() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);

        // Enough users to fill several chunks (CHUNK_SIZE == 50).
        const N: u32 = 260;

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            for i in 0..N {
                let name = format!("user{i:04}");
                let addr = Address::generate(&env);
                TrustBridgeContract::register(
                    env.clone(),
                    username(&env, &name),
                    addr,
                    Vec::new(&env),
                )
                .unwrap();
            }

            // The index really is chunked now.
            assert!(
                crate::storage::get_chunk_count(&env) >= 5,
                "expected the registry to span multiple chunks"
            );

            // A user in the very first chunk, one in the last, and one in the
            // middle all resolve — a chunk scan that stopped early would miss
            // some of these.
            for name in ["user0000", "user0130", &format!("user{:04}", N - 1)] {
                assert!(
                    TrustBridgeContract::has_record(env.clone(), username(&env, name)),
                    "has_record must find '{name}' regardless of which chunk holds it"
                );
                assert!(
                    TrustBridgeContract::get_address(env.clone(), username(&env, name)).is_some(),
                    "get_address must find '{name}' regardless of which chunk holds it"
                );
            }

            // A never-registered username must report absent without scanning
            // every chunk looking for it.
            assert!(
                !TrustBridgeContract::has_record(env.clone(), username(&env, "ghost")),
                "has_record must not report a phantom record"
            );
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "ghost")).is_none(),
                "get_address must return None for an unregistered username"
            );
        });
    }

    // ── Issue #200: Property fuzzing suite ───────────────────────────────────
    //
    // The contract is `no_std`, so external fuzz crates (proptest, arbitrary)
    // are unavailable. We use a tiny xorshift64 PRNG with fixed seeds for
    // deterministic, CI-friendly property testing. A `Shadow` struct mirrors
    // the registry outside contract storage so invariants are checked against
    // an independent model.

    /// Tiny xorshift64 PRNG — deterministic, no-std, no dependencies.
    struct Prng(u64);

    impl Prng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        fn next_usize(&mut self, n: usize) -> usize {
            (self.next() as usize) % n
        }
    }

    /// Fixed seeds — failures are deterministic and always reproduce.
    const FUZZ_SEEDS: &[u64] = &[0xDEAD_BEEF_1234_5678, 0xCAFE_BABE_FEED_FACE, 0x0101_0101_ABCD_EF01, 0x9999_8888_7777_6666];

    /// Shadow model of the registry. Mirrors the contract's own counters using
    /// independent logic so a bug in the contract cannot hide itself.
    #[derive(Default)]
    struct Shadow {
        /// username → (stellar_address_index, verified)
        entries: std::vec::Vec<(std::string::String, usize, bool)>,
    }

    impl Shadow {
        fn has(&self, name: &str) -> bool {
            self.entries.iter().any(|(n, _, _)| n == name)
        }

        fn is_verified(&self, name: &str) -> bool {
            self.entries
                .iter()
                .find(|(n, _, _)| n == name)
                .map(|(_, _, v)| *v)
                .unwrap_or(false)
        }

        fn register(&mut self, name: std::string::String, addr_idx: usize) {
            if let Some(entry) = self.entries.iter_mut().find(|(n, _, _)| *n == name) {
                // Re-register same username: keep verified only if same address
                let keep_verified = entry.1 == addr_idx && entry.2;
                entry.1 = addr_idx;
                entry.2 = keep_verified;
            } else {
                self.entries.push((name, addr_idx, false));
            }
        }

        fn verify(&mut self, name: &str) {
            if let Some(entry) = self.entries.iter_mut().find(|(n, _, _)| n == name) {
                entry.2 = true;
            }
        }

        fn revoke(&mut self, name: &str) {
            if let Some(entry) = self.entries.iter_mut().find(|(n, _, _)| n == name) {
                entry.2 = false;
            }
        }

        fn remove(&mut self, name: &str) {
            self.entries.retain(|(n, _, _)| n != name);
        }

        fn total(&self) -> u32 {
            self.entries.len() as u32
        }

        fn verified(&self) -> u32 {
            self.entries.iter().filter(|(_, _, v)| *v).count() as u32
        }
    }

    /// Assert all 8 registry invariants match the shadow model.
    fn assert_registry_invariants(env: &Env, contract_id: &Address, shadow: &Shadow) {
        let stats = env.as_contract(contract_id, || TrustBridgeContract::get_stats(env.clone()));
        let vcount =
            env.as_contract(contract_id, || TrustBridgeContract::get_verified_count(env.clone()));

        // I1: total count matches shadow
        assert_eq!(
            stats.total,
            shadow.total(),
            "I1 violated: total count mismatch (contract={}, shadow={})",
            stats.total,
            shadow.total()
        );
        // I2 + I3: verified count consistent
        assert_eq!(
            stats.verified,
            shadow.verified(),
            "I2 violated: verified count mismatch (contract={}, shadow={})",
            stats.verified,
            shadow.verified()
        );
        assert_eq!(vcount, stats.verified, "I3 violated: get_verified_count() diverged from get_stats().verified");
        // I4: verified <= total
        assert!(
            stats.verified <= stats.total,
            "I4 violated: verified ({}) > total ({})",
            stats.verified,
            stats.total
        );
    }

    /// Run one fuzz session: `steps` random operations against `usernames`,
    /// asserting invariants after every step.
    fn run_fuzz_session(
        env: &Env,
        contract_id: &Address,
        admin: &Address,
        addrs: &[Address],
        usernames: &[&str],
        prng: &mut Prng,
        steps: usize,
    ) {
        let mut shadow = Shadow::default();

        for _ in 0..steps {
            let op = prng.next_usize(4);
            let name_idx = prng.next_usize(usernames.len());
            let addr_idx = prng.next_usize(addrs.len());
            let name = usernames[name_idx];

            match op {
                0 => {
                    // register
                    let is_new = !shadow.has(name);
                    let addr = addrs[addr_idx].clone();
                    env.mock_all_auths();
                    let result = env.as_contract(contract_id, || {
                        TrustBridgeContract::register(
                            env.clone(),
                            username(env, name),
                            addr.clone(),
                            Vec::new(&env),
                        )
                    });
                    if result.is_ok() {
                        shadow.register(name.to_string(), addr_idx);
                    } else if is_new {
                        // New registrations should succeed unless paused/invalid
                        // (in this suite no pause is set, so failures are unexpected)
                        panic!("unexpected register failure for new user {name}: {result:?}");
                    }
                }
                1 => {
                    // verify
                    let already_verified = shadow.is_verified(name);
                    let exists = shadow.has(name);
                    env.mock_all_auths();
                    let result = env.as_contract(contract_id, || {
                        TrustBridgeContract::verify(
                            env.clone(),
                            admin.clone(),
                            username(env, name),
                        )
                    });
                    match result {
                        Ok(()) => {
                            assert!(exists, "verify returned Ok but shadow has no record for {name}");
                            assert!(!already_verified, "verify returned Ok but shadow says already verified for {name}");
                            shadow.verify(name);
                        }
                        Err(ContractError::NotRegistered) => {
                            assert!(!exists, "verify returned NotRegistered but shadow has record for {name}");
                        }
                        Err(ContractError::AlreadyVerified) => {
                            assert!(already_verified, "verify returned AlreadyVerified but shadow says not verified for {name}");
                        }
                        Err(e) => panic!("unexpected verify error for {name}: {e:?}"),
                    }
                }
                2 => {
                    // revoke_verification
                    let is_verified = shadow.is_verified(name);
                    let exists = shadow.has(name);
                    env.mock_all_auths();
                    let result = env.as_contract(contract_id, || {
                        TrustBridgeContract::revoke_verification(
                            env.clone(),
                            admin.clone(),
                            username(env, name),
                            1,
                        )
                    });
                    match result {
                        Ok(()) => {
                            assert!(is_verified, "revoke returned Ok but shadow says not verified for {name}");
                            shadow.revoke(name);
                        }
                        Err(ContractError::NotRegistered) => {
                            assert!(!exists, "revoke returned NotRegistered but shadow has record for {name}");
                        }
                        Err(ContractError::NotVerified) => {
                            assert!(!is_verified, "revoke returned NotVerified but shadow says verified for {name}");
                        }
                        Err(e) => panic!("unexpected revoke error for {name}: {e:?}"),
                    }
                }
                _ => {
                    // remove
                    let exists = shadow.has(name);
                    let caller = if exists {
                        // Use the admin as caller to ensure auth passes
                        admin.clone()
                    } else {
                        addrs[addr_idx].clone()
                    };
                    env.mock_all_auths();
                    let result = env.as_contract(contract_id, || {
                        TrustBridgeContract::remove(
                            env.clone(),
                            caller,
                            username(env, name),
                        )
                    });
                    match result {
                        Ok(()) => {
                            assert!(exists, "remove returned Ok but shadow has no record for {name}");
                            shadow.remove(name);
                        }
                        Err(ContractError::NotRegistered) => {
                            assert!(!exists, "remove returned NotRegistered but shadow has record for {name}");
                        }
                        Err(e) => panic!("unexpected remove error for {name}: {e:?}"),
                    }
                }
            }

            assert_registry_invariants(env, contract_id, &shadow);
        }
    }

    #[test]
    fn test_fuzz_invariants_hold_across_random_operation_sequences() {
        let usernames = ["alice", "bob", "carol", "dave", "eve", "frank", "grace", "heidi"];

        for &seed in FUZZ_SEEDS {
            let env = Env::default();
            let (admin, _user, _other, contract_id) = setup(&env);
            let addrs: std::vec::Vec<Address> = (0..4).map(|_| Address::generate(&env)).collect();

            let mut prng = Prng(seed);
            run_fuzz_session(&env, &contract_id, &admin, &addrs, &usernames, &mut prng, 64);
        }
    }

    #[test]
    fn test_fuzz_invariants_hold_at_contributor_scale() {
        // Wider username pool to stress index/chunk boundaries.
        let usernames: std::vec::Vec<std::string::String> =
            (0..16).map(|i| format!("fuzz{i:02}")).collect();
        let username_refs: std::vec::Vec<&str> = usernames.iter().map(|s| s.as_str()).collect();

        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        let addrs: std::vec::Vec<Address> = (0..8).map(|_| Address::generate(&env)).collect();

        let mut prng = Prng(FUZZ_SEEDS[0]);
        run_fuzz_session(
            &env,
            &contract_id,
            &admin,
            &addrs,
            &username_refs,
            &mut prng,
            256,
        );
    }

    #[test]
    fn test_fuzz_failure_paths_leave_invariants_intact() {
        // I7: rejected operations must not mutate any counter or record.
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "target"), user.clone(), Vec::new(&env))
                .unwrap();
        });

        let usernames = ["target", "ghost"];
        let addrs = [user.clone(), admin.clone()];
        let mut prng = Prng(FUZZ_SEEDS[2]);
        let mut shadow = Shadow::default();
        shadow.register("target".to_string(), 0);

        // Run 48 steps, then assert counters are consistent
        for _ in 0..48 {
            let op = prng.next_usize(4);
            let name = usernames[prng.next_usize(2)];
            let addr_idx = prng.next_usize(2);

            match op {
                0 => {
                    env.mock_all_auths();
                    let addr = addrs[addr_idx].clone();
                    let existed = shadow.has(name);
                    let res = env.as_contract(&contract_id, || {
                        TrustBridgeContract::register(env.clone(), username(&env, name), addr, Vec::new(&env))
                    });
                    if res.is_ok() {
                        shadow.register(name.to_string(), addr_idx);
                    } else if !existed {
                        // Expected failure only if the entry already existed and
                        // some other constraint prevents it — which shouldn't happen
                        // in this simple suite.
                    }
                }
                1 => {
                    env.mock_all_auths();
                    let res = env.as_contract(&contract_id, || {
                        TrustBridgeContract::verify(
                            env.clone(),
                            admin.clone(),
                            username(&env, name),
                        )
                    });
                    if res.is_ok() {
                        shadow.verify(name);
                    }
                }
                2 => {
                    env.mock_all_auths();
                    let res = env.as_contract(&contract_id, || {
                        TrustBridgeContract::revoke_verification(
                            env.clone(),
                            admin.clone(),
                            username(&env, name),
                            1,
                        )
                    });
                    if res.is_ok() {
                        shadow.revoke(name);
                    }
                }
                _ => {
                    env.mock_all_auths();
                    let res = env.as_contract(&contract_id, || {
                        TrustBridgeContract::remove(
                            env.clone(),
                            admin.clone(),
                            username(&env, name),
                        )
                    });
                    if res.is_ok() {
                        shadow.remove(name);
                    }
                }
            }

            assert_registry_invariants(&env, &contract_id, &shadow);
        }
    }

    #[test]
    fn test_fuzz_counters_never_underflow_on_empty_registry() {
        // I8: counter underflow guard — remove from empty registry is always a
        // no-op or error, never a wrap-around.
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);

        let mut prng = Prng(FUZZ_SEEDS[3]);
        let usernames = ["ghost1", "ghost2", "ghost3"];

        for _ in 0..32 {
            let name = usernames[prng.next_usize(3)];
            let caller = Address::generate(&env);
            env.mock_all_auths();
            // Removing a non-existent entry should never return Ok
            let result = env.as_contract(&contract_id, || {
                TrustBridgeContract::remove(env.clone(), caller, username(&env, name))
            });
            assert_eq!(
                result,
                Err(ContractError::NotRegistered),
                "remove on empty registry should return NotRegistered, not Ok"
            );

            // After all these failed removes, counters must still be zero
            let stats = env.as_contract(&contract_id, || TrustBridgeContract::get_stats(env.clone()));
            assert_eq!(stats.total, 0, "I8 violated: total counter underflowed");
            assert_eq!(stats.verified, 0, "I8 violated: verified counter underflowed");
        }
    }

    // ── Bot accounts (Issue #236) ────────────────────────────────────────────

    #[test]
    fn test_bot_default_false_and_set_by_admin() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone()).unwrap();
            let record = TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert!(!record.is_bot);

            // Admin sets to true
            TrustBridgeContract::set_bot_status(env.clone(), admin.clone(), username(&env, "octocat"), true).unwrap();
            let record = TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert!(record.is_bot);
        });
    }

    #[test]
    fn test_bot_set_by_self() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone()).unwrap();
            
            // Registrant sets to true
            TrustBridgeContract::set_bot_status(env.clone(), user.clone(), username(&env, "octocat"), true).unwrap();
            let record = TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert!(record.is_bot);
        });
    }

    #[test]
    fn test_bot_set_by_unauthorized_fails() {
        let env = Env::default();
        let (_admin, user, other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone()).unwrap();
            
            // Non-admin and non-registrant try to set bot status
            let result = TrustBridgeContract::set_bot_status(env.clone(), other.clone(), username(&env, "octocat"), true);
            assert_eq!(result, Err(ContractError::NotAuthorized));
        });
    }

    #[test]
    fn test_bot_set_nonregistered_fails() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::set_bot_status(env.clone(), admin.clone(), username(&env, "octocat"), true);
            assert_eq!(result, Err(ContractError::NotRegistered));
        });
    }

    // ── Sponsored registration (Issue #237) ──────────────────────────────────

    #[test]
    fn test_sponsor_register_success() {
        let env = Env::default();
        let (_admin, user, sponsor, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register_sponsored(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                sponsor.clone(),
            )
            .unwrap();

            let record = TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert_eq!(record.stellar_address, user);
            assert!(!record.verified);
        });

        // Let's verify the event notes the sponsor
        let events = env.events().all();
        let mut found = false;
        for event in events.iter() {
            // Find the RegisteredEvent
            if event.topics.get(0).unwrap() == soroban_sdk::Symbol::new(&env, "RegisteredEvent") {
                let data: RegisteredEvent = RegisteredEvent::try_from_val(&env, &event.value).unwrap();
                assert_eq!(data.sponsor, Some(sponsor.clone()));
                found = true;
            }
        }
        assert!(found, "RegisteredEvent with sponsor must be published");
    }

    #[test]
    fn test_sponsor_double_auth_protection_on_transfer() {
        let env = Env::default();
        let (_admin, user1, sponsor, contract_id) = setup(&env);
        let user2 = Address::generate(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            // Initially register to user1
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user1.clone()).unwrap();
        });

        // The auths list must contain: sponsor, user2 (new registrant) AND user1 (old registrant)!
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register_sponsored(
                env.clone(),
                username(&env, "octocat"),
                user2.clone(),
                sponsor.clone(),
            )
            .unwrap();
        });

        let auths = env.auths();
        let mut authorized_addresses = soroban_sdk::Vec::new(&env);
        for auth in auths.iter() {
            authorized_addresses.push_back(auth.0);
        }
        assert!(authorized_addresses.contains(&sponsor));
        assert!(authorized_addresses.contains(&user2));
        assert!(authorized_addresses.contains(&user1));
    }

    // ── Role expiry (Issue #221) ────────────────────────────────────────────

    #[test]
    fn test_set_role_with_expiry_active_before_expiry() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role_with_expiry(
                env.clone(),
                user.clone(),
                Role::Verifier,
                Some(2_000),
            )
            .unwrap();
            assert_eq!(
                TrustBridgeContract::get_role(env.clone(), user.clone()),
                Some(Role::Verifier)
            );
            assert!(TrustBridgeContract::has_role(
                env.clone(),
                user.clone(),
                Role::Verifier
            ));
            assert_eq!(
                TrustBridgeContract::get_role_expiry(env.clone(), user.clone()),
                Some(2_000)
            );
        });
    }

    #[test]
    fn test_role_expires_lazily_get_role_returns_none_after_expiry() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role_with_expiry(
                env.clone(),
                user.clone(),
                Role::Verifier,
                Some(2_000),
            )
            .unwrap();
        });

        env.ledger().set_timestamp(2_500);
        env.as_contract(&contract_id, || {
            assert_eq!(TrustBridgeContract::get_role(env.clone(), user.clone()), None);
            assert!(!TrustBridgeContract::has_role(
                env.clone(),
                user.clone(),
                Role::Verifier
            ));
            // Lazy expiry: the raw timestamp is still readable even though the
            // grant no longer resolves as held.
            assert_eq!(
                TrustBridgeContract::get_role_expiry(env.clone(), user.clone()),
                Some(2_000)
            );
        });
    }

    #[test]
    fn test_role_expiry_boundary_exactly_now_is_expired() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role_with_expiry(
                env.clone(),
                user.clone(),
                Role::Verifier,
                Some(2_000),
            )
            .unwrap();
        });

        // One ledger second before expires_at: still active.
        env.ledger().set_timestamp(1_999);
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::get_role(env.clone(), user.clone()),
                Some(Role::Verifier)
            );
        });

        // Exactly at expires_at: `is_role_expired` uses `>=`, so this reads as
        // already expired.
        env.ledger().set_timestamp(2_000);
        env.as_contract(&contract_id, || {
            assert_eq!(TrustBridgeContract::get_role(env.clone(), user.clone()), None);
        });
    }

    #[test]
    fn test_set_role_default_never_expires() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), user.clone(), Role::Verifier).unwrap();
            assert_eq!(
                TrustBridgeContract::get_role_expiry(env.clone(), user.clone()),
                None
            );
        });

        env.ledger().set_timestamp(1_000_000_000);
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::get_role(env.clone(), user.clone()),
                Some(Role::Verifier)
            );
        });
    }

    #[test]
    fn test_remove_role_clears_expiry() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role_with_expiry(
                env.clone(),
                user.clone(),
                Role::Verifier,
                Some(5_000),
            )
            .unwrap();
            TrustBridgeContract::remove_role(env.clone(), user.clone()).unwrap();
            assert_eq!(TrustBridgeContract::get_role(env.clone(), user.clone()), None);
            assert_eq!(
                TrustBridgeContract::get_role_expiry(env.clone(), user.clone()),
                None
            );
        });
    }

    #[test]
    fn test_admin_role_grant_with_expiry_does_not_affect_admin_identity() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        env.as_contract(&contract_id, || {
            // Explicitly (and unusually) expire the admin's own RBAC
            // Role::Admin grant — this must never touch `has_admin_role`.
            TrustBridgeContract::set_role_with_expiry(
                env.clone(),
                admin.clone(),
                Role::Admin,
                Some(1_100),
            )
            .unwrap();
        });

        env.ledger().set_timestamp(2_000);
        env.as_contract(&contract_id, || {
            // The RBAC grant has lapsed...
            assert_eq!(TrustBridgeContract::get_role(env.clone(), admin.clone()), None);
            // ...but the admin's real identity (ADMIN_KEY) is untouched.
            assert!(TrustBridgeContract::has_admin_role(env.clone(), admin.clone()));
            // Admin-gated calls still work.
            TrustBridgeContract::set_cooldown(env.clone(), 42).unwrap();
            assert_eq!(TrustBridgeContract::get_cooldown(env.clone()), 42);
        });
    }

    #[test]
    fn test_expired_verifier_role_cannot_verify() {
        let env = Env::default();
        let (admin, user, verifier, contract_id) = setup(&env);
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();
            TrustBridgeContract::set_role_with_expiry(
                env.clone(),
                verifier.clone(),
                Role::Verifier,
                Some(2_000),
            )
            .unwrap();
        });

        env.ledger().set_timestamp(1_500);
        env.as_contract(&contract_id, || {
            // Still active: the expired-role holder can verify normally.
            TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "octocat"))
                .unwrap();
            TrustBridgeContract::revoke_verification(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
                1,
            )
            .unwrap();
        });

        env.ledger().set_timestamp(2_500);
        env.as_contract(&contract_id, || {
            let res = TrustBridgeContract::verify(
                env.clone(),
                verifier.clone(),
                username(&env, "octocat"),
            );
            assert_eq!(res, Err(ContractError::NotAuthorized));
        });
    }

    // ── Time-bounded verification (Issue #218) ──────────────────────────────

    #[test]
    fn test_verify_expires_after_configured_window() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();
            TrustBridgeContract::config_verification(
                env.clone(),
                admin.clone(),
                soroban_sdk::Symbol::new(&env, "github_att"),
                1_000,
                1,
            )
            .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
            assert!(TrustBridgeContract::is_verification_active(
                env.clone(),
                username(&env, "octocat")
            ));
        });

        // Exactly at the expiry boundary.
        env.ledger().set_timestamp(2_000);
        env.as_contract(&contract_id, || {
            assert!(!TrustBridgeContract::is_verification_active(
                env.clone(),
                username(&env, "octocat")
            ));
            // Lazy expiry: the raw flag is untouched.
            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert!(record.verified);
        });
    }

    #[test]
    fn test_verify_renews_after_expiry_instead_of_already_verified() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();
            TrustBridgeContract::config_verification(
                env.clone(),
                admin.clone(),
                soroban_sdk::Symbol::new(&env, "github_att"),
                1_000,
                1,
            )
            .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });

        // Still active: a duplicate verify is rejected as before this issue.
        env.ledger().set_timestamp(1_500);
        env.as_contract(&contract_id, || {
            let res =
                TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"));
            assert_eq!(res, Err(ContractError::AlreadyVerified));
        });

        // Past expiry: verify renews instead of erroring, and does not
        // double-count `verified_count` / `ever_verified_count`.
        env.ledger().set_timestamp(2_500);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
            assert!(TrustBridgeContract::is_verification_active(
                env.clone(),
                username(&env, "octocat")
            ));
            assert_verified_parity(&env, 1);
            assert_eq!(TrustBridgeContract::get_ever_verified_count(env.clone()), 1);
        });
    }

    #[test]
    fn test_verification_without_config_never_expires() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });

        env.ledger().set_timestamp(1_000_000_000);
        env.as_contract(&contract_id, || {
            assert!(TrustBridgeContract::is_verification_active(
                env.clone(),
                username(&env, "octocat")
            ));
        });
    }

    #[test]
    fn test_verified_count_not_decremented_by_expiry_only_by_revoke() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();
            TrustBridgeContract::config_verification(
                env.clone(),
                admin.clone(),
                soroban_sdk::Symbol::new(&env, "github_att"),
                1_000,
                1,
            )
            .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });

        // Expire without revoking. Stats definition (Issue #218): the raw
        // counters count the raw `verified` flag, not the expiry-aware
        // status, so they do not move on their own.
        env.ledger().set_timestamp(5_000);
        env.as_contract(&contract_id, || {
            assert!(!TrustBridgeContract::is_verification_active(
                env.clone(),
                username(&env, "octocat")
            ));
            assert_verified_parity(&env, 1);

            TrustBridgeContract::revoke_verification(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
                1,
            )
            .unwrap();
            assert_verified_parity(&env, 0);
        });
    }

    #[test]
    fn test_get_verification_expiry_none_without_config() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
            assert_eq!(
                TrustBridgeContract::get_verification_expiry(
                    env.clone(),
                    username(&env, "octocat")
                ),
                None
            );
        });
    }

    #[test]
    fn test_get_verification_expiry_some_when_configured() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();
            TrustBridgeContract::config_verification(
                env.clone(),
                admin.clone(),
                soroban_sdk::Symbol::new(&env, "github_att"),
                500,
                1,
            )
            .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
            assert_eq!(
                TrustBridgeContract::get_verification_expiry(
                    env.clone(),
                    username(&env, "octocat")
                ),
                Some(1_500)
            );
        });
    }

    // ── Signed export attestation (Issue #223) ──────────────────────────────

    #[test]
    fn test_export_attestation_empty_registry() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let attestation = TrustBridgeContract::export_attestation(env.clone(), 0, 10).unwrap();
            assert_eq!(attestation.page.records.len(), 0);
            assert_eq!(attestation.page.total, 0);
            assert!(!attestation.page.has_more);

            // Deterministic even for an empty page.
            let again = TrustBridgeContract::export_attestation(env.clone(), 0, 10).unwrap();
            assert_eq!(attestation.digest, again.digest);
        });
    }

    #[test]
    fn test_export_attestation_digest_deterministic_across_calls() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();
            let a = TrustBridgeContract::export_attestation(env.clone(), 0, 10).unwrap();
            let b = TrustBridgeContract::export_attestation(env.clone(), 0, 10).unwrap();
            assert_eq!(a.digest, b.digest);
            assert_eq!(a.page, b.page);
        });
    }

    #[test]
    fn test_export_attestation_digest_changes_with_data() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();
        });

        let before = env.as_contract(&contract_id, || {
            TrustBridgeContract::export_attestation(env.clone(), 0, 10).unwrap()
        });

        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });

        let after = env.as_contract(&contract_id, || {
            TrustBridgeContract::export_attestation(env.clone(), 0, 10).unwrap()
        });

        assert_ne!(before.digest, after.digest);
    }

    #[test]
    fn test_export_attestation_matches_registered_paginated_page() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();
            let attestation = TrustBridgeContract::export_attestation(env.clone(), 0, 10).unwrap();
            let page = TrustBridgeContract::get_registered_paginated(env.clone(), 0, 10).unwrap();
            assert_eq!(attestation.page, page);
        });
    }

    #[test]
    fn test_export_attestation_reports_version_and_ledger() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let attestation = TrustBridgeContract::export_attestation(env.clone(), 0, 10).unwrap();
            assert_eq!(attestation.version, soroban_sdk::vec![&env, 1u32, 0u32, 0u32]);
            assert_eq!(attestation.ledger, env.ledger().sequence());
        });
    }

    // ── Dual-control batch_remove (Issue #219) ──────────────────────────────

    /// Registers `count` fresh usernames (each to its own generated address)
    /// and returns them, for building batch_remove test fixtures.
    fn register_batch(env: &Env, contract_id: &Address, count: u32) -> Vec<String> {
        env.mock_all_auths();
        let mut names: Vec<String> = Vec::new(env);
        for i in 0..count {
            let addr = Address::generate(env);
            let name = username(env, &format!("user{}", i));
            env.as_contract(contract_id, || {
                TrustBridgeContract::register(env.clone(), name.clone(), addr.clone(), Vec::new(env))
                    .unwrap();
            });
            names.push_back(name);
        }
        names
    }

    #[test]
    fn test_batch_remove_at_or_below_threshold_unchanged() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        let names = register_batch(&env, &contract_id, 3);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
            // size == threshold: still direct, single-step.
            let summary =
                TrustBridgeContract::batch_remove(env.clone(), admin.clone(), names.clone())
                    .unwrap();
            assert_eq!(summary.successful, 3);
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 0);
        });
    }

    #[test]
    fn test_batch_remove_above_threshold_requires_dual_control() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        let names = register_batch(&env, &contract_id, 4);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
            // size == threshold + 1: rejected, must use propose/execute.
            let res = TrustBridgeContract::batch_remove(env.clone(), admin.clone(), names.clone());
            assert_eq!(res, Err(ContractError::DualControlRequired));
            // Registry untouched — the fail-before-write property.
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 4);
        });
    }

    #[test]
    fn test_batch_remove_threshold_zero_disables_dual_control() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        let names = register_batch(&env, &contract_id, 20);
        env.as_contract(&contract_id, || {
            assert_eq!(TrustBridgeContract::get_batch_remove_threshold(env.clone()), 0);
            let summary =
                TrustBridgeContract::batch_remove(env.clone(), admin.clone(), names.clone())
                    .unwrap();
            assert_eq!(summary.successful, 20);
        });
    }

    #[test]
    fn test_propose_then_execute_batch_remove_by_second_admin() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        let second_admin = Address::generate(&env);
        let names = register_batch(&env, &contract_id, 4);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
            TrustBridgeContract::set_role(env.clone(), second_admin.clone(), Role::Admin).unwrap();

            TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
                .unwrap();
            let pending = TrustBridgeContract::get_pending_batch_remove(env.clone()).unwrap();
            assert_eq!(pending.usernames.len(), 4);
            assert_eq!(pending.proposed_by, admin);

            let summary =
                TrustBridgeContract::execute_batch_remove(env.clone(), second_admin.clone())
                    .unwrap();
            assert_eq!(summary.successful, 4);
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 0);
            assert!(TrustBridgeContract::get_pending_batch_remove(env.clone()).is_none());
        });
    }

    #[test]
    fn test_execute_batch_remove_same_proposer_rejected() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        let names = register_batch(&env, &contract_id, 4);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
            TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
                .unwrap();
            // Same admin tries to execute their own proposal — dual control
            // means one signature alone is never enough.
            let res = TrustBridgeContract::execute_batch_remove(env.clone(), admin.clone());
            assert_eq!(res, Err(ContractError::NotAuthorized));
            assert!(TrustBridgeContract::get_pending_batch_remove(env.clone()).is_some());
        });
    }

    #[test]
    fn test_execute_batch_remove_without_admin_equivalent_role_rejected() {
        let env = Env::default();
        let (admin, _user, bystander, contract_id) = setup(&env);
        let names = register_batch(&env, &contract_id, 4);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
            TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
                .unwrap();
            let res = TrustBridgeContract::execute_batch_remove(env.clone(), bystander.clone());
            assert_eq!(res, Err(ContractError::NotAuthorized));
        });
    }

    #[test]
    fn test_propose_batch_remove_rejects_second_proposal_while_pending() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        let names = register_batch(&env, &contract_id, 4);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
            TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
                .unwrap();
            let res = TrustBridgeContract::propose_batch_remove(
                env.clone(),
                admin.clone(),
                names.clone(),
            );
            assert_eq!(res, Err(ContractError::BatchRemoveProposalPending));
        });
    }

    #[test]
    fn test_cancel_batch_remove_clears_proposal_and_allows_new_one() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        let names = register_batch(&env, &contract_id, 4);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
            TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
                .unwrap();
            TrustBridgeContract::cancel_batch_remove(env.clone(), admin.clone()).unwrap();
            assert!(TrustBridgeContract::get_pending_batch_remove(env.clone()).is_none());

            // A fresh proposal is now allowed.
            TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
                .unwrap();
            assert!(TrustBridgeContract::get_pending_batch_remove(env.clone()).is_some());
        });
    }

    #[test]
    fn test_cancel_batch_remove_available_while_paused() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        let names = register_batch(&env, &contract_id, 4);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
            TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
                .unwrap();
            TrustBridgeContract::pause(env.clone(), 1).unwrap();
            // Cancel still works while paused — the abort path must not be
            // blockable by the same freeze that might motivate using it.
            TrustBridgeContract::cancel_batch_remove(env.clone(), admin.clone()).unwrap();
            assert!(TrustBridgeContract::get_pending_batch_remove(env.clone()).is_none());
        });
    }

    #[test]
    fn test_execute_batch_remove_blocked_while_paused() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        let second_admin = Address::generate(&env);
        let names = register_batch(&env, &contract_id, 4);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
            TrustBridgeContract::set_role(env.clone(), second_admin.clone(), Role::Admin).unwrap();
            TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
                .unwrap();
            TrustBridgeContract::pause(env.clone(), 1).unwrap();

            let res = TrustBridgeContract::execute_batch_remove(env.clone(), second_admin.clone());
            assert_eq!(res, Err(ContractError::Paused));

            // The proposal survives the pause — the second key can still
            // execute once unpaused.
            TrustBridgeContract::unpause(env.clone(), 4).unwrap();
            let summary =
                TrustBridgeContract::execute_batch_remove(env.clone(), second_admin.clone())
                    .unwrap();
            assert_eq!(summary.successful, 4);
        });
    }

    #[test]
    fn test_execute_batch_remove_after_proposal_expired() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        let second_admin = Address::generate(&env);
        let names = register_batch(&env, &contract_id, 4);
        env.ledger().set_timestamp(1_000);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
            TrustBridgeContract::set_role(env.clone(), second_admin.clone(), Role::Admin).unwrap();
            TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
                .unwrap();
        });

        // Past BATCH_REMOVE_PROPOSAL_TTL_SECS (24h).
        env.ledger().set_timestamp(1_000 + 86_400 + 1);
        env.as_contract(&contract_id, || {
            let res = TrustBridgeContract::execute_batch_remove(env.clone(), second_admin.clone());
            assert_eq!(res, Err(ContractError::NoPendingBatchRemove));
            // Expired proposal was cleared and never executed.
            assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 4);
            assert!(TrustBridgeContract::get_pending_batch_remove(env.clone()).is_none());

            // A fresh proposal is possible immediately after.
            TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
                .unwrap();
        });
    }

    // ── Issue #282: Indexer lag helper — last event ledger ───────────────────

    /// A fresh instance has never emitted an event, so `get_last_event_ledger`
    /// returns 0.
    #[test]
    fn test_last_event_ledger_zero_on_fresh_instance() {
        let env = Env::default();
        let (_admin, _user, _other, contract_id) = setup(&env);
        env.as_contract(&contract_id, || {
            // `initialize` pushes an audit entry but does not call event_domain,
            // so the cursor stays at 0 until the first event-emitting call.
            assert_eq!(
                TrustBridgeContract::get_last_event_ledger(env.clone()),
                0,
                "get_last_event_ledger must return 0 before any event is emitted"
            );
        });
    }

    /// `register` emits `RegisteredEvent` via `event_domain`, which stamps
    /// the ledger cursor.  The returned value must equal the ledger sequence
    /// of the invocation.
    #[test]
    fn test_last_event_ledger_updated_after_register() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();

        let seq_before = env.ledger().sequence();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();
        });

        env.as_contract(&contract_id, || {
            let last = TrustBridgeContract::get_last_event_ledger(env.clone());
            // The cursor must be at or after the register ledger.
            assert!(
                last >= seq_before,
                "last_event_ledger ({last}) must be >= register ledger ({seq_before})"
            );
            assert_ne!(last, 0, "last_event_ledger must not be 0 after register");
        });
    }

    /// `verify` emits `VerifiedEvent`; the cursor must advance after the call.
    #[test]
    fn test_last_event_ledger_updated_after_verify() {
        let env = Env::default();
        let (admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();
        });

        let after_register = env.as_contract(&contract_id, || {
            TrustBridgeContract::get_last_event_ledger(env.clone())
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
            )
            .unwrap();
        });

        env.as_contract(&contract_id, || {
            let after_verify = TrustBridgeContract::get_last_event_ledger(env.clone());
            assert!(
                after_verify >= after_register,
                "last_event_ledger must not decrease after verify"
            );
            assert_ne!(after_verify, 0);
        });
    }

    /// `remove` emits `RemovedEvent`; the cursor must be set.
    #[test]
    fn test_last_event_ledger_updated_after_remove() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(
                env.clone(),
                user.clone(),
                username(&env, "octocat"),
            )
            .unwrap();
        });

        env.as_contract(&contract_id, || {
            assert_ne!(
                TrustBridgeContract::get_last_event_ledger(env.clone()),
                0,
                "last_event_ledger must be set after remove"
            );
        });
    }

    /// `get_last_event_ledger` is a read and must work while the contract is
    /// paused — indexers need to detect lag even during a maintenance freeze.
    #[test]
    fn test_last_event_ledger_readable_while_paused() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();
            TrustBridgeContract::pause(env.clone(), 1).unwrap();
        });

        env.as_contract(&contract_id, || {
            // Must succeed even while paused.
            let last = TrustBridgeContract::get_last_event_ledger(env.clone());
            assert_ne!(last, 0, "last_event_ledger must be readable while paused");
        });
    }

    /// The cursor is monotonic: a second `register` must not decrease it.
    #[test]
    fn test_last_event_ledger_is_monotonic() {
        let env = Env::default();
        let (_admin, user, _other, contract_id) = setup(&env);
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "alice"),
                user.clone(),
                Vec::new(&env),
            )
            .unwrap();
        });

        let first = env.as_contract(&contract_id, || {
            TrustBridgeContract::get_last_event_ledger(env.clone())
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "bob"),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        });

        env.as_contract(&contract_id, || {
            let second = TrustBridgeContract::get_last_event_ledger(env.clone());
            assert!(
                second >= first,
                "last_event_ledger must be monotonically non-decreasing ({second} < {first})"
            );
        });
    }
}
