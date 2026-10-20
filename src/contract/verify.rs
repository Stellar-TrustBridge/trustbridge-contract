//! Verification, oracle proofs, revocation and bot status entrypoints.
//!
//! Split out of `lib.rs` (Issue #601); see `docs/ARCHITECTURE.md#module-map`.

use crate::*;

#[contractimpl]
impl TrustBridgeContract {
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
        if !is_admin {
            require_role_not_expired(&env, &caller)?;
        }
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
            Self::record_unauthorized_attempt(&env, &caller, String::from_str(&env, "verify"));
            return Err(ContractError::NotAuthorized);
        }
        // Per-verifier, per-ledger anti-grief cap (Issue #292). Charged before
        // any state read so a spammer calling `verify` on junk usernames still
        // pays into the limit. The admin is exempt.
        if !is_admin {
            charge_verify_rate(&env, &caller, 1)?;
        }

        let record = get_record(&env, &github_username).ok_or(ContractError::NotRegistered)?;
        require_bound_record_address(&env, &record)?;

        // Issue #218: an expired prior verification does not block a fresh
        // one — only a still-active verification does.
        let was_active =
            record.verified && !crate::storage::is_verification_expired(&env, &github_username);
        if was_active {
            return Err(ContractError::AlreadyVerified);
        }

        let record = get_record(&env, &github_username).ok_or(ContractError::NotRegistered)?;
        mark_verified(&env, github_username, record, caller)
    }

    /// Replaces the oracle Ed25519 public keys whose proofs
    /// `verify_with_oracle_proof` accepts. An empty list disables oracle
    /// verification entirely.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    pub fn set_oracle_allowlist(env: Env, pubkeys: Vec<BytesN<32>>) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        crate::oracle_proof::set_oracle_allowlist(&env, &admin, pubkeys);
        Ok(())
    }

    /// Returns the oracle public-key allowlist (empty if never set).
    ///
    /// Read-only; no auth required.
    #[must_use]
    pub fn get_oracle_allowlist(env: Env) -> Vec<BytesN<32>> {
        crate::oracle_proof::get_oracle_allowlist(&env)
    }

    /// Marks `github_username` verified on the strength of an allowlisted
    /// oracle's signed proof instead of an admin/Verifier attestation.
    ///
    /// `proof.message` must be the canonical ownership message
    /// `<github_username>:<stellar_address>:<expires_at>` for the record's
    /// current `stellar_address` (see `docs/SECURITY.md`). Verification state,
    /// counters, the `VerifiedEvent`, and the audit row are updated exactly as
    /// `verify` does.
    ///
    /// # Auth
    ///
    /// Requires auth from the registered `stellar_address`.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotRegistered`] if `github_username` is not registered.
    /// - [`ContractError::OracleProofBadLayout`] if the proof is malformed or
    ///   its message does not bind this username/address/expiry.
    /// - [`ContractError::OracleProofNotAllowlisted`] if the signing key is not
    ///   allowlisted (always, when the allowlist is empty).
    /// - [`ContractError::OracleProofExpired`] if `expires_at` is non-zero and past.
    /// - [`ContractError::OracleProofBadSignature`] if the signature is not
    ///   canonically encoded; an invalid but well-formed signature traps.
    /// - [`ContractError::AlreadyVerified`] if the record is verified and that
    ///   verification has not expired.
    /// - [`ContractError::AddressMismatch`] if the proof is not bound to this
    ///   record's current `stellar_address`.
    /// - [`ContractError::ZeroAddress`] if the record's bound address is zero.
    pub fn verify_with_oracle_proof(
        env: Env,
        github_username: String,
        proof: OracleProof,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        let record = get_record(&env, &github_username).ok_or(ContractError::NotRegistered)?;
        require_bound_record_address(&env, &record)?;
        // Authenticated identity is the record's bound address — never a
        // caller-supplied substitute. A proof for a different address cannot
        // skip this check.
        record.stellar_address.require_auth();

        crate::oracle_proof::verify_ownership_proof(
            &env,
            &github_username,
            &record.stellar_address,
            &proof,
        )?;

        let actor = record.stellar_address.clone();
        mark_verified(&env, github_username, record, actor)
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
        crate::batch::batch_verify(env, caller, usernames)
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
        if !is_admin {
            require_role_not_expired(&env, &caller)?;
        }
        let is_revoker = storage_get_role(&env, &caller) == Some(Role::Revoker);
        if !is_admin && !is_revoker {
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "revoke_verification"),
            );
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

        // Revocation wrote no audit entry at all before Issue #397. The
        // contract event exists, but events are not retained on-chain — the
        // audit log is what a reader querying the contract can still see, and
        // losing verification is exactly the state change it should record.
        push_audit_entry(
            &env,
            AuditLogEntry::new(
                AuditEventType::VerificationRevoked,
                timestamp,
                Some(caller.clone()),
            )
            .with_username(github_username.clone())
            .with_address(record.stellar_address.clone()),
        );

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
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "set_bot_status"),
            );
            return Err(ContractError::NotAuthorized);
        }

        record.is_bot = is_bot;
        set_record(&env, &github_username, &record);

        BotStatusChangedEvent {
            github_username: github_username.clone(),
            is_bot,
            actor: caller,
            timestamp: env.ledger().timestamp(),
            domain: event_domain(&env),
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
}
