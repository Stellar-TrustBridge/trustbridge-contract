//! Integration coverage for the staged-WASM hash mismatch error path
//! (Issue #680).
//!
//! When a hash is staged via `stage_wasm`, both upgrade paths (`upgrade` and
//! multisig `execute_upgrade`) must fail closed with `StagedWasmMismatch` if
//! given a different hash, leaving the staged slot, live proposal, and
//! provenance untouched. See `docs/ABI.md` (error code 39).

#![cfg(test)]

use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, BytesN, Env};

use trustbridge_contract::{ContractError, TrustBridgeContract};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    (env, admin, contract_id)
}

fn hash(env: &Env, byte: u8) -> BytesN<32> {
    BytesN::from_array(env, &[byte; 32])
}

fn real_wasm_hash(env: &Env) -> BytesN<32> {
    let wasm_bytes = soroban_sdk::Bytes::from_slice(
        env,
        include_bytes!("../target/wasm32v1-none/release/trustbridge_contract.wasm"),
    );
    env.deployer().upload_contract_wasm(wasm_bytes)
}

#[test]
fn test_upgrade_with_mismatched_staged_hash_fails_closed() {
    let (env, admin, contract_id) = setup();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::stage_wasm(env.clone(), admin.clone(), hash(&env, 0x01)).unwrap();
        assert_eq!(
            TrustBridgeContract::upgrade(env.clone(), hash(&env, 0x02)),
            Err(ContractError::StagedWasmMismatch)
        );
        let staged = TrustBridgeContract::get_staged(env.clone()).unwrap();
        assert_eq!(staged.wasm_hash, hash(&env, 0x01));
        assert!(TrustBridgeContract::get_provenance(env.clone()).is_none());
    });
}

#[test]
fn test_upgrade_with_matching_staged_hash_succeeds() {
    let (env, admin, contract_id) = setup();
    let new_hash = real_wasm_hash(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::stage_wasm(env.clone(), admin.clone(), new_hash.clone()).unwrap();
        TrustBridgeContract::upgrade(env.clone(), new_hash.clone()).unwrap();
        let prov = TrustBridgeContract::get_provenance(env.clone()).unwrap();
        assert_eq!(prov.wasm_hash, new_hash);
    });
}

#[test]
fn test_execute_upgrade_with_mismatched_staged_hash_fails_closed() {
    let (env, admin, contract_id) = setup();
    env.ledger().set_timestamp(1_000);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::stage_wasm(env.clone(), admin.clone(), hash(&env, 0x0A)).unwrap();
        TrustBridgeContract::propose_multisig_upgrade(env.clone(), admin.clone(), hash(&env, 0x0B), 0)
            .unwrap();
        assert_eq!(
            TrustBridgeContract::execute_upgrade(env.clone(), admin.clone(), 0),
            Err(ContractError::StagedWasmMismatch)
        );
        // No partial privileged state: proposal, staged slot, provenance intact.
        assert!(TrustBridgeContract::get_upgrade_proposal(env.clone()).is_some());
        let staged = TrustBridgeContract::get_staged(env.clone()).unwrap();
        assert_eq!(staged.wasm_hash, hash(&env, 0x0A));
        assert!(TrustBridgeContract::get_provenance(env.clone()).is_none());
    });
}

#[test]
fn test_execute_upgrade_with_matching_staged_hash_clears_slot() {
    let (env, admin, contract_id) = setup();
    let new_hash = real_wasm_hash(&env);
    env.ledger().set_timestamp(1_000);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::stage_wasm(env.clone(), admin.clone(), new_hash.clone()).unwrap();
        TrustBridgeContract::propose_multisig_upgrade(env.clone(), admin.clone(), new_hash.clone(), 0)
            .unwrap();
        TrustBridgeContract::execute_upgrade(env.clone(), admin.clone(), 0).unwrap();
        assert!(TrustBridgeContract::get_staged(env.clone()).is_none());
        assert_eq!(
            TrustBridgeContract::get_provenance(env.clone()).unwrap().wasm_hash,
            new_hash
        );
    });
}
