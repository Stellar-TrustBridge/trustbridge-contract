//! Public oracle verification clock-skew boundaries (Issue #854).
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Bytes, BytesN, Env, String, Vec,
};
use trustbridge_contract::{
    ContractError, OracleProof, TrustBridgeContract, TrustBridgeContractClient,
};

// Offline test-only Ed25519 vector, using the same public fixture key as
// src/oracle_proof.rs. The signed message binds the username, address and expiry.
const OWNER: &str = "GADQOBYHA4DQOBYHA4DQOBYHA4DQOBYHA4DQOBYHA4DQOBYHA4DQOZPI";
const EXPIRES: u64 = 1_700_000_000;
const KEY: [u8; 32] = [
    179, 58, 29, 50, 177, 220, 29, 101, 34, 2, 132, 169, 155, 83, 4, 194, 225, 186, 125, 165,
    137, 197, 121, 222, 196, 246, 147, 247, 102, 8, 90, 166,
];
const SIG: [u8; 64] = [
    97, 226, 116, 36, 193, 8, 110, 168, 151, 100, 154, 52, 38, 46, 97, 94, 44, 96, 240, 82, 51, 205,
    129, 3, 220, 168, 125, 176, 51, 42, 97, 219, 222, 127, 191, 141, 142, 56, 228, 84, 107, 8, 68,
    84, 120, 102, 43, 242, 48, 172, 93, 232, 139, 16, 89, 0, 179, 100, 207, 153, 166, 168, 191, 14,
];

fn check_boundary(now: u64, signed_expiry: u64, paused: bool, expected: Result<(), ContractError>) {
    let env = Env::default();
    env.mock_all_auths();
    let client = TrustBridgeContractClient::new(&env, &env.register(TrustBridgeContract, ()));
    client.initialize(&Address::generate(&env));
    let owner = Address::from_str(&env, OWNER);
    let name = String::from_str(&env, "octocat");
    client.register(&name, &owner, &Vec::new(&env));
    let key = BytesN::from_array(&env, &KEY);
    let mut allowlist = Vec::new(&env);
    allowlist.push_back(key.clone());
    client.set_oracle_allowlist(&allowlist);
    let proof = OracleProof {
        oracle_pubkey: key,
        message: Bytes::from_slice(
            &env,
            b"octocat:GADQOBYHA4DQOBYHA4DQOBYHA4DQOBYHA4DQOBYHA4DQOBYHA4DQOZPI:1700000000",
        ),
        signature: BytesN::from_array(&env, &SIG),
        expires_at: signed_expiry,
    };

    env.ledger().set_timestamp(now);
    if paused {
        client.pause(&1);
    }
    match expected {
        Ok(()) => {
            client.verify_with_oracle_proof(&name, &proof);
            assert!(client.get_address(&name).unwrap().verified);
        }
        Err(error) => {
            assert_eq!(
                client.try_verify_with_oracle_proof(&name, &proof),
                Err(Ok(error))
            );
            assert!(!client.get_address(&name).unwrap().verified);
        }
    }
}

#[test]
fn oracle_proof_clock_skew_accepts_expiry_and_grace_boundary() {
    for now in [EXPIRES - 1, EXPIRES, EXPIRES + 30] {
        check_boundary(now, EXPIRES, false, Ok(()));
    }
}

#[test]
fn oracle_proof_clock_skew_rejects_expired_without_verifying() {
    for now in [EXPIRES + 31, EXPIRES + 500] {
        check_boundary(now, EXPIRES, false, Err(ContractError::OracleProofExpired));
    }
}

#[test]
fn oracle_proof_clock_skew_does_not_allow_tampered_expiry() {
    check_boundary(
        EXPIRES + 31,
        EXPIRES + 31,
        false,
        Err(ContractError::AddressMismatch),
    );
}

#[test]
fn oracle_proof_clock_skew_does_not_bypass_pause() {
    check_boundary(EXPIRES + 30, EXPIRES, true, Err(ContractError::Paused));
}
