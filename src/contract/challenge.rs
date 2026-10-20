//! Challenge-period entrypoints.
//!
//! Split out of `lib.rs` (Issue #601); see `docs/ARCHITECTURE.md#module-map`.

use crate::*;

#[contractimpl]
impl TrustBridgeContract {
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
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "start_challenge"),
            );
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
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "cancel_challenge"),
            );
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
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "complete_challenge"),
            );
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
}
