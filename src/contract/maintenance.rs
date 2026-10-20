//! Status helpers, reserved usernames and index maintenance entrypoints.
//!
//! Split out of `lib.rs` (Issue #601); see `docs/ARCHITECTURE.md#module-map`.

use crate::*;

#[contractimpl]
impl TrustBridgeContract {
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

    // ── Index repair (admin) ────────────────────────────────────────────────

    /// Recomputes `count` and `verified` from the chunked username index and
    /// each entry's stored record, independent of the counters themselves.
    /// Admin-only.
    ///
    /// Pass `apply = false` for a dry run: nothing is written, only the
    /// [`RepairReport`] comparing stored vs. recomputed values is returned.
    /// Pass `apply = true` to have any drift corrected in the same call — a
    /// call that finds no drift never writes, even with `apply = true`.
    /// This is never run automatically on any other entry point; see
    /// `docs/SECURITY.md#index-repair-repair_index` for when an operator
    /// should reach for it.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    pub fn repair_index(env: Env, apply: bool) -> Result<RepairReport, ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        Ok(crate::storage::repair_index(&env, apply))
    }
}
