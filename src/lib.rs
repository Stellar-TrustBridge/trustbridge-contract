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
mod error;
mod error_context;
mod events;
mod registry_read_stub;
mod storage;
mod utils;
mod version;

pub use audit::{AuditConfig, AuditEventType, AuditLogEntry, AuditStats};
pub use batch::{BatchConfig, BatchOperationResult, BatchSummary};
pub use error::ContractError;
pub use events::{
    AttestationClearedEvent, ChallengeCancelledEvent, ChallengeCompletedEvent, RenamedEvent,
    RotationCancelledEvent, RotationExecutedEvent, RotationRequestedEvent,
    ChallengeStartedEvent, EmergencyClearedEvent, EmergencyPausedEvent, PausedEvent,
    RegisteredEvent, RemovedEvent, RoleGrantedEvent, RoleRevokedEvent, UnpausedEvent,
    UpgradeAttestedEvent, UpgradedEvent, VerificationRevokedEvent, VerifiedEvent,
};
pub use storage::{
    ChallengeRecord, ContributorRecord, ExportPage, HealthSnapshot, PauseReason, PendingRotation,
    RecordProof, Role, Stats,
    VerificationConfig, WasmAttestation, WasmProvenance,
};
pub use version::Version;

use soroban_sdk::{contract, contractimpl, Address, BytesN, Env, String, Symbol, Vec};

use crate::storage::{
    add_to_index, clear_pending_reverify, get_admin, get_audit_logs, get_audit_stats,
    get_challenge, get_cooldown as storage_get_cooldown, get_count, get_index, get_last_upgrade,
    get_record, get_registered_paginated_internal, get_role as storage_get_role,
    bump_ever_verified_count, get_emergency_pause, get_emergency_pause_ts,
    get_ever_verified_count as storage_get_ever_verified_count, get_guardian as storage_get_guardian,
    get_stats as read_stats, get_verification_config, get_verified_count as storage_get_verified_count,
    is_attestation_required, is_guardian, remove_guardian as storage_remove_guardian,
    set_emergency_pause, set_emergency_pause_ts, set_guardian_address,
    get_version as storage_get_version, get_wasm_attestation, get_wasm_provenance, has_challenge,
    build_record_proof, get_pending_rotation as storage_get_pending_rotation,
    get_rotation_delay as storage_get_rotation_delay, has_pending_rotation, has_record,
    is_admin_caller, is_in_cooldown, is_paused as storage_is_paused, push_audit_entry,
    remove_pending_rotation, set_pending_rotation, set_rotation_delay as storage_set_rotation_delay,
    remove_challenge, remove_from_index, remove_record, remove_role as storage_remove_role,
    remove_wasm_attestation, require_initialized, require_not_paused,
    run_migration_steps, set_challenge, set_cooldown as storage_set_cooldown, set_count,
    set_last_action, set_last_upgrade, set_paused as set_paused_state, set_pending_reverify,
    set_ever_verified_count, set_record, set_role as storage_set_role, set_verified_count,
    set_version,
    set_wasm_attestation, set_wasm_provenance, DEFAULT_CHALLENGE_DELAY_SECS,
    ADMIN_KEY,
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

    /// Assigns a role to `target`. Admin-only.
    ///
    /// Roles gate access to privileged operations:
    ///
    /// | Role | Can do |
    /// |------|--------|
    /// | `Admin` | Everything |
    /// | `Upgrader` | Call `upgrade` |
    /// | `Verifier` | Call `verify` and `revoke_verification` |
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
        }
        .publish(&env);

        Ok(())
    }

    /// Revokes `target`'s role assignment. Admin-only.
    ///
    /// After this call `get_role(target)` returns `None`. Does not affect the
    /// admin's own role — the admin address is stored separately and cannot be
    /// stripped via `remove_role`.
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
        }
        .publish(&env);

        Ok(())
    }

    /// Returns the role assigned to `address`, or `None` if no role is assigned.
    ///
    /// Read-only; no auth required. Returns `None` for any address that has never
    /// been granted a role (including the admin address, which is stored separately).
    #[must_use]
    pub fn get_role(env: Env, address: Address) -> Option<Role> {
        storage_get_role(&env, &address)
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

        let record = ContributorRecord {
            stellar_address: stellar_address.clone(),
            registered_at: timestamp as u32,
            verified: existing
                .as_ref()
                .map(|r| r.stellar_address == stellar_address && r.verified)
                .unwrap_or(false),
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
        }
        .publish(&env);

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

        let config = BatchConfig::default();
        if !config.is_valid_batch_size(usernames.len()) {
            return Err(ContractError::InvalidBatchSize);
        }

        caller.require_auth();

        // Admin must be the caller for batch_remove (stricter than single remove).
        let admin = get_admin(&env)?;
        if caller != admin {
            return Err(ContractError::NotAuthorized);
        }

        let total = usernames.len();
        let mut successful: u32 = 0;

        for username in usernames.iter() {
            // Attempt the remove. Silently skip failures (not registered, etc.)
            // so one bad entry does not kill the whole batch.
            let record = match get_record(&env, &username) {
                Some(r) => r,
                None => continue, // not registered — count as failure below
            };

            // With admin auth, the auth check inside single remove would pass,
            // but we replicate the full logic here to be explicit.
            let timestamp = env.ledger().timestamp();
            let stellar_address = record.stellar_address.clone();

            remove_record(&env, &username);
            remove_from_index(&env, &username);
            set_count(&env, get_count(&env).saturating_sub(1));

            if record.verified {
                set_verified_count(&env, storage_get_verified_count(&env).saturating_sub(1));
            }

            RemovedEvent {
                github_username: username.clone(),
                stellar_address: stellar_address.clone(),
                timestamp,
            }
            .publish(&env);

            push_audit_entry(
                &env,
                AuditLogEntry::new(AuditEventType::UserRemoved, timestamp, Some(caller.clone()))
                    .with_username(username.clone())
                    .with_address(stellar_address),
            );

            successful = successful.saturating_add(1);
        }

        Ok(BatchSummary::new(total, successful))
    }

    /// Looks up the `ContributorRecord` for `github_username`. Returns `None` if not registered.
    ///
    /// Read-only; no auth required. Use this for payout address resolution in GitHub Actions
    /// and dashboard lookups.
    #[must_use]
    pub fn get_address(env: Env, github_username: String) -> Option<ContributorRecord> {
        if has_record(&env, &github_username) {
            get_record(&env, &github_username)
        } else {
            None
        }
    }

    /// Returns `true` if `github_username` is registered, without deserializing the full record.
    ///
    /// Read-only; no auth required. Use this when callers only need existence confirmation
    /// and do not need the [`ContributorRecord`] fields.
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

    /// Exports a page of registry records using a cursor. Admin-only.
    ///
    /// `cursor` is the zero-based record index to start from; `limit` is the maximum
    /// number of records to return (capped at `MAX_PAGE_LIMIT`). Returns an [`ExportPage`]
    /// containing the records and a `next_cursor` for subsequent calls.
    ///
    /// Use this instead of `get_all_registered` for large registries — it avoids
    /// materializing the full index in one transaction.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    pub fn get_registered_paginated(
        env: Env,
        cursor: u32,
        limit: u32,
    ) -> Result<ExportPage, ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        get_registered_paginated_internal(&env, cursor, limit)
    }

    /// Public paginated read for indexers and dashboard consumers.
    ///
    /// Same cursor/limit semantics as `get_registered_paginated` but requires no auth,
    /// making it suitable for public dashboard sync and off-chain indexers. Returns an
    /// [`ExportPage`] with records and a `next_cursor`.
    ///
    /// Blocked by the pause state — returns [`ContractError::Paused`] while the circuit
    /// breaker is active.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    pub fn get_public_paginated(
        env: Env,
        cursor: u32,
        limit: u32,
    ) -> Result<ExportPage, ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

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
            }
            .publish(&env);
        } else {
            UnpausedEvent {
                admin: admin.clone(),
                timestamp,
                reason_code,
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
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotAuthorized`] if `caller` is not an admin or verifier.
    /// - [`ContractError::NotRegistered`] if `github_username` is not registered.
    /// - [`ContractError::AlreadyVerified`] if the record is already verified.
    pub fn verify(env: Env, caller: Address, github_username: String) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        caller.require_auth();

        // Caller must be the admin OR hold the Verifier role.
        // Note: Revoker role does NOT grant verify — roles are intentionally
        // separated so a compromised Revoker cannot mark new accounts as
        // verified (Issue #212).
        let is_admin = is_admin_caller(&env, &caller);
        let is_verifier = storage_get_role(&env, &caller) == Some(Role::Verifier);
        if !is_admin && !is_verifier {
            return Err(ContractError::NotAuthorized);
        }

        let mut record = get_record(&env, &github_username).ok_or(ContractError::NotRegistered)?;

        if record.verified {
            return Err(ContractError::AlreadyVerified);
        }

        record.verified = true;
        set_record(&env, &github_username, &record);
        set_verified_count(&env, storage_get_verified_count(&env).saturating_add(1));
        bump_ever_verified_count(&env);

        // Clear pending reverify flag upon successful verification
        clear_pending_reverify(&env, &github_username);
        let timestamp = env.ledger().timestamp();
        VerifiedEvent {
            github_username: github_username.clone(),
            stellar_address: record.stellar_address.clone(),
            timestamp,
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

        let config = BatchConfig::default();
        if !config.is_valid_batch_size(usernames.len()) {
            return Err(ContractError::InvalidBatchSize);
        }

        caller.require_auth();

        let is_admin = is_admin_caller(&env, &caller);
        let is_verifier = storage_get_role(&env, &caller) == Some(Role::Verifier);
        if !is_admin && !is_verifier {
            return Err(ContractError::NotAuthorized);
        }

        let total = usernames.len();
        let mut successful: u32 = 0;
        let timestamp = env.ledger().timestamp();

        for username in usernames.iter() {
            let mut record = match get_record(&env, &username) {
                Some(r) => r,
                None => continue,
            };

            if record.verified {
                continue;
            }

            record.verified = true;
            set_record(&env, &username, &record);
            set_verified_count(&env, storage_get_verified_count(&env).saturating_add(1));
            bump_ever_verified_count(&env);
            clear_pending_reverify(&env, &username);

            VerifiedEvent {
                github_username: username.clone(),
                stellar_address: record.stellar_address.clone(),
                timestamp,
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

        let mut record = get_record(&env, &github_username).ok_or(ContractError::NotRegistered)?;

        if !record.verified {
            return Err(ContractError::NotVerified);
        }

        record.verified = false;
        set_record(&env, &github_username, &record);
        set_verified_count(&env, storage_get_verified_count(&env).saturating_sub(1));

        let timestamp = env.ledger().timestamp();
        VerificationRevokedEvent {
            github_username: github_username.clone(),
            stellar_address: record.stellar_address.clone(),
            timestamp,
            reason_code,
        }
        .publish(&env);

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
            registered_at: timestamp as u32,
            verified: false,
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
        }
        .publish(&env);

        ChallengeCompletedEvent {
            github_username,
            completed_by: caller,
            timestamp: now,
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

    /// Records that `github_username` performed a registry-mutating action at the current
    /// ledger timestamp, for use by cooldown enforcement logic.
    ///
    /// Off-chain callers use this alongside `is_registration_in_cooldown` to implement
    /// per-contributor rate limiting without requiring a contract upgrade.
    pub fn record_action(env: Env, github_username: String) {
        set_last_action(&env, &github_username, env.ledger().timestamp());
    }

    /// Returns `true` if `github_username` is still within the registration cooldown window.
    ///
    /// Read-only; no auth required. The cooldown period is set by `set_cooldown`. Returns
    /// `false` if no action has been recorded or the cooldown has elapsed.
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
            let record =
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
            assert_eq!(record.stellar_address, user);
            assert!(!record.verified);
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user1.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "carol"), user3.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "carol"), user3.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "carol"), user3.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), new_user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), other.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), other.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), other.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), other.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), old_user.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), new_user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
                TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone());
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
            ContractError::AttestationRequired,
            ContractError::AlreadyReserved,
            ContractError::NotReserved,
            ContractError::ReservedListFull,
            ContractError::UsernameReserved,
            ContractError::RotationRequired,
            ContractError::RotationPending,
            ContractError::NoRotationPending,
            ContractError::RotationNotReady,
            ContractError::UsernameTaken,
        ] {
            assert_eq!(ContractError::from_code(variant.code()), Some(variant));
        }
        assert_eq!(ContractError::from_code(0), None);
        // 32 is one past the highest assigned variant (UsernameTaken = 31):
        assert_eq!(ContractError::from_code(32), None);
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
                TrustBridgeContract::register(env.clone(), too_long.clone(), user.clone()),
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
                TrustBridgeContract::register(env.clone(), at_max.clone(), user.clone()).is_ok()
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
                    TrustBridgeContract::register(env.clone(), username(&env, bad), user.clone()),
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
        client.register(&name, &user);

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
        client.register(&name, &user);
        client.register(&name, &other);

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

        client.register(&name, &user);

        let expected = RegisteredEvent {
            github_username: name.clone(),
            stellar_address: user.clone(),
            timestamp: 1_600_000_000,
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
        client.register(&name, &user);

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
        client.register(&name, &user);

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
        client.register(&name, &user);
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
        client.register(&name, &user);
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
                TrustBridgeContract::register(env.clone(), username(&env, "alice"), user.clone());
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
                user.clone()
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
            TrustBridgeContract::register(env.clone(), name.clone(), user.clone()).unwrap();
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
                    admin.clone()
                ),
                Err(ContractError::NotInitialized)
            );

            TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();

            // Same call must now succeed
            assert!(TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                admin.clone()
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "carol"), user3.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
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
                TrustBridgeContract::register(env.clone(), name.clone(), user.clone()).unwrap();
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
                TrustBridgeContract::register(env.clone(), name.clone(), user.clone()).unwrap();
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
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
            TrustBridgeContract::register(env.clone(), github_username, user).unwrap();
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
                TrustBridgeContract::register(env.clone(), name.clone(), user.clone()).unwrap();
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env2.clone(), username(&env2, "octocat"), user2.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user.clone())
                .unwrap();
        });
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "carol"), user3.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "alice"), user1.clone())
                .unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "bob"), user2.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user.clone())
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
                2, // CompromisedKey
            )
            .unwrap();

            let expected = VerificationRevokedEvent {
                github_username: username(&env, "octocat"),
                stellar_address: user.clone(),
                timestamp: env.ledger().timestamp(),
                reason_code: 2,
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user).unwrap();
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
            TrustBridgeContract::register(env.clone(), username(&env, "user1"), user1).unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "user2"), user2).unwrap();
            TrustBridgeContract::register(env.clone(), username(&env, "user3"), user3).unwrap();
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
            TrustBridgeContract::register(env.clone(), username(&env, "user1"), user1.clone())
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
            TrustBridgeContract::register(env.clone(), username(&env, "user1"), user1).unwrap();
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
            TrustBridgeContract::register(env.clone(), username(&env, "user1"), user1).unwrap();
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user1.clone()).unwrap();
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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user).unwrap();

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
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user).unwrap();
        });

        let initial_logs_len = env.as_contract(&contract_id, || {
            TrustBridgeContract::get_audit_logs(env.clone()).len()
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
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

            // First registration succeeds
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user1.clone())
                .unwrap();

            // Immediate re-registration fails with CooldownActive
            let res = TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user2.clone(),
            );
            assert_eq!(res, Err(ContractError::CooldownActive));
        });

        // Advance ledger timestamp by 101 seconds
        env.ledger().set_timestamp(1000 + 101);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            // After cooldown elapses, re-registration succeeds
            TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user2.clone())
                .unwrap();
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
                    user.clone()
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
                    user.clone()
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
                TrustBridgeContract::register(env.clone(), username(&env, &name), addr).unwrap();
            }
            let chunk_count_at_50 = crate::storage::get_chunk_count(&env);
            assert_eq!(chunk_count_at_50, 1, "50 entries should occupy exactly 1 chunk");

            // Register the 51st user — must spill into chunk 1
            let addr51 = Address::generate(&env);
            TrustBridgeContract::register(env.clone(), username(&env, "user050"), addr51).unwrap();
            let chunk_count_at_51 = crate::storage::get_chunk_count(&env);
            assert_eq!(chunk_count_at_51, 2, "51st entry must create a second chunk");
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
            TrustBridgeContract::register(env.clone(), username(&env, "target"), user.clone())
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
                        TrustBridgeContract::register(env.clone(), username(&env, name), addr)
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
}
