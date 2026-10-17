# Changelog

All notable public ABI changes are recorded here. Entries use the contract
version exposed by `version()` and follow semantic versioning: major for
breaking changes, minor for additive interface changes, and patch for
compatible corrections.

## [1.2.0] - Unreleased

### Added

- Added `get_address_if_verified` to the public ABI for secure CI payouts (see [get_address_if_verified](docs/ABI.md#get_address_if_verifiedgithub_username-string---resultcontributorrecord-contracterror)).
- Added verifier allowlist management (`add_verifier`, `remove_verifier`, `get_verifiers`, `is_active_verifier`, `verifier_slots_remaining`, `prune_expired_verifiers`).
- Added role-grant timelock lifecycle functions (`activate_role`, `cancel_role_grant`).
- Added oracle-proof verification: `set_oracle_allowlist`, `get_oracle_allowlist`, and `verify_with_oracle_proof` (see [verify_with_oracle_proof](docs/ABI.md#verify_with_oracle_proofgithub_username-string-proof-oracleproof---result-contracterror)).
- Added typed export page bindings (`ExportRecord`, `ExportPage`, `EXPORT_PAGE_LAYOUT_VERSION = 2`) and updated `abi/export_page.layout.golden` layout regression fixtures.
- Added TypeScript export page decoding and typed parser helpers in `ts-differential-tests`.
- Added `set_audit_config(caller, enabled, max_events, log_unauthorized)` — admin-gated entry point to persist `AuditConfig` on-chain. Emits `AuditConfigChangedEvent` and appends an `AuditConfigChanged` (code 14) log entry (Issue #412).
- Added `get_audit_config()` — public read of the stored `AuditConfig`, falling back to defaults when none has been set (Issue #412).
- Added `AuditConfigChangedEvent` with topic `admin` and body fields `enabled`, `max_events`, `log_unauthorized`, `timestamp` (Issue #412).
- Added `UnauthorizedAttempt` (code 6) audit log entries: every public entry point that returns `NotAuthorized` now calls `record_unauthorized_attempt` before returning, gated on `AuditConfig.log_unauthorized`. Actors and operation names are recorded for forensics (Issue #412).
- `AuditStats.unauthorized_attempts` counter now increments on every failed auth regardless of `log_unauthorized` flag.

### Changed

- Documented the `set_bot_status` auth policy consistently as admin-or-registrant across `tests/auth_matrix.csv`, `docs/ABI.md` and `docs/ADMIN_RUNBOOK.md` (the runbook previously said admin only). No contract behaviour change.
- Fixed `calculate_verification_percentage` and `BatchSummary::new` rounding at boundary and fractional values (0%, 100%, 1/3, 2/3) to use standard round-half-up semantics.
- Fixed `NetworkMismatch` error code by moving its discriminant from 21 to 30 to avoid overlap with `InvalidPauseReason` (see [ContractError](docs/ABI.md#contracterror-u32-discriminant)).
- Added new error codes `VerifierAllowlistFull` (31), `VerifierNotAllowlisted` (32), `VerifierExpiryInPast` (33), `NoPendingRoleGrant` (34), `RoleGrantNotReady` (35), `ProvenanceMissing` (36), and `ProvenanceMismatch` (37).
- **`docs/ABI.md` AuditConfig section**: corrected field list to match `src/audit.rs` exactly. Previous docs listed non-existent fields (`retention_epochs`, `max_events_per_epoch`, `require_actor`); actual fields are `enabled`, `max_events`, `log_unauthorized`. Added `AuditEventType` code table and entry-point reference (Issue #412).
- **`scripts/event_indexer.sh`**: added `audit_config_changed_event|audit_config_changed|audit` row to `TOPIC_TABLE` so the indexer classifies the new event instead of labelling it `unknown`.

## [1.1.0] - 2026-09-26

### Added

- Added `batch_verify` to the public contract interface. Clients should gate
    this call on `is_compatible(1, 1, 0)` when older deployments are supported.
- New instances now initialize and report contract version `1.1.0`.

## [1.0.0] - 2026-08-28

### ABI snapshot

- Initial documented TrustBridge Contract ABI, including registry operations,
  verification and revocation, pagination, role management, upgrade
  attestation, admin transfer, challenge-period, health, and network-tagging
  interfaces.
- Initial event and public type reference captured in [docs/ABI.md](docs/ABI.md).

<!-- changelog-check: skip - Added get_verification_config() docs entry and VerificationConfiguredEvent to docs/ABI.md; both were already implemented under the 1.0.0 ABI (config observability follow-up), no version bump. -->

