use soroban_sdk::contracterror;

/// Errors returned by contract entry points.
///
/// Each variant maps to a stable `u32` code (see `code()` / `from_code()`).
/// Off-chain consumers such as the dashboard and indexer use these codes to
/// classify failed invocations without depending on the Rust enum layout.
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
/// | 8 | `CooldownActive` | `upgrade` |
/// | 9 | `InvalidVersion` | `migrate` |
/// | 10 | `InvalidRole` | `set_role` |
/// | 11 | `InvalidUsername` | `register` |
/// | 15 | `InvalidReasonCode` | `revoke_verification` |
/// | 16 | `ZeroAddress` | `register` |
/// | 17 | `InvalidPauseReason` | `pause`, `unpause`, `set_paused` |
/// | 18 | `AlreadyReserved` | `add_reserved` |
/// | 19 | `NotReserved` | `remove_reserved` |
/// | 20 | `UsernameReserved` | `register` |
/// | 21 | `ReservedListFull` | `add_reserved` |
/// | 31 | `VerifierAllowlistFull` | `add_verifier` |
/// | 32 | `VerifierNotAllowlisted` | `remove_verifier` |
/// | 33 | `VerifierExpiryInPast` | `add_verifier` |
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
    InvalidEntityType = 6,
    OrgNameRequired = 7,
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
    /// `add_verifier` would exceed the `MAX_VERIFIERS` allowlist cap (Issue #293).
    VerifierAllowlistFull = 31,
    /// `remove_verifier` was called for an address not on the allowlist (Issue #293).
    VerifierNotAllowlisted = 32,
    /// `add_verifier` was given a non-zero `expires_at` that is not in the
    /// future (Issue #293).
    VerifierExpiryInPast = 33,
}

impl ContractError {
    #[must_use]
    pub fn code(self) -> u32 {
        self as u32
    }

    /// Reverse of `code()`: maps a raw u32 (e.g. decoded from a failed
    /// invocation's XDR result by a dashboard or indexer) back to the typed
    /// variant. Returns `None` for codes that don't correspond to a variant,
    /// so callers don't need to keep their own copy of this table in sync.
    #[must_use]
    pub fn from_code(code: u32) -> Option<ContractError> {
        match code {
            1 => Some(ContractError::AlreadyInitialized),
            2 => Some(ContractError::NotInitialized),
            3 => Some(ContractError::NotAuthorized),
            4 => Some(ContractError::NotRegistered),
            5 => Some(ContractError::AlreadyVerified),
            6 => Some(ContractError::NotVerified),
            7 => Some(ContractError::Paused),
            8 => Some(ContractError::CooldownActive),
            9 => Some(ContractError::InvalidVersion),
            10 => Some(ContractError::InvalidRole),
            11 => Some(ContractError::InvalidUsername),
            12 => Some(ContractError::AttestationExpired),
            13 => Some(ContractError::UnattestedWasm),
            14 => Some(ContractError::InvalidBatchSize),
            15 => Some(ContractError::InvalidReasonCode),
            16 => Some(ContractError::ZeroAddress),
            17 => Some(ContractError::ChallengeAlreadyActive),
            18 => Some(ContractError::NoChallengeActive),
            19 => Some(ContractError::ChallengeNotResolvable),
            20 => Some(ContractError::ChallengeActive),
            21 => Some(ContractError::InvalidPauseReason),
            22 => Some(ContractError::AlreadyReserved),
            23 => Some(ContractError::NotReserved),
            24 => Some(ContractError::UsernameReserved),
            25 => Some(ContractError::ReservedListFull),
            26 => Some(ContractError::AdminTransferPending),
            27 => Some(ContractError::AdminTransferDelayActive),
            28 => Some(ContractError::NoPendingAdminTransfer),
            29 => Some(ContractError::AttestationRequired),
            31 => Some(ContractError::VerifierAllowlistFull),
            32 => Some(ContractError::VerifierNotAllowlisted),
            33 => Some(ContractError::VerifierExpiryInPast),
            _ => None,
        }
    }
}

// Wave #42: ContractError code mapping for register / verify / remove / export
// consumers (dashboard, indexer, off-chain tooling) that need stable u32 codes
// without depending on the Rust enum layout.
//
// | Code | Variant             | Raised by                          |
// |------|----------------------|-------------------------------------|
// | 1    | AlreadyInitialized   | initialize                         |
// | 2    | NotInitialized       | register, remove, get_all_registered, verify, revoke_verification |
// | 3    | NotAuthorized        | remove, verify, revoke_verification |
// | 4    | NotRegistered        | remove, verify, revoke_verification |
// | 5    | AlreadyVerified      | verify                             |
// | 6    | NotVerified          | revoke_verification                |
// | 7    | Paused               | any state-mutating call while paused |
// | 8    | CooldownActive       | upgrade                            |
// | 9    | InvalidVersion       | migrate                            |
// | 10   | InvalidRole          | set_role                           |
// | 11   | InvalidUsername      | register                           |
// | 12   | AttestationExpired   | attest_upgrade, upgrade            |
// | 13   | UnattestedWasm       | upgrade                            |
// | 14   | InvalidBatchSize     | extend_registry_ttl                |
// | 15   | InvalidReasonCode    | revoke_verification                |
// | 16   | ZeroAddress          | register                           |
// | 21   | NetworkMismatch      | any call on state restored to a different network |
//
// `ContractError::from_code` is the reverse of this table for off-chain
// consumers decoding a raw error code back into a typed variant.
//
// Tests covering this mapping live in `src/lib.rs`
// (`test_error_codes_match_repr`, `test_from_code_round_trips_all_variants`,
// `test_from_code_unknown_returns_none`).
