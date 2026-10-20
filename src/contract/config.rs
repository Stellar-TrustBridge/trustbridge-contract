//! Verification configuration and audit log entrypoints.
//!
//! Split out of `lib.rs` (Issue #601); see `docs/ARCHITECTURE.md#module-map`.

use crate::*;

#[contractimpl]
impl TrustBridgeContract {
    /// One-time configuration of the verification parameters.
    ///
    /// Stores the attestation symbol, expiry window, and threshold. May only
    /// be called once — a second invocation returns [`ContractError::AlreadyInitialized`].
    ///
    /// Emits [`VerificationConfiguredEvent`]. The stored configuration can be
    /// read back with [`get_verification_config`][Self::get_verification_config].
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
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "config_verification"),
            );
            return Err(ContractError::NotAuthorized);
        }

        if crate::storage::is_verification_configured(&env) {
            return Err(ContractError::AlreadyInitialized);
        }

        crate::storage::set_verification_config(&env, attestation.clone(), expires_in, threshold);

        let timestamp = env.ledger().timestamp();
        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::AdminAction, timestamp, Some(caller.clone())),
        );

        VerificationConfiguredEvent {
            admin: caller,
            attestation,
            expires_in,
            threshold,
            timestamp,
        }
        .publish(&env);

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

    // ── Audit configuration (Issue #412) ─────────────────────────────────────

    /// Returns the current on-chain audit configuration.
    ///
    /// Falls back to [`AuditConfig::default`] when no config has been
    /// explicitly stored yet, so callers always get a well-formed value.
    ///
    /// Read-only; no auth required.
    #[must_use]
    pub fn get_audit_config(env: Env) -> AuditConfig {
        storage_get_audit_config(&env)
    }

    /// Stores a new audit configuration. Admin-only.
    ///
    /// Operators use this to tune the on-chain audit trail without redeploying:
    ///
    /// - `enabled` — globally enables or disables audit logging. When `false`,
    ///   `push_audit_entry` calls are no-ops (the audit log does not grow).
    /// - `max_events` — ring-buffer size (capped to `MAX_AUDIT_LOG_ENTRIES`).
    ///   Lowering it does not truncate existing entries until the next write.
    /// - `log_unauthorized` — whether failed auth attempts are recorded as
    ///   `AuditEventType::UnauthorizedAttempt`. Set to `false` to suppress
    ///   spam from probing bots; the event counter in `AuditStats` still
    ///   increments.
    ///
    /// Persists the config and emits [`AuditConfigChangedEvent`].
    /// Also appends an `AuditConfigChanged` log entry so the change itself
    /// is auditable.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if `caller` is not the contract admin.
    pub fn set_audit_config(
        env: Env,
        caller: Address,
        enabled: bool,
        max_events: u32,
        log_unauthorized: bool,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        caller.require_auth();

        let admin = get_admin(&env)?;
        if caller != admin {
            // Record the unauthorized attempt before returning, gated on the
            // *current* config so that a disable-then-probe pattern cannot
            // suppress its own trace.
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "set_audit_config"),
            );
            return Err(ContractError::NotAuthorized);
        }

        let new_config = AuditConfig::custom(enabled, max_events, log_unauthorized);
        storage_set_audit_config(&env, &new_config);

        let timestamp = env.ledger().timestamp();
        crate::events::AuditConfigChangedEvent {
            admin: admin.clone(),
            enabled,
            max_events,
            log_unauthorized,
            timestamp,
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::AuditConfigChanged, timestamp, Some(admin)),
        );

        Ok(())
    }

    /// Internal helper — records an `UnauthorizedAttempt` audit entry when the
    /// current [`AuditConfig`] has `log_unauthorized = true`.
    ///
    /// Call this immediately before returning
    /// `Err(ContractError::NotAuthorized)` at any role-gated entry point.
    /// The write is gated on the stored config so operators can suppress the
    /// entries (e.g. during a spam campaign) via `set_audit_config`.
    ///
    /// The helper is deliberately non-recursive: it calls `push_audit_entry`
    /// directly without re-entering any auth-gated path, and `push_audit_entry`
    /// itself has no auth checks.
    pub(crate) fn record_unauthorized_attempt(env: &Env, actor: &Address, operation: String) {
        let config = storage_get_audit_config(env);
        if !config.log_unauthorized {
            return;
        }
        let timestamp = env.ledger().timestamp();
        push_audit_entry(
            env,
            AuditLogEntry::new(
                AuditEventType::UnauthorizedAttempt,
                timestamp,
                Some(actor.clone()),
            )
            .with_details(operation),
        );
    }
}
