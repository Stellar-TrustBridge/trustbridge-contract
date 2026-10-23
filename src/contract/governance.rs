//! Configuration limits, versioning, upgrade attestation, staged WASM and multisig upgrades.
//!
//! Split out of `lib.rs` (Issue #601); see `docs/ARCHITECTURE.md#module-map`.

use crate::*;

#[contractimpl]
impl TrustBridgeContract {
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

    /// Sets the per-verifier, per-ledger cap on verify/revoke units (Issue #292).
    /// Admin-only.
    ///
    /// One `verify` or `revoke_verification` call spends one unit; one
    /// `batch_verify` spends one unit per requested username. When a non-admin
    /// caller would exceed `limit` units in a single ledger, the call fails with
    /// [`ContractError::VerifyRateLimited`] and writes nothing.
    ///
    /// `limit == 0` disables the check entirely. With no configured value the
    /// contract uses a built-in default (`DEFAULT_VERIFIES_PER_LEDGER`).
    ///
    /// The admin is never rate-limited — incident response must not be throttled.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the contract admin.
    pub fn set_verify_limit(env: Env, limit: u32) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        set_verify_limit(&env, limit);
        Ok(())
    }

    /// Returns the active per-verifier, per-ledger verify/revoke cap (Issue #292).
    ///
    /// This is the configured value when the admin has set one (including `0`,
    /// meaning disabled), otherwise the built-in default. Read-only; no auth.
    #[must_use]
    pub fn get_verify_limit(env: Env) -> u32 {
        get_verify_limit(&env)
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

    /// Enables or disables mandatory attestation before WASM upgrades (Issue #198).
    ///
    /// When `required` is `true`, every `upgrade` call must be preceded by a
    /// valid, unexpired `attest_upgrade`; without it the upgrade fails with
    /// [`ContractError::AttestationRequired`].  When `false` (the default),
    /// attestation is optional — the two-step flow is advisory only.
    ///
    /// Note: `clear_attestation` removes the pending attestation record but does
    /// **not** change this flag.  To disable the gate, call
    /// `set_attestation_required(false)` explicitly.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if `caller` is not the admin.
    pub fn set_attestation_required(
        env: Env,
        caller: Address,
        required: bool,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        caller.require_auth();

        let admin = get_admin(&env)?;
        if caller != admin {
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "set_attestation_required"),
            );
            return Err(ContractError::NotAuthorized);
        }

        crate::storage::set_attestation_required(&env, required);
        Ok(())
    }

    /// Returns whether WASM upgrade attestation is currently required.
    ///
    /// Defaults to `false` on instances that have never explicitly set the flag.
    /// Dashboards and operator tooling can read this to show the current gate
    /// configuration without needing admin credentials.
    #[must_use]
    pub fn get_attestation_required(env: Env) -> bool {
        crate::storage::is_attestation_required(&env)
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
                upgraded_by: admin.clone(),
                upgraded_at: now,
                version: soroban_sdk::vec![&env, version.0, version.1, version.2],
                attested,
                // Digests are recorded out of band by the operator after the
                // upgrade; `upgrade` itself has no way to know them (Issue #224).
                sbom_hash: None,
                source_hash: None,
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
            domain: event_domain(&env),
        }
        .publish(&env);

        // An upgrade replaces the code every other audit entry was produced
        // by, so it is the single most important entry in the log — and it
        // was not being written (Issue #397). Recorded after the WASM swap so
        // it lands in the storage the new binary reads.
        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::ContractUpgraded, now, Some(admin)),
        );

        Ok(())
    }

    /// Records the SBOM and source digests for the currently deployed WASM.
    /// Admin-only (Issue #224).
    ///
    /// Kept separate from `upgrade` because these digests are produced by the
    /// build pipeline, not by the chain. Passing `None` leaves the existing
    /// value untouched, so the two digests can be filled in independently. This
    /// is also the migration path for provenance records written before the
    /// fields existed — see `docs/DEPLOYMENT.md`.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if the caller is not the admin.
    /// - [`ContractError::ProvenanceMissing`] if no provenance record exists yet.
    pub fn set_provenance_digests(
        env: Env,
        sbom_hash: Option<BytesN<32>>,
        source_hash: Option<BytesN<32>>,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();

        let mut provenance = get_wasm_provenance(&env).ok_or(ContractError::ProvenanceMissing)?;
        if sbom_hash.is_some() {
            provenance.sbom_hash = sbom_hash;
        }
        if source_hash.is_some() {
            provenance.source_hash = source_hash;
        }
        set_wasm_provenance(&env, &provenance);
        Ok(())
    }

    /// Reproducible-build check: fails unless `hash` equals the WASM hash in
    /// stored provenance (Issue #225).
    ///
    /// An operator runs `stellar contract build`, takes the resulting hash, and
    /// calls this before authorising an upgrade. A read-only check by design —
    /// it does not consume or require an attestation, and does not touch state,
    /// so it is safe to call from a `Makefile` target or CI step.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::ProvenanceMissing`] if nothing has been deployed via
    ///   `upgrade` yet, so there is no recorded hash to compare against.
    /// - [`ContractError::ProvenanceMismatch`] if the hashes differ.
    pub fn assert_build(env: Env, hash: BytesN<32>) -> Result<(), ContractError> {
        require_initialized(&env)?;
        let provenance = get_wasm_provenance(&env).ok_or(ContractError::ProvenanceMissing)?;
        if provenance.wasm_hash != hash {
            return Err(ContractError::ProvenanceMismatch);
        }
        Ok(())
    }

    /// Enforces the upgrade timelock.
    ///
    /// Extracted from `upgrade` so the entry point reads as its four distinct
    /// steps — timelock, attestation, provenance, swap — instead of one block
    /// of interleaved policy.
    pub(crate) fn require_cooldown_elapsed(env: &Env, now: u64) -> Result<(), ContractError> {
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
    pub(crate) fn consume_attestation(
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

    // ── Staged WASM slot (Issue #300) ────────────────────────────────────────

    /// Stages a WASM hash for the next upgrade. Admin or Upgrader role required.
    ///
    /// The staged hash is publicly visible via `get_staged` so any observer
    /// knows what binary the operator intends to deploy before the upgrade
    /// transaction is submitted. When a staged hash is present, `upgrade` will
    /// only accept that exact hash; staging a different hash first replaces
    /// the slot.
    ///
    /// Staging does **not** make attestation mandatory — use
    /// `set_attestation_required(true)` for that. The two mechanisms are
    /// complementary.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin or any `Role::Upgrader` holder.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotAuthorized`] if `caller` is not admin or Upgrader.
    pub fn stage_wasm(
        env: Env,
        caller: Address,
        wasm_hash: BytesN<32>,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;
        caller.require_auth();

        let admin = get_admin(&env)?;
        if caller != admin {
            require_role_not_expired(&env, &caller)?;
        }
        let role = storage_get_role(&env, &caller);
        let is_upgrader = matches!(role, Some(crate::storage::Role::Upgrader));
        if caller != admin && !is_upgrader {
            Self::record_unauthorized_attempt(&env, &caller, String::from_str(&env, "stage_wasm"));
            return Err(ContractError::NotAuthorized);
        }

        let now = env.ledger().timestamp();
        let entry = crate::staged_wasm::StagedWasm {
            wasm_hash: wasm_hash.clone(),
            staged_by: caller.clone(),
            staged_at: now,
        };
        crate::staged_wasm::set_staged_wasm(&env, &entry);

        crate::events::WasmStagedEvent {
            wasm_hash,
            staged_by: caller,
            timestamp: now,
        }
        .publish(&env);

        Ok(())
    }

    /// Returns the currently staged WASM entry, or `None` if nothing is staged.
    ///
    /// Publicly readable — no auth required.
    #[must_use]
    pub fn get_staged(env: Env) -> Option<crate::staged_wasm::StagedWasm> {
        crate::staged_wasm::get_staged_wasm(&env)
    }

    /// Clears the staged WASM slot. Admin-only.
    ///
    /// Use this to retract a staging before a new binary is decided on. A
    /// no-op (not an error) if nothing is staged.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if `caller` is not the admin.
    pub fn clear_staged(env: Env, caller: Address) -> Result<(), ContractError> {
        require_initialized(&env)?;
        caller.require_auth();

        let admin = get_admin(&env)?;
        if caller != admin {
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "clear_staged"),
            );
            return Err(ContractError::NotAuthorized);
        }

        let now = env.ledger().timestamp();
        if let Some(entry) = crate::staged_wasm::get_staged_wasm(&env) {
            crate::staged_wasm::clear_staged_wasm(&env);
            crate::events::StagedWasmClearedEvent {
                wasm_hash: entry.wasm_hash,
                cleared_by: caller,
                timestamp: now,
            }
            .publish(&env);
        }

        Ok(())
    }

    // ── Multisig upgrade flow (Issue #301) ───────────────────────────────────

    /// Sets the number of distinct approvals required before an upgrade
    /// proposal can be executed. Default `1` (single-admin, backward
    /// compatible). Set to `2` or higher for M-of-N governance.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if caller is not the admin.
    pub fn set_upgrade_threshold(
        env: Env,
        caller: Address,
        threshold: u32,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        caller.require_auth();

        let admin = get_admin(&env)?;
        if caller != admin {
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "set_upgrade_threshold"),
            );
            return Err(ContractError::NotAuthorized);
        }

        crate::multisig_upgrade::set_upgrade_threshold(&env, threshold);

        let timestamp = env.ledger().timestamp();
        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::AdminAction, timestamp, Some(caller))
                .with_details(soroban_sdk::String::from_str(&env, "set_upgrade_threshold")),
        );

        Ok(())
    }

    /// Returns the current upgrade approval threshold.
    #[must_use]
    pub fn get_upgrade_threshold(env: Env) -> u32 {
        crate::multisig_upgrade::get_upgrade_threshold(&env)
    }

    /// Proposes a multisig WASM upgrade.
    ///
    /// The proposer is recorded as the first approver. Once `N` distinct
    /// addresses have approved (where `N = get_upgrade_threshold()`) and
    /// `delay_secs` have elapsed, any admin or Upgrader can call
    /// `execute_upgrade` to perform the swap.
    ///
    /// Only one proposal may be live at a time — call `cancel_upgrade_proposal`
    /// to discard an existing one before proposing a new one.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin or any `Role::Upgrader` holder.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if `caller` is not admin or Upgrader.
    /// - [`ContractError::UpgradeProposalAlreadyPending`] if a proposal exists.
    pub fn propose_multisig_upgrade(
        env: Env,
        caller: Address,
        wasm_hash: BytesN<32>,
        delay_secs: u64,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        caller.require_auth();

        let admin = get_admin(&env)?;
        if caller != admin {
            require_role_not_expired(&env, &caller)?;
        }
        let role = storage_get_role(&env, &caller);
        let is_upgrader = matches!(role, Some(crate::storage::Role::Upgrader));
        if caller != admin && !is_upgrader {
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "propose_multisig_upgrade"),
            );
            return Err(ContractError::NotAuthorized);
        }

        let proposal = crate::multisig_upgrade::create_upgrade_proposal(
            &env,
            caller.clone(),
            wasm_hash.clone(),
            delay_secs,
        )?;

        crate::events::UpgradeProposedEvent {
            proposal_id: proposal.id,
            wasm_hash,
            proposed_by: caller.clone(),
            executable_at: proposal.executable_at,
            timestamp: proposal.proposed_at,
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(
                AuditEventType::AdminAction,
                proposal.proposed_at,
                Some(caller),
            )
            .with_details(soroban_sdk::String::from_str(
                &env,
                "propose_multisig_upgrade",
            )),
        );

        Ok(())
    }

    /// Adds `caller`'s approval to the live upgrade proposal.
    ///
    /// The caller must be the admin or hold `Role::Upgrader`, and must not
    /// have already approved this proposal.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin or any `Role::Upgrader` holder.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if `caller` is not admin or Upgrader.
    /// - [`ContractError::NoUpgradeProposalPending`] if no proposal is live.
    /// - [`ContractError::UpgradeProposalAlreadyApproved`] if already approved.
    pub fn approve_upgrade(
        env: Env,
        caller: Address,
        proposal_id: u32,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        caller.require_auth();

        let admin = get_admin(&env)?;
        if caller != admin {
            require_role_not_expired(&env, &caller)?;
        }
        let role = storage_get_role(&env, &caller);
        let is_upgrader = matches!(role, Some(crate::storage::Role::Upgrader));
        if caller != admin && !is_upgrader {
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "approve_upgrade"),
            );
            return Err(ContractError::NotAuthorized);
        }

        let proposal = crate::multisig_upgrade::record_approval(&env, proposal_id, caller.clone())?;

        crate::events::UpgradeApprovedEvent {
            proposal_id,
            approved_by: caller.clone(),
            approval_count: proposal.approvers.len(),
            timestamp: env.ledger().timestamp(),
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(
                AuditEventType::AdminAction,
                env.ledger().timestamp(),
                Some(caller),
            )
            .with_details(soroban_sdk::String::from_str(&env, "approve_upgrade")),
        );

        Ok(())
    }

    /// Executes the live upgrade proposal if the delay has elapsed and the
    /// approval threshold has been met.
    ///
    /// On success the staged WASM slot is also consumed (cleared) if it matches,
    /// and provenance is recorded exactly as the regular `upgrade` path does.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin or any `Role::Upgrader` holder.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::Paused`] if the contract is paused.
    /// - [`ContractError::NotAuthorized`] if `caller` is not admin or Upgrader.
    /// - [`ContractError::NoUpgradeProposalPending`] if no proposal exists.
    /// - [`ContractError::UpgradeProposalDelayActive`] if delay has not elapsed.
    /// - [`ContractError::UpgradeProposalInsufficientApprovals`] if threshold not met.
    /// - [`ContractError::StagedWasmMismatch`] if a staged hash conflicts.
    pub fn execute_upgrade(
        env: Env,
        caller: Address,
        proposal_id: u32,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        require_not_paused(&env)?;
        caller.require_auth();

        let admin = get_admin(&env)?;
        if caller != admin {
            require_role_not_expired(&env, &caller)?;
        }
        let role = storage_get_role(&env, &caller);
        let is_upgrader = matches!(role, Some(crate::storage::Role::Upgrader));
        if caller != admin && !is_upgrader {
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "execute_upgrade"),
            );
            return Err(ContractError::NotAuthorized);
        }

        let proposal = crate::multisig_upgrade::require_proposal_executable(&env, proposal_id)?;

        // If a staged WASM slot is set, it must match the proposal.
        crate::staged_wasm::require_staged_wasm_consistent(&env, &proposal.wasm_hash)?;

        let now = env.ledger().timestamp();

        // Capture provenance before the swap (same policy as `upgrade`).
        let previous_wasm_hash = get_wasm_provenance(&env).map(|p| p.wasm_hash);
        let version = storage_get_version(&env).unwrap_or_else(|| CONTRACT_VERSION.to_tuple());

        set_wasm_provenance(
            &env,
            &WasmProvenance {
                wasm_hash: proposal.wasm_hash.clone(),
                previous_wasm_hash,
                upgraded_by: caller.clone(),
                upgraded_at: now,
                version: soroban_sdk::vec![&env, version.0, version.1, version.2],
                attested: false, // multisig path; attestation is orthogonal
                // Same policy as the single-admin `upgrade` path: the chain has
                // no way to know either digest at upgrade time, so the operator
                // records them afterwards with `set_provenance_digests`
                // (Issue #224). Omitting the fields here did not default them —
                // it failed to compile (Issue #400).
                sbom_hash: None,
                source_hash: None,
            },
        );

        // Consume the proposal.
        let approval_count = proposal.approvers.len();
        crate::multisig_upgrade::clear_upgrade_proposal(&env);

        // Clear the staged slot if present (no error if absent).
        crate::staged_wasm::clear_staged_wasm(&env);

        env.deployer()
            .update_current_contract_wasm(proposal.wasm_hash.clone());
        set_last_upgrade(&env, now);

        let version = storage_get_version(&env).unwrap_or_else(|| CONTRACT_VERSION.to_tuple());
        crate::events::UpgradeProposalExecutedEvent {
            proposal_id,
            wasm_hash: proposal.wasm_hash.clone(),
            executed_by: caller.clone(),
            approval_count,
            timestamp: now,
        }
        .publish(&env);

        UpgradedEvent {
            new_wasm_hash: proposal.wasm_hash,
            version,
            timestamp: now,
            domain: event_domain(&env),
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(AuditEventType::ContractUpgraded, now, Some(caller)),
        );

        Ok(())
    }

    /// Cancels the live upgrade proposal. Admin-only.
    ///
    /// Available at any time before execution, including while the contract is
    /// paused.
    ///
    /// # Auth
    ///
    /// Requires auth from the contract admin.
    ///
    /// # Errors
    ///
    /// - [`ContractError::NotInitialized`] if `initialize` has not been called.
    /// - [`ContractError::NotAuthorized`] if `caller` is not the admin.
    /// - [`ContractError::NoUpgradeProposalPending`] if no proposal is live.
    pub fn cancel_upgrade_proposal(
        env: Env,
        caller: Address,
        proposal_id: u32,
    ) -> Result<(), ContractError> {
        require_initialized(&env)?;
        caller.require_auth();

        let admin = get_admin(&env)?;
        if caller != admin {
            Self::record_unauthorized_attempt(
                &env,
                &caller,
                String::from_str(&env, "cancel_upgrade_proposal"),
            );
            return Err(ContractError::NotAuthorized);
        }

        let proposal = crate::multisig_upgrade::get_upgrade_proposal(&env)
            .ok_or(ContractError::NoUpgradeProposalPending)?;
        if proposal.id != proposal_id {
            return Err(ContractError::NoUpgradeProposalPending);
        }

        crate::multisig_upgrade::clear_upgrade_proposal(&env);

        crate::events::UpgradeProposalCancelledEvent {
            proposal_id,
            wasm_hash: proposal.wasm_hash,
            cancelled_by: caller.clone(),
            timestamp: env.ledger().timestamp(),
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(
                AuditEventType::AdminAction,
                env.ledger().timestamp(),
                Some(caller),
            )
            .with_details(soroban_sdk::String::from_str(
                &env,
                "cancel_upgrade_proposal",
            )),
        );

        Ok(())
    }

    /// Returns the live upgrade proposal, if any.
    #[must_use]
    pub fn get_upgrade_proposal(env: Env) -> Option<crate::multisig_upgrade::UpgradeProposal> {
        crate::multisig_upgrade::get_upgrade_proposal(&env)
    }

    /// Authorization check for `remove`: only the registrant or the contract
    /// admin may remove a record.
    pub(crate) fn require_remove_auth(
        caller: &Address,
        admin: &Address,
        record_address: &Address,
    ) -> Result<(), ContractError> {
        if caller != admin && caller != record_address {
            return Err(ContractError::NotAuthorized);
        }
        Ok(())
    }
}
