//! Oracle-signed proof-of-ownership interface.
//!
//! Today `verify()` is "admin said so": the admin (or a `Role::Verifier`
//! holder) calls `verify`, and the contract trusts that they checked GitHub
//! ownership off-chain. There is no proof bytes anywhere in that path — no
//! signature, no key the contract can point to, nothing an auditor can
//! replay independently of trusting the caller.
//!
//! This module adds the missing piece: a **signature-check interface**. An
//! allowlisted oracle key signs a message off-chain (after doing its own
//! GitHub OAuth / API check) and `verify_with_proof` checks that signature,
//! the signer's allowlist membership, and the proof's expiry, on-chain.
//!
//! Explicitly out of scope here (see `docs/SECURITY.md`):
//! - Running a production GitHub oracle service.
//! - Wiring a valid proof into `verify()`/`batch_verify()` as an alternative
//!   auth path — this module ships the primitive and its tests, not the
//!   integration.
//! - This is **not** the attestation-hash flow (`attest_upgrade`/`upgrade`),
//!   which binds a WASM hash for upgrades. This binds an oracle signature to
//!   an arbitrary message for identity/ownership proofs.

use soroban_sdk::{contracttype, symbol_short, Address, Bytes, BytesN, Env, Symbol, Vec};

use crate::error::ContractError;

/// Instance storage key for the oracle pubkey allowlist.
const ORACLE_ALLOWLIST_KEY: Symbol = symbol_short!("oraclepk");

/// A signed attestation from an off-chain oracle, covering `message`.
///
/// Canonical `message` construction (e.g. binding a specific
/// `github_username` + `stellar_address` + expiry into the signed bytes) is
/// left to the caller/oracle to agree on and is documented in
/// `docs/SECURITY.md`; this primitive only checks the signature, the
/// allowlist, and the expiry — not message contents.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleProof {
    /// Ed25519 public key of the oracle that produced `signature`.
    pub oracle_pubkey: BytesN<32>,
    /// The exact bytes the oracle signed.
    pub message: Bytes,
    /// Ed25519 signature over `message`, produced by `oracle_pubkey`.
    pub signature: BytesN<64>,
    /// Unix timestamp after which this proof is rejected. `0` disables the
    /// expiry check — production callers should always set a real value.
    pub expires_at: u64,
}

/// Replaces the set of oracle public keys whose signatures
/// `verify_with_proof` will accept. Admin-gated.
///
/// # Errors
/// Propagates the panic from `admin.require_auth()` if `admin` does not sign
/// (Soroban auth failures abort the invocation rather than returning `Err`,
/// consistent with every other admin-gated entry point in this contract).
pub fn set_oracle_allowlist(env: &Env, admin: &Address, pubkeys: Vec<BytesN<32>>) {
    admin.require_auth();
    env.storage().instance().set(&ORACLE_ALLOWLIST_KEY, &pubkeys);
}

/// Reads the current oracle pubkey allowlist (empty `Vec` if never set).
#[must_use]
pub fn get_oracle_allowlist(env: &Env) -> Vec<BytesN<32>> {
    env.storage()
        .instance()
        .get(&ORACLE_ALLOWLIST_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

/// Verifies an [`OracleProof`]: the signing key must be allowlisted, the
/// proof must not be expired, and the signature must be valid.
///
/// # Errors
/// - [`ContractError::NotAuthorized`] if `oracle_pubkey` is not on the
///   allowlist, or the allowlist is empty.
/// - [`ContractError::NotAuthorized`] if `expires_at != 0` and the current
///   ledger timestamp is past it.
///
/// # Panics
/// Panics (aborting the host invocation) if `signature` is not a valid
/// Ed25519 signature over `message` under `oracle_pubkey`.
/// `soroban_sdk::Env::crypto().ed25519_verify` has no non-panicking form, so
/// an invalid proof fails the same way a bad `require_auth()` signature
/// already does elsewhere in this contract — see `docs/SECURITY.md`.
pub fn verify_with_proof(env: &Env, proof: &OracleProof) -> Result<(), ContractError> {
    let allowlist = get_oracle_allowlist(env);
    if !allowlist.contains(&proof.oracle_pubkey) {
        return Err(ContractError::NotAuthorized);
    }

    if proof.expires_at != 0 && env.ledger().timestamp() > proof.expires_at {
        return Err(ContractError::NotAuthorized);
    }

    env.crypto()
        .ed25519_verify(&proof.oracle_pubkey, &proof.message, &proof.signature);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};

    // Fixed Ed25519 test vectors (seed = sha256("trustbridge-oracle-test-key-v1")),
    // generated offline so this test has no crypto dependency of its own.
    const ORACLE_PUBKEY: [u8; 32] = [
        179, 58, 29, 50, 177, 220, 29, 101, 34, 2, 132, 169, 155, 83, 4, 194, 225, 186, 125, 165,
        137, 197, 121, 222, 196, 246, 147, 247, 102, 8, 90, 166,
    ];
    const MESSAGE: [u8; 77] = [
        116, 101, 115, 116, 117, 115, 101, 114, 58, 71, 65, 66, 67, 68, 69, 70, 48, 48, 48, 48,
        48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48,
        48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 83, 84,
        85, 66, 58, 49, 55, 48, 48, 48, 48, 48, 48, 48, 48,
    ];
    const VALID_SIG: [u8; 64] = [
        117, 69, 211, 183, 32, 144, 147, 233, 197, 136, 100, 225, 34, 11, 169, 234, 39, 112, 220,
        241, 53, 11, 254, 255, 175, 189, 62, 37, 252, 46, 4, 183, 147, 111, 236, 142, 101, 220,
        35, 244, 100, 244, 111, 163, 221, 163, 35, 23, 135, 226, 112, 145, 252, 229, 115, 157, 23,
        214, 157, 70, 116, 9, 179, 11,
    ];
    // Same message, unrelated (non-allowlisted) key + its own valid signature.
    const ATTACKER_PUBKEY: [u8; 32] = [
        71, 194, 10, 17, 92, 51, 106, 49, 61, 177, 42, 166, 167, 111, 3, 185, 32, 53, 254, 31, 7,
        18, 197, 196, 78, 132, 56, 137, 212, 159, 252, 45,
    ];
    const ATTACKER_SIG: [u8; 64] = [
        107, 144, 82, 178, 100, 140, 209, 217, 141, 246, 240, 191, 121, 3, 27, 190, 98, 255, 82,
        26, 29, 54, 252, 221, 108, 39, 144, 147, 190, 19, 29, 183, 203, 154, 218, 176, 243, 4,
        250, 230, 53, 154, 23, 108, 120, 13, 174, 227, 104, 111, 30, 95, 209, 163, 116, 23, 213,
        157, 182, 119, 183, 178, 157, 0,
    ];
    // VALID_SIG with the last byte flipped — same key, corrupted signature.
    const TAMPERED_SIG: [u8; 64] = [
        117, 69, 211, 183, 32, 144, 147, 233, 197, 136, 100, 225, 34, 11, 169, 234, 39, 112, 220,
        241, 53, 11, 254, 255, 175, 189, 62, 37, 252, 46, 4, 183, 147, 111, 236, 142, 101, 220,
        35, 244, 100, 244, 111, 163, 221, 163, 35, 23, 135, 226, 112, 145, 252, 229, 115, 157, 23,
        214, 157, 70, 116, 9, 179, 244,
    ];

    fn setup(env: &Env) -> Address {
        let contract_id = env.register(crate::TrustBridgeContract, ());
        let admin = Address::generate(env);
        env.as_contract(&contract_id, || {
            crate::TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
        });
        contract_id
    }

    #[test]
    fn valid_allowlisted_proof_passes() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = setup(&env);
        let admin = env.as_contract(&contract_id, || crate::storage::get_admin(&env).unwrap());

        env.as_contract(&contract_id, || {
            let mut allowlist = Vec::new(&env);
            allowlist.push_back(BytesN::from_array(&env, &ORACLE_PUBKEY));
            set_oracle_allowlist(&env, &admin, allowlist);

            let proof = OracleProof {
                oracle_pubkey: BytesN::from_array(&env, &ORACLE_PUBKEY),
                message: Bytes::from_array(&env, &MESSAGE),
                signature: BytesN::from_array(&env, &VALID_SIG),
                expires_at: 0,
            };
            assert!(verify_with_proof(&env, &proof).is_ok());
        });
    }

    #[test]
    fn non_allowlisted_key_is_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = setup(&env);
        let admin = env.as_contract(&contract_id, || crate::storage::get_admin(&env).unwrap());

        env.as_contract(&contract_id, || {
            let mut allowlist = Vec::new(&env);
            allowlist.push_back(BytesN::from_array(&env, &ORACLE_PUBKEY));
            set_oracle_allowlist(&env, &admin, allowlist);

            // A signature that is internally valid, but from a key nobody allowlisted.
            let proof = OracleProof {
                oracle_pubkey: BytesN::from_array(&env, &ATTACKER_PUBKEY),
                message: Bytes::from_array(&env, &MESSAGE),
                signature: BytesN::from_array(&env, &ATTACKER_SIG),
                expires_at: 0,
            };
            assert_eq!(
                verify_with_proof(&env, &proof),
                Err(ContractError::NotAuthorized)
            );
        });
    }

    #[test]
    fn expired_proof_is_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = setup(&env);
        let admin = env.as_contract(&contract_id, || crate::storage::get_admin(&env).unwrap());

        env.as_contract(&contract_id, || {
            let mut allowlist = Vec::new(&env);
            allowlist.push_back(BytesN::from_array(&env, &ORACLE_PUBKEY));
            set_oracle_allowlist(&env, &admin, allowlist);
        });

        env.ledger().set_timestamp(2_000_000_000);

        env.as_contract(&contract_id, || {
            let proof = OracleProof {
                oracle_pubkey: BytesN::from_array(&env, &ORACLE_PUBKEY),
                message: Bytes::from_array(&env, &MESSAGE),
                signature: BytesN::from_array(&env, &VALID_SIG),
                expires_at: 1_700_000_000,
            };
            assert_eq!(
                verify_with_proof(&env, &proof),
                Err(ContractError::NotAuthorized)
            );
        });
    }

    #[test]
    #[should_panic]
    fn tampered_signature_traps() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = setup(&env);
        let admin = env.as_contract(&contract_id, || crate::storage::get_admin(&env).unwrap());

        env.as_contract(&contract_id, || {
            let mut allowlist = Vec::new(&env);
            allowlist.push_back(BytesN::from_array(&env, &ORACLE_PUBKEY));
            set_oracle_allowlist(&env, &admin, allowlist);

            let proof = OracleProof {
                oracle_pubkey: BytesN::from_array(&env, &ORACLE_PUBKEY),
                message: Bytes::from_array(&env, &MESSAGE),
                signature: BytesN::from_array(&env, &TAMPERED_SIG),
                expires_at: 0,
            };
            // Invalid signature under an allowlisted key: no clean `Err`, the
            // host traps. Documented in `verify_with_proof`'s `# Panics`.
            let _ = verify_with_proof(&env, &proof);
        });
    }
}
