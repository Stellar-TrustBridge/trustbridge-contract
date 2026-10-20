//! Initialization, pause, guardian and emergency-stop entrypoints.
//!
//! Split out of `lib.rs` (Issue #601); see `docs/ARCHITECTURE.md#module-map`.

use crate::*;

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
        // A restored instance from another network must fail closed even when
        // the admin key is already present. Fresh instances have no tag yet.
        crate::storage::require_matching_network(&env)?;
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
        set_version(&env, CONTRACT_VERSION.to_tuple());
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
            AuditLogEntry::new(AuditEventType::ContractPaused, timestamp, Some(admin)),
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
            AuditLogEntry::new(AuditEventType::ContractUnpaused, timestamp, Some(admin)),
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
        GuardianChangedEvent {
            guardian: Some(guardian),
            admin,
            timestamp: env.ledger().timestamp(),
        }
        .publish(&env);
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
        GuardianChangedEvent {
            guardian: None,
            admin,
            timestamp: env.ledger().timestamp(),
        }
        .publish(&env);
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
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "emergency_pause"),
            );
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
            admin: caller.clone(),
            timestamp,
            reason_code: 99,
            domain: event_domain(&env),
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::ContractPaused, timestamp, Some(caller)),
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
            domain: event_domain(&env),
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::ContractUnpaused, timestamp, Some(admin)),
        );

        Ok(())
    }
}
