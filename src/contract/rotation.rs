//! Username rename and delayed address rotation entrypoints.
//!
//! Split out of `lib.rs` (Issue #601); see `docs/ARCHITECTURE.md#module-map`.

use crate::*;

#[contractimpl]
impl TrustBridgeContract {
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
        if let Err(e) = Self::require_remove_auth(&caller, &admin, &record.stellar_address) {
            Self::record_unauthorized_attempt(&env, &caller, String::from_str(&env, "rename"));
            return Err(e);
        }

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
        // Read before `remove_record` below drops the old name's flag.
        let reverify_pending = get_pending_reverify(&env, &old_username);

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

        // An outstanding re-verify flag moves with the registration.
        if verification_cleared || reverify_pending {
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
                AuditEventType::UserRenamed,
                timestamp,
                Some(record.stellar_address),
            )
            // The new name alone would leave no way to find the entry from the
            // name the record used to have, which is what an auditor
            // investigating the old username searches for.
            .with_username(new_username.clone())
            .with_details(old_username.clone()),
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
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "cancel_address_rotation"),
            );
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
}
