//! Read-only stats and health entrypoints.
//!
//! Split out of `lib.rs` (Issue #601); see `docs/ARCHITECTURE.md#module-map`.

use crate::*;

#[contractimpl]
impl TrustBridgeContract {
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
}
