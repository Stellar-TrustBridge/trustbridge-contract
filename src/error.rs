use soroban_sdk::contracterror;

/// Errors returned by contract entry points.
///
/// Each variant maps to a stable `u32` code (see `code()` / `from_code()`).
/// Off-chain consumers such as the dashboard and indexer use these codes to
/// classify failed invocations without depending on the Rust enum layout.
///
/// # Append-only policy
///
/// **Every** code is frozen in `abi/contract_error_codes.golden`, not just the
/// first sixteen (Issue #394). `tests/contract_error_codes.rs` enforces the
/// full set in both directions: each recorded code still maps to its variant,
/// and each variant the enum defines has a record.
///
/// The rules, in order of how much damage breaking them does:
///
/// 1. **Never renumber or reorder an existing code.** Those numbers are read
///    out of failed invocations' XDR results and stored by off-chain
///    consumers. Renumbering silently re-labels every failure already
///    recorded against the old number — including ones in an indexer's
///    history that nobody will think to re-check.
/// 2. **Never reuse a gap.** Code 30 is unused, and is recorded as reserved
///    rather than filled. Handing it to a new error would make 30 mean one
///    thing in this build and nothing in every earlier one, which is the same
///    ambiguity renumbering causes.
/// 3. **Append the next unused code**, and add the matching golden entry in
///    the same change. A variant without a golden entry is unfrozen, and an
///    unfrozen code is the one a later refactor renumbers freely.
/// 4. **Removing a code is a breaking ABI change.** A consumer that still
///    resolves the old number gets `None` and no explanation. It requires a
///    major ABI version, as renumbering does.
///
/// This table is the single source of truth for the doc side of the mapping and
/// is checked against the enum and against `abi/contract_error_codes.golden` by
/// `scripts/check_error_codes.sh` (Issue #402). Before that check existed the
/// table had drifted four codes out of step with the enum from code 17 onward,
/// which would have made an off-chain consumer decode `InvalidPauseReason` as
/// `ChallengeAlreadyActive`.
///
/// | Code | Variant | Raised by |
/// |------|---------|-----------|
/// | 1 | `AlreadyInitialized` | `initialize` |
/// | 2 | `NotInitialized` | any function called before `initialize` |
/// | 3 | `NotAuthorized` | `remove`, `verify`, `revoke_verification`, role functions |
/// | 4 | `NotRegistered` | `remove`, `verify`, `revoke_verification` |
/// | 5 | `AlreadyVerified` | `verify` |
/// | 6 | `NotVerified` | `revoke_verification` |
/// | 7 | `Paused` | any state-mutating call while paused |
/// | 8 | `CooldownActive` | `upgrade`, `register` |
/// | 9 | `InvalidVersion` | `migrate` |
/// | 10 | `InvalidRole` | `set_role` |
/// | 11 | `InvalidUsername` | `register` |
/// | 12 | `AttestationExpired` | `attest_upgrade`, `upgrade` |
/// | 13 | `UnattestedWasm` | `upgrade` |
/// | 14 | `InvalidBatchSize` | `batch_verify`, `batch_remove` |
/// | 15 | `InvalidReasonCode` | `revoke_verification` |
/// | 16 | `ZeroAddress` | `register` |
/// | 17 | `ChallengeAlreadyActive` | `open_challenge` |
/// | 18 | `NoChallengeActive` | `resolve_challenge`, `cancel_challenge` |
/// | 19 | `ChallengeNotResolvable` | `resolve_challenge` |
/// | 20 | `ChallengeActive` | calls blocked while a challenge is open |
/// | 21 | `InvalidPauseReason` | `pause`, `unpause`, `set_paused` |
/// | 22 | `AlreadyReserved` | `add_reserved` |
/// | 23 | `NotReserved` | `remove_reserved` |
/// | 24 | `UsernameReserved` | `register` |
/// | 25 | `ReservedListFull` | `add_reserved` |
/// | 26 | `AdminTransferPending` | `transfer_admin` |
/// | 27 | `AdminTransferDelayActive` | `accept_admin` |
/// | 28 | `NoPendingAdminTransfer` | `accept_admin`, `cancel_admin_transfer` |
/// | 29 | `AttestationRequired` | `upgrade` |
/// | 30 | — | *reserved, never assigned* |
/// | 31 | `VerifierAllowlistFull` | `add_verifier` |
/// | 33 | `VerifierExpiryInPast` | `add_verifier` |
/// | 34 | `NoPendingRoleGrant` | `activate_role`, `cancel_role_grant` |
/// | 35 | `RoleGrantNotReady` | `activate_role` |
/// | 36 | `ProvenanceMissing` | `assert_build` |
/// | 37 | `ProvenanceMismatch` | `assert_build` |
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    /// `initialize` was called more than once.
    AlreadyInitialized = 1,
    /// A function was called before `initialize`.
    NotInitialized = 2,
    /// The caller does not have the required role or does not own the resource.
    NotAuthorized = 3,
    /// The referenced `github_username` is not registered.
    NotRegistered = 4,
    /// `verify` was called on a username that is already verified.
    AlreadyVerified = 5,
    /// `revoke_verification` was called on a username that is not verified.
    NotVerified = 6,
    /// A state-mutating function was called while the contract is paused.
    Paused = 7,
    /// `upgrade` was called before the cooldown period elapsed.
    CooldownActive = 8,
    /// `migrate` was called with a version that is not strictly greater than the current one.
    InvalidVersion = 9,
    /// `set_role` was called with an unrecognised role discriminant.
    InvalidRole = 10,
    /// The supplied GitHub username is empty, longer than
    /// `utils::MAX_USERNAME_LEN`, or contains characters GitHub does not allow.
    InvalidUsername = 11,
    /// `attest_upgrade` was called with an `expires_at` not in the future, or a
    /// live attestation lapsed before `upgrade` consumed it.
    AttestationExpired = 12,
    /// `upgrade` was called with a WASM hash that does not match the live
    /// attestation.
    UnattestedWasm = 13,
    /// A batch call (e.g. `extend_registry_ttl`) was given zero or more items
    /// than `batch::BatchConfig::max_batch_size` allows.
    InvalidBatchSize = 14,
    /// `revoke_verification` was called with an unrecognized reason code.
    InvalidReasonCode = 15,
    /// The supplied Stellar address is the well-known zero/burn address.
    ZeroAddress = 16,
    /// A challenge is already active for this username (Issue #214).
    ChallengeAlreadyActive = 17,
    /// No challenge is active for this username (Issue #214).
    NoChallengeActive = 18,
    /// The challenge delay has not elapsed yet (Issue #214).
    ChallengeNotResolvable = 19,
    /// Operation is blocked because a challenge is active on this username
    /// (Issue #214).
    ChallengeActive = 20,
    /// `pause` / `unpause` / `set_paused` were called with an unrecognized reason code.
    InvalidPauseReason = 21,
    /// `add_reserved` was called with a username that is already reserved.
    AlreadyReserved = 22,
    /// `remove_reserved` was called with a username that is not reserved.
    NotReserved = 23,
    /// `register` was called with a username on the reserved list.
    UsernameReserved = 24,
    /// The reserved list has reached its maximum allowed size.
    ReservedListFull = 25,
    /// `propose_admin_transfer` was called while a transfer is already pending,
    /// or `execute_admin_transfer` was called with no pending transfer.
    AdminTransferPending = 26,
    /// `execute_admin_transfer` was called before the delay has elapsed.
    AdminTransferDelayActive = 27,
    /// `execute_admin_transfer` was called with no pending transfer proposal.
    NoPendingAdminTransfer = 28,
    /// `upgrade` was called without a required attestation (attestation-required mode is on).
    AttestationRequired = 29,
    /// A gated call was made on instance state whose recorded network id does
    /// not match the network executing it (Issue #231 / #401).
    ///
    /// Raised by `storage::require_matching_network`, directly in `initialize`
    /// and through `require_initialized` in later guarded entry points.
    /// State restored onto the wrong network is the case this catches — a
    /// testnet snapshot replayed against mainnet, or the reverse.
    NetworkMismatch = 30,
    /// `add_verifier` was called when the verifier allowlist is already at its
    /// maximum size.
    VerifierAllowlistFull = 31,
    /// `add_verifier` was called with an address that is not on the verifier
    /// allowlist.
    VerifierNotAllowlisted = 32,
    /// `add_verifier` was called with an `expires_at` that is not in the future.
    VerifierExpiryInPast = 33,
    /// `activate_role` / `cancel_role_grant` was called with no pending role grant.
    NoPendingRoleGrant = 34,
    /// `activate_role` was called before the role grant delay elapsed.
    RoleGrantNotReady = 35,
    /// `assert_build` was called without provenance metadata.
    ProvenanceMissing = 36,
    /// `assert_build` was called with provenance metadata that does not match.
    ProvenanceMismatch = 37,
}

/// Coarse classification of a [`ContractError`] for off-chain retry policy.
///
/// Off-chain consumers (indexer, dashboard, retry workers) use this to decide
/// whether a failed invocation is worth retrying, requires operator action, or
/// is terminal. The mapping must stay exhaustive: a new `ContractError`
/// variant that is not classified here is a compile error (E0004), which is
/// deliberate — it forces the retry policy to be reconsidered whenever the
/// error surface grows.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ErrorCategory {
    /// The caller is not permitted to perform the operation. Retrying with the
    /// same credentials will not help; the caller must be authorized first.
    Auth,
    /// A transient condition (cooldown, delay, contention) that may clear on
    /// its own. Safe to retry after a backoff.
    Retry,
    /// A terminal condition that will not resolve by retrying. Requires
    /// operator or caller intervention.
    Fatal,
}

impl ContractError {
    /// Classify this error for off-chain retry policy.
    ///
    /// The match is intentionally exhaustive with no wildcard arm so that
    /// adding a variant without classifying it fails to compile (E0004).
    pub fn category(&self) -> ErrorCategory {
        match self {
            // Authorization failures: the caller lacks the required role,
            // ownership, or allowlist membership. Retrying unchanged is futile.
            ContractError::NotAuthorized => ErrorCategory::Auth,
            ContractError::InvalidRole => ErrorCategory::Auth,
            ContractError::VerifierNotAllowlisted => ErrorCategory::Auth,
            ContractError::VerifierExpiryInPast => ErrorCategory::Auth,

            // Transient conditions that may clear without intervention.
            ContractError::CooldownActive => ErrorCategory::Retry,
            ContractError::AdminTransferDelayActive => ErrorCategory::Retry,
            ContractError::ChallengeNotResolvable => ErrorCategory::Retry,
            ContractError::RoleGrantNotReady => ErrorCategory::Retry,

            // Operator/config conditions: the allowlist is full, which is a
            // capacity limit an operator must raise, not a caller mistake.
            ContractError::VerifierAllowlistFull => ErrorCategory::Fatal,

            // Everything else is terminal for the caller as-is.
            ContractError::AlreadyInitialized => ErrorCategory::Fatal,
            ContractError::NotInitialized => ErrorCategory::Fatal,
            ContractError::NotRegistered => ErrorCategory::Fatal,
            ContractError::AlreadyVerified => ErrorCategory::Fatal,
            ContractError::NotVerified => ErrorCategory::Fatal,
            ContractError::Paused => ErrorCategory::Fatal,
            ContractError::InvalidVersion => ErrorCategory::Fatal,
            ContractError::InvalidUsername => ErrorCategory::Fatal,
            ContractError::AttestationExpired => ErrorCategory::Fatal,
            ContractError::UnattestedWasm => ErrorCategory::Fatal,
            ContractError::InvalidBatchSize => ErrorCategory::Fatal,
            ContractError::InvalidReasonCode => ErrorCategory::Fatal,
            ContractError::ZeroAddress => ErrorCategory::Fatal,
            ContractError::ChallengeAlreadyActive => ErrorCategory::Fatal,
            ContractError::NoChallengeActive => ErrorCategory::Fatal,
            ContractError::ChallengeActive => ErrorCategory::Fatal,
            ContractError::InvalidPauseReason => ErrorCategory::Fatal,
            ContractError::AlreadyReserved => ErrorCategory::Fatal,
            ContractError::NotReserved => ErrorCategory::Fatal,
            ContractError::UsernameReserved => ErrorCategory::Fatal,
            ContractError::ReservedListFull => ErrorCategory::Fatal,
            ContractError::AdminTransferPending => ErrorCategory::Fatal,
            ContractError::NoPendingAdminTransfer => ErrorCategory::Fatal,
            ContractError::AttestationRequired => ErrorCategory::Fatal,
            ContractError::NetworkMismatch => ErrorCategory::Fatal,
            ContractError::NoPendingRoleGrant => ErrorCategory::Fatal,
            ContractError::ProvenanceMissing => ErrorCategory::Fatal,
            ContractError::ProvenanceMismatch => ErrorCategory::Fatal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_allowlist_variants_are_classified() {
        assert_eq!(
            ContractError::VerifierAllowlistFull.category(),
            ErrorCategory::Fatal
        );
        assert_eq!(
            ContractError::VerifierNotAllowlisted.category(),
            ErrorCategory::Auth
        );
        assert_eq!(
            ContractError::VerifierExpiryInPast.category(),
            ErrorCategory::Auth
        );
    }

    #[test]
    fn newly_added_variants_are_classified() {
        assert_eq!(
            ContractError::NoPendingRoleGrant.category(),
            ErrorCategory::Fatal
        );
        assert_eq!(
            ContractError::RoleGrantNotReady.category(),
            ErrorCategory::Retry
        );
        assert_eq!(
            ContractError::ProvenanceMissing.category(),
            ErrorCategory::Fatal
        );
        assert_eq!(
            ContractError::ProvenanceMismatch.category(),
            ErrorCategory::Fatal
        );
    }

    #[test]
    fn category_is_exhaustive_for_all_variants() {
        // Every variant must resolve to a category; this compiles only when
        // `category()` has no wildcard arm and covers the full enum.
        let all = [
            ContractError::AlreadyInitialized,
            ContractError::NotInitialized,
            ContractError::NotAuthorized,
            ContractError::NotRegistered,
            ContractError::AlreadyVerified,
            ContractError::NotVerified,
            ContractError::Paused,
            ContractError::CooldownActive,
            ContractError::InvalidVersion,
            ContractError::InvalidRole,
            ContractError::InvalidUsername,
            ContractError::AttestationExpired,
            ContractError::UnattestedWasm,
            ContractError::InvalidBatchSize,
            ContractError::InvalidReasonCode,
            ContractError::ZeroAddress,
            ContractError::ChallengeAlreadyActive,
            ContractError::NoChallengeActive,
            ContractError::ChallengeNotResolvable,
            ContractError::ChallengeActive,
            ContractError::InvalidPauseReason,
            ContractError::AlreadyReserved,
            ContractError::NotReserved,
            ContractError::UsernameReserved,
            ContractError::ReservedListFull,
            ContractError::AdminTransferPending,
            ContractError::AdminTransferDelayActive,
            ContractError::NoPendingAdminTransfer,
            ContractError::AttestationRequired,
            ContractError::NetworkMismatch,
            ContractError::VerifierAllowlistFull,
            ContractError::VerifierNotAllowlisted,
            ContractError::VerifierExpiryInPast,
            ContractError::NoPendingRoleGrant,
            ContractError::RoleGrantNotReady,
            ContractError::ProvenanceMissing,
            ContractError::ProvenanceMismatch,
        ];
        for err in all {
            let _ = err.category();
        }
    }
}
