//! Frozen numeric ABI checks for every `ContractError` code (Issue #394).
//!
//! The golden file is the record of which number means which error. These
//! tests enforce it in both directions, which matters more than it sounds: a
//! golden that only checks the codes it lists cannot notice a *new* variant
//! added without an entry, and an unrecorded code is exactly the one a later
//! refactor feels free to renumber.

use trustbridge_contract::ContractError;

const GOLDEN: &str = include_str!("../abi/contract_error_codes.golden");

fn variant_for_name(name: &str) -> ContractError {
    match name {
        "AlreadyInitialized" => ContractError::AlreadyInitialized,
        "NotInitialized" => ContractError::NotInitialized,
        "NotAuthorized" => ContractError::NotAuthorized,
        "NotRegistered" => ContractError::NotRegistered,
        "AlreadyVerified" => ContractError::AlreadyVerified,
        "NotVerified" => ContractError::NotVerified,
        "Paused" => ContractError::Paused,
        "CooldownActive" => ContractError::CooldownActive,
        "InvalidVersion" => ContractError::InvalidVersion,
        "InvalidRole" => ContractError::InvalidRole,
        "InvalidUsername" => ContractError::InvalidUsername,
        "AttestationExpired" => ContractError::AttestationExpired,
        "UnattestedWasm" => ContractError::UnattestedWasm,
        "InvalidBatchSize" => ContractError::InvalidBatchSize,
        "InvalidReasonCode" => ContractError::InvalidReasonCode,
        "ZeroAddress" => ContractError::ZeroAddress,
        "ChallengeAlreadyActive" => ContractError::ChallengeAlreadyActive,
        "NoChallengeActive" => ContractError::NoChallengeActive,
        "ChallengeNotResolvable" => ContractError::ChallengeNotResolvable,
        "ChallengeActive" => ContractError::ChallengeActive,
        "InvalidPauseReason" => ContractError::InvalidPauseReason,
        "AlreadyReserved" => ContractError::AlreadyReserved,
        "NotReserved" => ContractError::NotReserved,
        "UsernameReserved" => ContractError::UsernameReserved,
        "ReservedListFull" => ContractError::ReservedListFull,
        "AdminTransferPending" => ContractError::AdminTransferPending,
        "AdminTransferDelayActive" => ContractError::AdminTransferDelayActive,
        "NoPendingAdminTransfer" => ContractError::NoPendingAdminTransfer,
        "AttestationRequired" => ContractError::AttestationRequired,
        "NetworkMismatch" => ContractError::NetworkMismatch,
        "VerifierAllowlistFull" => ContractError::VerifierAllowlistFull,
        "VerifierNotAllowlisted" => ContractError::VerifierNotAllowlisted,
        "VerifierExpiryInPast" => ContractError::VerifierExpiryInPast,
        "NoPendingRoleGrant" => ContractError::NoPendingRoleGrant,
        "RoleGrantNotReady" => ContractError::RoleGrantNotReady,
        "RoleExpired" => ContractError::RoleExpired,
        "ProvenanceMissing" => ContractError::ProvenanceMissing,
        "ProvenanceMismatch" => ContractError::ProvenanceMismatch,
        "StagedWasmMismatch" => ContractError::StagedWasmMismatch,
        "UpgradeProposalAlreadyPending" => ContractError::UpgradeProposalAlreadyPending,
        "NoUpgradeProposalPending" => ContractError::NoUpgradeProposalPending,
        "UpgradeProposalAlreadyApproved" => ContractError::UpgradeProposalAlreadyApproved,
        "UpgradeProposalDelayActive" => ContractError::UpgradeProposalDelayActive,
        "UpgradeProposalInsufficientApprovals" => {
            ContractError::UpgradeProposalInsufficientApprovals
        }
        "FallbackListFull" => ContractError::FallbackListFull,
        "UsernameTaken" => ContractError::UsernameTaken,
        "RotationRequired" => ContractError::RotationRequired,
        "RotationPending" => ContractError::RotationPending,
        "NoRotationPending" => ContractError::NoRotationPending,
        "RotationNotReady" => ContractError::RotationNotReady,
        "InvalidCursor" => ContractError::InvalidCursor,
        "VerifyRateLimited" => ContractError::VerifyRateLimited,
        "DualControlRequired" => ContractError::DualControlRequired,
        "BatchRemoveProposalPending" => ContractError::BatchRemoveProposalPending,
        "NoPendingBatchRemove" => ContractError::NoPendingBatchRemove,
        "OracleProofBadLayout" => ContractError::OracleProofBadLayout,
        "OracleProofNotAllowlisted" => ContractError::OracleProofNotAllowlisted,
        "OracleProofExpired" => ContractError::OracleProofExpired,
        "OracleProofBadSignature" => ContractError::OracleProofBadSignature,
        "AddressMismatch" => ContractError::AddressMismatch,
        _ => panic!(
            "unknown ContractError in golden file: {name}. \
             Add the variant to variant_for_name, or fix the golden entry."
        ),
    }
}

/// One parsed golden line.
enum Entry {
    /// `<code> <Variant>` — a live code frozen to that variant.
    Live { code: u32, name: String },
    /// `!<code> reserved` — a code that must map to no variant.
    Reserved { code: u32 },
}

/// Parse the golden, skipping comments and blank lines.
fn entries() -> Vec<Entry> {
    GOLDEN
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (first, rest) = line
                .split_once(' ')
                .expect("golden entries must be `<code> <Variant>` or `!<code> reserved`");

            if let Some(code) = first.strip_prefix('!') {
                assert_eq!(
                    rest.trim(),
                    "reserved",
                    "a `!<code>` entry must be marked `reserved`"
                );
                return Entry::Reserved {
                    code: code.parse().expect("golden error codes must be u32"),
                };
            }

            Entry::Live {
                code: first.parse().expect("golden error codes must be u32"),
                name: rest.trim().to_string(),
            }
        })
        .collect()
}

#[test]
fn network_mismatch_is_code_30_not_invalid_pause_reason() {
    // Issue #459: rustdoc / abi.json had listed NetworkMismatch at 21, which
    // is InvalidPauseReason. The contract discriminant is 30.
    assert_eq!(ContractError::InvalidPauseReason.code(), 21);
    assert_eq!(ContractError::NetworkMismatch.code(), 30);
    assert_eq!(
        ContractError::from_code(21),
        Some(ContractError::InvalidPauseReason)
    );
    assert_eq!(
        ContractError::from_code(30),
        Some(ContractError::NetworkMismatch)
    );
}

#[test]
fn contract_error_codes_match_golden() {
    for entry in entries() {
        let Entry::Live { code, name } = entry else {
            continue;
        };
        let variant = variant_for_name(&name);

        assert_eq!(variant.code(), code, "ContractError::{name} was renumbered");
        assert_eq!(
            ContractError::from_code(code),
            Some(variant),
            "from_code mapping changed for ContractError::{name}"
        );
    }
}

#[test]
fn reserved_codes_stay_unclaimed() {
    for entry in entries() {
        let Entry::Reserved { code } = entry else {
            continue;
        };

        assert_eq!(
            ContractError::from_code(code),
            None,
            "code {code} is reserved in the golden file but now maps to a variant. \
             Append the next unused code instead of filling a historical gap."
        );
    }
}

#[test]
fn golden_covers_every_variant() {
    // The half a per-entry check cannot do: a variant added to the enum
    // without a golden entry is unfrozen, and an unfrozen code is the one a
    // later refactor renumbers without anything failing.
    let recorded: Vec<u32> = entries()
        .iter()
        .filter_map(|entry| match entry {
            Entry::Live { code, .. } => Some(*code),
            Entry::Reserved { .. } => None,
        })
        .collect();

    // Scan well past the current maximum so a newly appended variant is caught
    // rather than sitting outside the window.
    for code in 1..=100u32 {
        let live = ContractError::from_code(code).is_some();
        let frozen = recorded.contains(&code);

        assert_eq!(
            live,
            frozen,
            "code {code}: {}",
            if live {
                "the enum defines it but the golden file does not record it — append an entry"
            } else {
                "the golden file records it but the enum no longer defines it — \
                 removing an error code is a breaking ABI change"
            }
        );
    }
}

#[test]
fn golden_codes_are_unique_and_ascending() {
    // Ascending order is not cosmetic: it is what makes "append the next
    // unused code" a reviewable one-line diff at the end of the file rather
    // than an insertion someone has to scan the whole list to verify.
    let mut previous = 0u32;

    for entry in entries() {
        let code = match entry {
            Entry::Live { code, .. } => code,
            Entry::Reserved { code } => code,
        };

        assert!(
            code > previous,
            "golden codes must be unique and ascending; {code} follows {previous}"
        );
        previous = code;
    }
}

// Display-string parity tests (Issue #625).
//
// The human-readable `Display` message for each variant must match the
// documented catalog in docs/ABI.md so operators decoding failed invocations
// see consistent wording.

#[test]
fn display_strings_match_documented_catalog() {
    let cases: &[(ContractError, &str)] = &[
        (ContractError::InvalidUsername, "invalid github username"),
        (ContractError::Paused, "contract is paused"),
        (ContractError::NotAuthorized, "caller not authorized"),
        (ContractError::NetworkMismatch, "network mismatch"),
        (ContractError::InvalidPauseReason, "invalid pause reason"),
        (ContractError::UsernameTaken, "username taken"),
        (ContractError::RoleExpired, "role expired"),
        (
            ContractError::OracleProofBadSignature,
            "oracle proof bad signature",
        ),
        (
            ContractError::AlreadyInitialized,
            "contract already initialized",
        ),
        (ContractError::VerifyRateLimited, "verify rate limited"),
        (ContractError::AddressMismatch, "address mismatch"),
    ];
    for (error, expected) in cases {
        assert_eq!(
            error.to_string(),
            *expected,
            "display string drifted for {error:?}"
        );
    }
}

// User-facing message snapshot (Issue #686).
//
// `abi/contract_error_messages.golden` freezes the `Display` text of every
// live code so integrators can map messages to UX copy without churn.

const MESSAGES_GOLDEN: &str = include_str!("../abi/contract_error_messages.golden");

/// Parse `<code> <Variant> <message>` lines, skipping comments and blanks.
fn message_entries() -> Vec<(u32, String, String)> {
    MESSAGES_GOLDEN
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut parts = line.splitn(3, ' ');
            let code = parts
                .next()
                .and_then(|c| c.parse().ok())
                .expect("message golden entries must start with a u32 code");
            let name = parts.next().expect("missing variant name").to_string();
            let message = parts.next().unwrap_or("").trim().to_string();
            (code, name, message)
        })
        .collect()
}

#[test]
fn display_messages_match_snapshot() {
    for (code, name, message) in message_entries() {
        let error = variant_for_name(&name);
        assert_eq!(error.code(), code, "{name} snapshot code drifted");
        assert!(!message.is_empty(), "{name} must have a non-empty message");
        assert_eq!(
            error.to_string(),
            message,
            "Display message for {name} drifted from abi/contract_error_messages.golden"
        );
    }
}

#[test]
fn message_snapshot_covers_every_live_code() {
    let live: Vec<(u32, String)> = entries()
        .into_iter()
        .filter_map(|entry| match entry {
            Entry::Live { code, name } => Some((code, name)),
            Entry::Reserved { .. } => None,
        })
        .collect();
    let snapshot: Vec<(u32, String)> = message_entries()
        .into_iter()
        .map(|(code, name, _)| (code, name))
        .collect();
    assert_eq!(
        snapshot, live,
        "abi/contract_error_messages.golden must list exactly the live codes, in order"
    );
}

#[test]
fn reserved_and_unknown_codes_have_no_message() {
    let snapshot_codes: Vec<u32> = message_entries().iter().map(|(c, _, _)| *c).collect();
    for entry in entries() {
        if let Entry::Reserved { code } = entry {
            assert!(
                !snapshot_codes.contains(&code),
                "reserved code {code} must not carry a user-facing message"
            );
        }
    }
    assert_eq!(ContractError::from_code(u32::MAX), None);
}
