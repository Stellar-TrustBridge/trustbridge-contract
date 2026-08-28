//! Frozen numeric ABI checks for the first ContractError codes.

use trustbridge_contract::ContractError;

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
        _ => panic!("unknown ContractError in golden file: {name}"),
    }
}

#[test]
fn contract_error_codes_match_golden() {
    for line in include_str!("../abi/contract_error_codes.golden").lines() {
        let (code, name) = line
            .split_once(' ')
            .expect("golden entries must contain a code and variant name");
        let code: u32 = code.parse().expect("golden error codes must be u32");
        let variant = variant_for_name(name);

        assert_eq!(variant.code(), code, "ContractError::{name} was renumbered");
        assert_eq!(
            ContractError::from_code(code),
            Some(variant),
            "from_code mapping changed for ContractError::{name}"
        );
    }
}