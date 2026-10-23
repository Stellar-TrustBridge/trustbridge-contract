//! Migration, registration, removal, lookup and export entrypoints.
//!
//! Split out of `lib.rs` (Issue #601); see `docs/ARCHITECTURE.md#module-map`.

use crate::*;

#[contractimpl]
impl TrustBridgeContract {
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
            domain: event_domain(&env),
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
            domain: event_domain(&env),
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
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "batch_remove"),
            );
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
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "propose_batch_remove"),
            );
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
        let admin = get_admin(&env)?;
        if caller != admin {
            require_role_not_expired(&env, &caller)?;
        }

        let proposal = crate::storage::get_pending_batch_remove(&env)
            .ok_or(ContractError::NoPendingBatchRemove)?;

        if crate::storage::is_batch_remove_proposal_expired(&env, &proposal) {
            crate::storage::clear_pending_batch_remove(&env);
            return Err(ContractError::NoPendingBatchRemove);
        }

        let is_admin_equivalent =
            caller == admin || storage_get_role(&env, &caller) == Some(Role::Admin);
        if !is_admin_equivalent || caller == proposal.proposed_by {
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "execute_batch_remove"),
            );
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
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "cancel_batch_remove"),
            );
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
    pub(crate) fn apply_batch_remove(
        env: &Env,
        caller: &Address,
        usernames: &Vec<String>,
    ) -> BatchSummary {
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
        if let Err(e) = Self::require_remove_auth(&caller, &admin, &record.stellar_address) {
            Self::record_unauthorized_attempt(&env, &caller, String::from_str(&env, "remove"));
            return Err(e);
        }

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

    /// Returns a page of GitHub usernames flagged for re-verification after an
    /// address change (Issue #587).
    ///
    /// Scans registry index entries `[offset, offset + limit)` and returns the
    /// canonical usernames whose `PENDING_REVERIFY_KEY` flag is set. Gas is
    /// bounded by `limit`, so a page can hold fewer than `limit` names; keep
    /// advancing `offset` by `limit` until it reaches `get_stats().total`.
    /// `limit == 0` uses `DEFAULT_PAGE_LIMIT`. An empty registry returns an
    /// empty page. Read-only, no auth required.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::InvalidBatchSize`] if `limit > MAX_PAGE_LIMIT` (100).
    pub fn get_pending_reverify(
        env: Env,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<String>, ContractError> {
        require_initialized(&env)?;
        crate::storage::get_pending_reverify_page(&env, offset, limit)
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
        cursor: Option<BytesN<8>>,
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
    /// **Persistent-chunk reads only (dual-index audit).** This reads from
    /// [`get_public_paginated_internal`], which walks the persistent chunked
    /// index directly, rather than [`get_registered_paginated_internal`],
    /// which loads the whole legacy flat index out of instance storage on
    /// every call. Being unauthenticated and pause-exempt, this is the path
    /// most exposed to that cost, so it must never fall back to the flat-index
    /// scan. See the dual-index audit note in `docs/DASHBOARD_SYNC.md`.
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

        get_public_paginated_internal(&env, cursor, limit)
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
            AuditLogEntry::new(
                // Direction matters more than the fact an admin acted: an
                // auditor reading "ADMIN_ACTION" has to correlate with the
                // previous entry to learn whether the contract went down or
                // came back up.
                if paused {
                    AuditEventType::ContractPaused
                } else {
                    AuditEventType::ContractUnpaused
                },
                timestamp,
                Some(admin),
            ),
        );

        Ok(())
    }
}
