//! Role-based access control, network tagging and the verifier allowlist.
//!
//! Split out of `lib.rs` (Issue #601); see `docs/ARCHITECTURE.md#module-map`.

use crate::*;

#[contractimpl]
impl TrustBridgeContract {
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

        let timestamp = env.ledger().timestamp();
        let delay = storage_get_role_delay(&env);
        if delay > 0 {
            // Timelocked path (Issue #220): the grant is only queued here, so a
            // stolen admin key cannot mint a Verifier/Upgrader and use it in the
            // same transaction — the window is observable and cancellable.
            let activate_at = timestamp.saturating_add(delay);
            set_pending_role(&env, &target, &PendingRoleGrantRecord { role, activate_at });
            RoleGrantPendingEvent {
                address: target,
                role: role as u32,
                admin,
                activate_at,
                timestamp,
            }
            .publish(&env);
            return Ok(());
        }

        storage_set_role(&env, &target, &role);
        RoleGrantedEvent {
            address: target.clone(),
            role: role as u32,
            admin: admin.clone(),
            timestamp,
            domain: event_domain(&env),
        }
        .publish(&env);

        // Who holds which role is the first thing an incident review asks.
        // It was reconstructable only from events, which are not retained
        // on-chain (Issue #397).
        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::RoleChanged, timestamp, Some(admin))
                .with_address(target),
        );

        Ok(())
    }

    /// Sets the timelock applied to future `set_role` grants, in seconds.
    /// Admin-only. `0` restores instant grants.
    ///
    /// The admin address itself is stored separately and is unaffected — the
    /// admin never has to wait out its own timelock to act (Issue #220).
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the admin.
    pub fn set_role_delay(env: Env, secs: u64) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();
        storage_set_role_delay(&env, secs);
        Ok(())
    }

    /// Current `set_role` timelock in seconds. `0` == grants take effect immediately.
    #[must_use]
    pub fn get_role_delay(env: Env) -> u64 {
        storage_get_role_delay(&env)
    }

    /// The pending (not yet active) role grant for `target`, if any.
    #[must_use]
    pub fn get_pending_role(env: Env, target: Address) -> Option<PendingRoleGrant> {
        storage_get_pending_role(&env, &target)
    }

    /// Applies a pending role grant once its timelock has elapsed. Admin-only.
    ///
    /// Emits [`RoleGrantedEvent`] — the same event an instant grant emits, so
    /// indexers do not need a second code path for the activated role.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotAuthorized`] if the caller is not the admin.
    /// - [`ContractError::NoPendingRoleGrant`] if `target` has no pending grant.
    /// - [`ContractError::RoleGrantNotReady`] if the timelock has not elapsed.
    pub fn activate_role(env: Env, target: Address) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;

        let admin = get_admin(&env)?;
        admin.require_auth();

        let pending =
            storage_get_pending_role(&env, &target).ok_or(ContractError::NoPendingRoleGrant)?;
        let timestamp = env.ledger().timestamp();
        if timestamp < pending.activate_at {
            return Err(ContractError::RoleGrantNotReady);
        }

        storage_set_role(&env, &target, &pending.role);
        remove_pending_role(&env, &target);
        RoleGrantedEvent {
            address: target.clone(),
            role: pending.role as u32,
            admin: admin.clone(),
            timestamp,
            domain: event_domain(&env),
        }
        .publish(&env);

        // Activation, not the earlier `set_role`, is when the grant takes
        // effect — so it is the entry that answers "when did this address
        // gain the role" (Issue #397).
        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::RoleChanged, timestamp, Some(admin))
                .with_address(target),
        );

        Ok(())
    }

    /// Drops a pending role grant before it activates. Admin-only.
    ///
    /// Emits [`RoleGrantCancelledEvent`].
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the admin.
    /// - [`ContractError::NoPendingRoleGrant`] if `target` has no pending grant.
    pub fn cancel_role_grant(env: Env, target: Address) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        if storage_get_pending_role(&env, &target).is_none() {
            return Err(ContractError::NoPendingRoleGrant);
        }
        remove_pending_role(&env, &target);
        RoleGrantCancelledEvent {
            address: target,
            admin,
            timestamp: env.ledger().timestamp(),
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
        // A revoke also drops any grant still inside its timelock (Issue #220),
        // otherwise the pending grant would resurrect the role on activation.
        remove_pending_role(&env, &target);
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
    /// - [`ContractError::RoleGrantNotReady`] if a `set_role_delay` timelock is
    ///   armed and `target` does not already actively hold `role`: new grants
    ///   must go through `set_role` + `activate_role` (Issue #220).
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

        // With the grant timelock armed (Issue #220), this path must not mint
        // a role instantly. It may still change the expiry of a grant the
        // target already actively holds (e.g. to off-board a key early).
        if storage_get_role_delay(&env) > 0 && storage_get_role(&env, &target) != Some(role) {
            return Err(ContractError::RoleGrantNotReady);
        }

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
    pub fn add_verifier(env: Env, verifier: Address, expires_at: u64) -> Result<(), ContractError> {
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
}
