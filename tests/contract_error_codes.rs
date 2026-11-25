//! Frozen numeric ABI checks for every `ContractError` code (Issue #394).
//!
//! The golden file is the record of which number means which error. These
//! tests enforce it in both directions, which matters more than it sounds: a
//! golden that only checks the codes it lists cannot notice a *new* variant
//! added without an entry, and an unrecorded code is exactly the one a later
//! refactor feels free to renumber.

use soroban_sdk::{
    testutils::{Address as _, Ledger as _, LedgerInfo},
    Address, Env, String as SString, Vec as SVec,
};
use trustbridge_contract::{ContractError, ErrorCategory, TrustBridgeContract};

const GOLDEN: &str = include_str!("../abi/contract_error_codes.golden");
const ABI_DOCS: &str = include_str!("../docs/ABI.md");

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
        "ThresholdExceedsSignerCap" => ContractError::ThresholdExceedsSignerCap,
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

// Frozen Display snapshots for every variant. The category mapping is
// separately cross-checked against docs/ABI.md below.

#[test]
fn display_strings_match_snapshot() {
    let cases: &[(ContractError, &str)] = &[
        (
            ContractError::AlreadyInitialized,
            "contract already initialized",
        ),
        (ContractError::NotInitialized, "contract not initialized"),
        (ContractError::NotAuthorized, "caller not authorized"),
        (
            ContractError::NotRegistered,
            "github username not registered",
        ),
        (ContractError::AlreadyVerified, "username already verified"),
        (ContractError::NotVerified, "username not verified"),
        (ContractError::Paused, "contract is paused"),
        (ContractError::CooldownActive, "upgrade cooldown active"),
        (ContractError::InvalidVersion, "invalid version"),
        (ContractError::InvalidRole, "invalid role"),
        (ContractError::InvalidUsername, "invalid github username"),
        (ContractError::AttestationExpired, "attestation expired"),
        (ContractError::UnattestedWasm, "unattested wasm"),
        (ContractError::InvalidBatchSize, "invalid batch size"),
        (ContractError::InvalidReasonCode, "invalid reason code"),
        (ContractError::ZeroAddress, "zero address"),
        (
            ContractError::ChallengeAlreadyActive,
            "challenge already active",
        ),
        (ContractError::NoChallengeActive, "no challenge active"),
        (
            ContractError::ChallengeNotResolvable,
            "challenge not resolvable",
        ),
        (ContractError::ChallengeActive, "challenge active"),
        (ContractError::InvalidPauseReason, "invalid pause reason"),
        (ContractError::AlreadyReserved, "username already reserved"),
        (ContractError::NotReserved, "username not reserved"),
        (ContractError::UsernameReserved, "username reserved"),
        (ContractError::ReservedListFull, "reserved list full"),
        (
            ContractError::AdminTransferPending,
            "admin transfer pending",
        ),
        (
            ContractError::AdminTransferDelayActive,
            "admin transfer delay active",
        ),
        (
            ContractError::NoPendingAdminTransfer,
            "no pending admin transfer",
        ),
        (
            ContractError::AttestationRequired,
            "attestation required",
        ),
        (ContractError::NetworkMismatch, "network mismatch"),
        (
            ContractError::VerifierAllowlistFull,
            "verifier allowlist full",
        ),
        (
            ContractError::VerifierNotAllowlisted,
            "verifier not allowlisted",
        ),
        (
            ContractError::VerifierExpiryInPast,
            "verifier expiry in past",
        ),
        (
            ContractError::NoPendingRoleGrant,
            "no pending role grant",
        ),
        (ContractError::RoleGrantNotReady, "role grant not ready"),
        (ContractError::ProvenanceMissing, "provenance missing"),
        (ContractError::ProvenanceMismatch, "provenance mismatch"),
        (
            ContractError::StagedWasmMismatch,
            "staged wasm mismatch",
        ),
        (
            ContractError::UpgradeProposalAlreadyPending,
            "upgrade proposal already pending",
        ),
        (
            ContractError::NoUpgradeProposalPending,
            "no upgrade proposal pending",
        ),
        (
            ContractError::UpgradeProposalAlreadyApproved,
            "upgrade proposal already approved",
        ),
        (
            ContractError::ThresholdExceedsSignerCap,
            "threshold exceeds signer cap",
        ),
        (
            ContractError::UpgradeProposalDelayActive,
            "upgrade proposal delay active",
        ),
        (
            ContractError::UpgradeProposalInsufficientApprovals,
            "upgrade proposal insufficient approvals",
        ),
        (ContractError::FallbackListFull, "fallback list full"),
        (ContractError::UsernameTaken, "username taken"),
        (ContractError::RotationRequired, "rotation required"),
        (ContractError::RotationPending, "rotation pending"),
        (ContractError::NoRotationPending, "no rotation pending"),
        (ContractError::RotationNotReady, "rotation not ready"),
        (ContractError::InvalidCursor, "invalid cursor"),
        (ContractError::VerifyRateLimited, "verify rate limited"),
        (
            ContractError::DualControlRequired,
            "dual control required",
        ),
        (
            ContractError::BatchRemoveProposalPending,
            "batch remove proposal pending",
        ),
        (
            ContractError::NoPendingBatchRemove,
            "no pending batch remove",
        ),
        (ContractError::RoleExpired, "role expired"),
        (
            ContractError::OracleProofBadLayout,
            "oracle proof bad layout",
        ),
        (
            ContractError::OracleProofNotAllowlisted,
            "oracle proof not allowlisted",
        ),
        (
            ContractError::OracleProofExpired,
            "oracle proof expired",
        ),
        (
            ContractError::OracleProofBadSignature,
            "oracle proof bad signature",
        ),
        (ContractError::AddressMismatch, "address mismatch"),
    ];
    for (error, expected) in cases {
        assert_eq!(
            error.to_string(),
            *expected,
            "display string drifted for {error:?}"
        );
    }
    let mut live_count = 0;
    for code in 1..=62 {
        if let Some(error) = ContractError::from_code(code) {
            live_count += 1;
            assert!(
                cases.iter().any(|(snapshot, _)| *snapshot == error),
                "ContractError::{error:?} is missing a Display snapshot"
            );
        }
    }
    assert_eq!(
        cases.len(),
        live_count,
        "every live error must have exactly one Display snapshot"
    );
}

#[test]
fn error_category_mapping_matches_abi_documentation() {
    let section = ABI_DOCS
        .split("<!-- error-category-mapping:start -->")
        .nth(1)
        .and_then(|tail| tail.split("<!-- error-category-mapping:end -->").next())
        .expect("ABI.md must have a delimited error-category mapping table");
    let mut documented = std::collections::BTreeMap::new();

    for line in section.lines().filter(|line| line.starts_with("| `")) {
        let columns: Vec<&str> = line.split('|').map(str::trim).collect();
        assert_eq!(columns.len(), 4, "malformed category table row: {line}");
        let category = match columns[1].trim_matches('`') {
            "Auth" => ErrorCategory::Auth,
            "Retry" => ErrorCategory::Retry,
            "Fatal" => ErrorCategory::Fatal,
            other => panic!("unknown documented error category: {other}"),
        };
        let names: Vec<&str> = columns[2].split('`').collect();
        assert_eq!(
            names.len() % 2,
            1,
            "malformed variant list in category row: {line}"
        );
        for index in (1..names.len()).step_by(2) {
            let name = names[index].trim();
            assert!(
                documented.insert(name, category).is_none(),
                "ContractError::{name} appears more than once in category table"
            );
        }
    }

    let mut live_count = 0;
    for code in 1..=62 {
        let Some(error) = ContractError::from_code(code) else {
            continue;
        };
        live_count += 1;
        let name = std::format!("{error:?}");
        assert_eq!(
            documented.get(name.as_str()),
            Some(&error.category()),
            "ABI.md category differs for ContractError::{error:?}"
        );
    }
    assert_eq!(
        documented.len(),
        live_count,
        "ABI.md must document every live ContractError exactly once"
    );
}

#[test]
fn pending_reverify_pagination_happy_path_and_documented_codes_match_abi() {
    let error_rs = include_str!("../src/error.rs");
    let storage_rs = include_str!("../src/storage.rs");

    for (code, name) in [
        (2u32, "NotInitialized"),
        (14u32, "InvalidBatchSize"),
        (30u32, "NetworkMismatch"),
    ] {
        let err = ContractError::from_code(code).expect("documented code must exist");
        assert_eq!(err.code(), code);
        assert_eq!(err.category(), ErrorCategory::Fatal);
        assert!(ABI_DOCS.contains(name));
        assert!(error_rs.contains(name));
        assert!(storage_rs.contains(name));
    }
    assert!(ABI_DOCS.contains("### `get_pending_reverify(offset: u32, limit: u32)"));
    assert!(ABI_DOCS.contains("Error codes & integrator notes for `get_pending_reverify`"));

    let env = Env::default();
    let admin = Address::generate(&env);
    let user_a = Address::generate(&env);
    let user_b = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            SString::from_str(&env, "alice"),
            user_a,
            SVec::new(&env),
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(
            env.clone(),
            admin.clone(),
            SString::from_str(&env, "alice"),
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            SString::from_str(&env, "alice"),
            user_b,
            SVec::new(&env),
        )
        .unwrap();
    });
    // Pause must not block get_pending_reverify.
    env.as_contract(&contract_id, || {
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
    });

    env.as_contract(&contract_id, || {
        let default_page = TrustBridgeContract::get_pending_reverify(env.clone(), 0, 0).unwrap();
        assert_eq!(
            default_page,
            SVec::from_array(&env, [SString::from_str(&env, "alice")])
        );
        let max_page = TrustBridgeContract::get_pending_reverify(env.clone(), 0, 100).unwrap();
        assert_eq!(max_page, default_page);
        let past_end =
            TrustBridgeContract::get_pending_reverify(env.clone(), u32::MAX, 1).unwrap();
        assert!(past_end.is_empty());
    });
}

#[test]
fn pending_reverify_pagination_uninitialized_returns_code_2() {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());
    let res = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_pending_reverify(env.clone(), 0, 20)
    });
    assert_eq!(res, Err(ContractError::NotInitialized));
    assert_eq!(res.unwrap_err().code(), 2);
}

#[test]
fn pending_reverify_pagination_oversized_limit_returns_code_14() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin).unwrap();
    });

    for bad_limit in [101u32, u32::MAX] {
        let res = env.as_contract(&contract_id, || {
            TrustBridgeContract::get_pending_reverify(env.clone(), 0, bad_limit)
        });
        assert_eq!(res, Err(ContractError::InvalidBatchSize));
        assert_eq!(res.unwrap_err().code(), 14);
    }
}

#[test]
fn pending_reverify_pagination_wrong_network_returns_code_30() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin).unwrap();
    });

    let mut mismatched = [0u8; 32];
    mismatched[0] = 0xAA;
    let info = env.ledger().get();
    env.ledger().set(LedgerInfo {
        network_id: mismatched,
        ..info
    });

    let res = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_pending_reverify(env.clone(), 0, 20)
    });
    assert_eq!(res, Err(ContractError::NetworkMismatch));
    assert_eq!(res.unwrap_err().code(), 30);
}
