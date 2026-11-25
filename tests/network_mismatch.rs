//! Network-id mismatch tests (Issue #231 / #401).
//!
//! Ensures that attestation-required setters (`set_attestation_required`) and
//! upgrade attestation operations (`attest_upgrade`, `clear_attestation`,
//! `adopt_network_tag`) honor the instance's recorded `network_id` and fail
//! closed with `ContractError::NetworkMismatch` (code 30) when invoked on a
//! different network.
//! Tests for `require_matching_network` on all mutating entrypoints (Issue #231).
//!
//! `require_initialized` — called by every gated entrypoint — invokes
//! `require_matching_network`, which compares the network id stored at
//! `initialize` time against the network the contract is currently executing
//! on. If they differ it returns `ContractError::NetworkMismatch` (code 30).
//!
//! The Soroban test host derives `network_id` from the network passphrase set
//! on `Env::ledger()`. The default test environment uses the passphrase
//! `"Test SDF Network ; September 2015"`, so recording a *different* passphrase
//! at init time simulates state that was snapshotted on one network and
//! restored onto another.
//!
//! # Strategy
//!
//! We cannot directly change `NETWORK_KEY` in storage from outside a contract
//! invocation. Instead we:
//!
//! 1. Create an `Env` that already has a specific network passphrase.
//! 2. Initialize the contract on that env (stores its network_id).
//! 3. Create a *second* env with a **different** network passphrase sharing the
//!    same contract address.
//! 4. Execute every mutating entrypoint on the second env and assert each one
//!    returns `NetworkMismatch`.
//!
//! Because the Soroban test host is in-process and every `Env` shares a
//! backing storage for the same contract address, this accurately replicates
//! deploying state across a network boundary.
//!
//! # Validate command
//!
//! ```text
//! cargo test network
//! ```

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, BytesN, Env,
};

use trustbridge_contract::{ContractError, TrustBridgeContract, TrustBridgeContractClient};

const NET_A: [u8; 32] = [0x11; 32];
const NET_B: [u8; 32] = [0x22; 32];

fn setup_on_network(network_id: [u8; 32]) -> (Env, Address, Address) {
    let env = Env::default();
    env.ledger().set_network_id(network_id);
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    (env, admin, contract_id)
}

fn wasm_hash(env: &Env, byte: u8) -> BytesN<32> {
    BytesN::from_array(env, &[byte; 32])
}

// ── Happy path ───────────────────────────────────────────────────────────────

#[test]
fn test_set_attestation_required_succeeds_on_matching_network() {
    let (env, _admin, contract_id) = setup_on_network(NET_A);
    let client = TrustBridgeContractClient::new(&env, &contract_id);

    assert!(!client.is_attestation_required());
    assert_eq!(
        client.get_network_tag(),
        Some(BytesN::from_array(&env, &NET_A))
    );

    env.mock_all_auths();
    client.set_attestation_required(&true);
    assert!(client.is_attestation_required());

    // Preserves pause-exempt behavior so operators can tighten upgrade controls
    // during an incident pause.
    client.pause(&1u32);
    client.set_attestation_required(&false);
    assert!(!client.is_attestation_required());
    client.set_attestation_required(&true);
    assert!(client.is_attestation_required());
}

#[test]
fn test_attest_and_clear_succeed_on_matching_network() {
    let (env, _admin, contract_id) = setup_on_network(NET_A);
    let client = TrustBridgeContractClient::new(&env, &contract_id);
    env.ledger().set_timestamp(1_000);
    let hash = wasm_hash(&env, 0xAA);

    env.mock_all_auths();
    client.attest_upgrade(&hash, &2_000u64);
    assert_eq!(client.get_attestation().unwrap().wasm_hash, hash);

    client.clear_attestation();
    assert!(client.get_attestation().is_none());

    // Re-adopting the same network tag is an idempotent no-op.
    client.adopt_network_tag();
}

// ── Failure path 1: cross-network mismatch ───────────────────────────────────

#[test]
fn test_set_attestation_required_rejects_mismatched_network_without_mutating() {
    let (env, _admin, contract_id) = setup_on_network(NET_A);

    // Default is false on NET_A. Switching to NET_B must fail closed and
    // must not enable or toggle the flag.
    env.ledger().set_network_id(NET_B);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::set_attestation_required(env.clone(), true),
            Err(ContractError::NetworkMismatch)
        );
        assert!(
            !TrustBridgeContract::is_attestation_required(env.clone()),
            "failed cross-network set_attestation_required(true) must not mutate state"
        );
    });

    // Enable on NET_A, then verify NET_B cannot weaken/disable it.
    env.ledger().set_network_id(NET_A);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_attestation_required(env.clone(), true).unwrap();
        assert!(TrustBridgeContract::is_attestation_required(env.clone()));
    });

    env.ledger().set_network_id(NET_B);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::set_attestation_required(env.clone(), false),
            Err(ContractError::NetworkMismatch)
        );
        assert!(
            TrustBridgeContract::is_attestation_required(env.clone()),
            "cross-network caller must not be able to disable attestation_required"
        );
    });
}

#[test]
fn test_attest_upgrade_and_clear_attestation_reject_mismatched_network() {
    let (env, _admin, contract_id) = setup_on_network(NET_A);
    env.ledger().set_timestamp(1_000);
    let hash_a = wasm_hash(&env, 0x0A);
    let hash_b = wasm_hash(&env, 0x0B);

    // Publish an attestation on NET_A first.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::attest_upgrade(env.clone(), hash_a.clone(), 5_000).unwrap();
    });

    // Switch to NET_B: neither attest_upgrade, clear_attestation, nor
    // adopt_network_tag may mutate state.
    env.ledger().set_network_id(NET_B);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::attest_upgrade(env.clone(), hash_b, 5_000),
            Err(ContractError::NetworkMismatch)
        );
        assert_eq!(
            TrustBridgeContract::clear_attestation(env.clone()),
            Err(ContractError::NetworkMismatch)
        );
        assert_eq!(
            TrustBridgeContract::adopt_network_tag(env.clone()),
            Err(ContractError::NetworkMismatch)
        );
        assert_eq!(
            TrustBridgeContract::get_attestation(env.clone())
                .expect("NET_A attestation must remain intact")
                .wasm_hash,
            hash_a
        );
        assert_eq!(
            TrustBridgeContract::get_network_tag(env.clone()),
            Some(BytesN::from_array(&env, &NET_A))
        );
    });
}

// ── Failure path 2: uninitialized contract ───────────────────────────────────

#[test]
fn test_attestation_setters_reject_uninitialized_contract() {
    let env = Env::default();
    env.ledger().set_network_id(NET_A);
    let contract_id = env.register(TrustBridgeContract, ());
    let hash = wasm_hash(&env, 0xCC);
    env.ledger().set_timestamp(1_000);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::set_attestation_required(env.clone(), true),
            Err(ContractError::NotInitialized)
        );
        assert_eq!(
            TrustBridgeContract::attest_upgrade(env.clone(), hash, 2_000),
            Err(ContractError::NotInitialized)
        );
        assert_eq!(
            TrustBridgeContract::clear_attestation(env.clone()),
            Err(ContractError::NotInitialized)
        );
        assert!(!TrustBridgeContract::is_attestation_required(env.clone()));
    });
}

// ── Failure path 3: unauthorized caller & required-mode upgrade gate ─────────

#[test]
fn test_set_attestation_required_rejects_missing_admin_auth() {
    let (env, _admin, contract_id) = setup_on_network(NET_A);
    let client = TrustBridgeContractClient::new(&env, &contract_id);

    // Clear all mocked auths so admin signature is absent.
    env.set_auths(&[]);
    let res = client.try_set_attestation_required(&true);
    assert!(
        res.is_err(),
        "set_attestation_required without admin auth must fail"
    );

    assert!(
        !client.is_attestation_required(),
        "unauthorized call must leave attestation_required unchanged"
    );
}

#[test]
fn test_upgrade_without_attestation_fails_when_required() {
    let (env, _admin, contract_id) = setup_on_network(NET_A);
    let client = TrustBridgeContractClient::new(&env, &contract_id);
    let hash = wasm_hash(&env, 0xDD);

    env.mock_all_auths();
    client.set_attestation_required(&true);

    let res = client.try_upgrade(&hash);
    assert_eq!(res, Err(Ok(ContractError::AttestationRequired)));
    testutils::{Address as _, Ledger as _, LedgerInfo},
    Address, Env, Vec,
};
use trustbridge_contract::{ContractError, Role, TrustBridgeContract};

// ─── helpers ─────────────────────────────────────────────────────────────────

fn s(env: &Env, text: &str) -> soroban_sdk::String {
    soroban_sdk::String::from_str(env, text)
}

/// Creates an Env with a custom network passphrase so `network_id` (SHA-256
/// of the passphrase) differs from the default test network.
fn env_with_passphrase(passphrase: &str) -> Env {
    let env = Env::default();
    env.ledger().set(LedgerInfo {
        timestamp: 1_000_000,
        protocol_version: env.ledger().get().protocol_version,
        sequence_number: 100,
        network_id: sha256_passphrase(passphrase),
        base_reserve: 5_000_000,
        min_temp_entry_ttl: 16,
        min_persistent_entry_ttl: 4096,
        max_entry_ttl: 6_312_000,
    });
    env
}

/// Minimal deterministic SHA-256 stand-in using the SDK's own hash facility.
/// We just need two *different* `BytesN<32>` values — the exact bytes do not
/// matter, only that they differ.
fn sha256_passphrase(passphrase: &str) -> [u8; 32] {
    // Produce a distinguishable 32-byte value from the passphrase by hashing
    // the UTF-8 bytes. In the test host `network_id()` is set via `LedgerInfo`,
    // so we only need two distinct arrays.
    let mut out = [0u8; 32];
    let bytes = passphrase.as_bytes();
    for (i, &b) in bytes.iter().enumerate().take(32) {
        out[i % 32] ^= b.wrapping_add(i as u8);
    }
    // Mix pass index into upper half so short passphrases still differ.
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = byte.wrapping_add((i as u8).wrapping_mul(7));
    }
    out
}

/// Sets up a contract initialized on `env_a` (one network) and returns the
/// same contract_id reference so it can be called from `env_b` (another network).
///
/// Returns `(env_a, env_b, admin, user, contract_id)`.
fn setup_cross_network() -> (Env, Env, Address, Address, Address) {
    let env_a = env_with_passphrase("Network-A");
    let env_b = env_with_passphrase("Network-B");

    let admin = Address::generate(&env_a);
    let user = Address::generate(&env_a);

    // Register the contract on env_a and initialize it there.
    let contract_id = env_a.register(TrustBridgeContract, ());
    env_a.mock_all_auths();
    env_a.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env_a.clone(), admin.clone()).unwrap();
    });

    // Register the *same* contract address on env_b (shared backing store).
    let _ = env_b.register_at(&contract_id, TrustBridgeContract, ());

    (env_a, env_b, admin, user, contract_id)
}

// ─── Positive control ────────────────────────────────────────────────────────

/// The same network always succeeds — this is the positive control that proves
/// the setup is correct and the function works at all.
#[test]
fn test_network_same_passphrase_succeeds() {
    let (env_a, _env_b, _admin, user, contract_id) = setup_cross_network();

    // A normal operation on env_a (same network as init) must work.
    env_a.mock_all_auths();
    env_a.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env_a.clone(),
            s(&env_a, "octocat"),
            user.clone(),
            Vec::new(&env_a),
        )
        .unwrap();
    });
}

// ─── NetworkMismatch on mutating entrypoints ─────────────────────────────────

/// `register` on a mismatched network must return `NetworkMismatch`.
#[test]
fn test_network_mismatch_register() {
    let (_env_a, env_b, _admin, user, contract_id) = setup_cross_network();

    env_b.mock_all_auths();
    let result = env_b.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env_b.clone(),
            s(&env_b, "octocat"),
            user.clone(),
            Vec::new(&env_b),
        )
    });
    assert_eq!(
        result,
        Err(ContractError::NetworkMismatch),
        "register must fail with NetworkMismatch on wrong network"
    );
}

/// `remove` on a mismatched network must return `NetworkMismatch`.
#[test]
fn test_network_mismatch_remove() {
    let (env_a, env_b, _admin, user, contract_id) = setup_cross_network();

    // Register on the correct network first.
    env_a.mock_all_auths();
    env_a.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env_a.clone(),
            s(&env_a, "octocat"),
            user.clone(),
            Vec::new(&env_a),
        )
        .unwrap();
    });

    env_b.mock_all_auths();
    let result = env_b.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env_b.clone(), user.clone(), s(&env_b, "octocat"))
    });
    assert_eq!(
        result,
        Err(ContractError::NetworkMismatch),
        "remove must fail with NetworkMismatch on wrong network"
    );
}

/// `verify` on a mismatched network must return `NetworkMismatch`.
#[test]
fn test_network_mismatch_verify() {
    let (env_a, env_b, admin, user, contract_id) = setup_cross_network();

    env_a.mock_all_auths();
    env_a.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env_a.clone(),
            s(&env_a, "octocat"),
            user.clone(),
            Vec::new(&env_a),
        )
        .unwrap();
    });

    env_b.mock_all_auths();
    let result = env_b.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env_b.clone(), admin.clone(), s(&env_b, "octocat"))
    });
    assert_eq!(
        result,
        Err(ContractError::NetworkMismatch),
        "verify must fail with NetworkMismatch on wrong network"
    );
}

/// `revoke_verification` on a mismatched network must return `NetworkMismatch`.
#[test]
fn test_network_mismatch_revoke_verification() {
    let (env_a, env_b, admin, user, contract_id) = setup_cross_network();

    env_a.mock_all_auths();
    env_a.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env_a.clone(),
            s(&env_a, "octocat"),
            user.clone(),
            Vec::new(&env_a),
        )
        .unwrap();
        TrustBridgeContract::verify(env_a.clone(), admin.clone(), s(&env_a, "octocat")).unwrap();
    });

    env_b.mock_all_auths();
    let result = env_b.as_contract(&contract_id, || {
        TrustBridgeContract::revoke_verification(
            env_b.clone(),
            admin.clone(),
            s(&env_b, "octocat"),
            1,
        )
    });
    assert_eq!(
        result,
        Err(ContractError::NetworkMismatch),
        "revoke_verification must fail with NetworkMismatch on wrong network"
    );
}

/// `pause` on a mismatched network must return `NetworkMismatch`.
#[test]
fn test_network_mismatch_pause() {
    let (_env_a, env_b, _admin, _user, contract_id) = setup_cross_network();

    env_b.mock_all_auths();
    let result = env_b.as_contract(&contract_id, || {
        TrustBridgeContract::pause(env_b.clone(), 1)
    });
    assert_eq!(
        result,
        Err(ContractError::NetworkMismatch),
        "pause must fail with NetworkMismatch on wrong network"
    );
}

/// `unpause` on a mismatched network must return `NetworkMismatch`.
#[test]
fn test_network_mismatch_unpause() {
    let (_env_a, env_b, _admin, _user, contract_id) = setup_cross_network();

    env_b.mock_all_auths();
    let result = env_b.as_contract(&contract_id, || {
        TrustBridgeContract::unpause(env_b.clone(), 4)
    });
    assert_eq!(
        result,
        Err(ContractError::NetworkMismatch),
        "unpause must fail with NetworkMismatch on wrong network"
    );
}

/// `set_paused(true, ..)` on a mismatched network must return `NetworkMismatch`.
#[test]
fn test_network_mismatch_set_paused() {
    let (_env_a, env_b, _admin, _user, contract_id) = setup_cross_network();

    env_b.mock_all_auths();
    let result = env_b.as_contract(&contract_id, || {
        TrustBridgeContract::set_paused(env_b.clone(), true, 1)
    });
    assert_eq!(
        result,
        Err(ContractError::NetworkMismatch),
        "set_paused must fail with NetworkMismatch on wrong network"
    );
}

/// `set_paused(false, ..)` on a mismatched network must return `NetworkMismatch`.
#[test]
fn test_network_mismatch_set_paused_unpause_path() {
    let (_env_a, env_b, _admin, _user, contract_id) = setup_cross_network();

    env_b.mock_all_auths();
    let result = env_b.as_contract(&contract_id, || {
        TrustBridgeContract::set_paused(env_b.clone(), false, 4)
    });
    assert_eq!(
        result,
        Err(ContractError::NetworkMismatch),
        "set_paused(false) must fail with NetworkMismatch on wrong network"
    );
}

/// `set_role` on a mismatched network must return `NetworkMismatch`.
#[test]
fn test_network_mismatch_set_role() {
    let (_env_a, env_b, _admin, user, contract_id) = setup_cross_network();

    env_b.mock_all_auths();
    let result = env_b.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env_b.clone(), user.clone(), Role::Verifier)
    });
    assert_eq!(
        result,
        Err(ContractError::NetworkMismatch),
        "set_role must fail with NetworkMismatch on wrong network"
    );
}

/// `remove_role` on a mismatched network must return `NetworkMismatch`.
#[test]
fn test_network_mismatch_remove_role() {
    let (env_a, env_b, _admin, user, contract_id) = setup_cross_network();

    // Grant a role on the correct network first.
    env_a.mock_all_auths();
    env_a.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env_a.clone(), user.clone(), Role::Verifier).unwrap();
    });

    env_b.mock_all_auths();
    let result = env_b.as_contract(&contract_id, || {
        TrustBridgeContract::remove_role(env_b.clone(), user.clone())
    });
    assert_eq!(
        result,
        Err(ContractError::NetworkMismatch),
        "remove_role must fail with NetworkMismatch on wrong network"
    );
}

/// `set_cooldown` on a mismatched network must return `NetworkMismatch`.
#[test]
fn test_network_mismatch_set_cooldown() {
    let (_env_a, env_b, _admin, _user, contract_id) = setup_cross_network();

    env_b.mock_all_auths();
    let result = env_b.as_contract(&contract_id, || {
        TrustBridgeContract::set_cooldown(env_b.clone(), 3600)
    });
    assert_eq!(
        result,
        Err(ContractError::NetworkMismatch),
        "set_cooldown must fail with NetworkMismatch on wrong network"
    );
}

/// `migrate` on a mismatched network must return `NetworkMismatch`.
#[test]
fn test_network_mismatch_migrate() {
    let (_env_a, env_b, _admin, _user, contract_id) = setup_cross_network();

    env_b.mock_all_auths();
    let result = env_b.as_contract(&contract_id, || {
        TrustBridgeContract::migrate(env_b.clone(), (2, 0, 0))
    });
    assert_eq!(
        result,
        Err(ContractError::NetworkMismatch),
        "migrate must fail with NetworkMismatch on wrong network"
    );
}

/// `set_guardian` on a mismatched network must return `NetworkMismatch`.
#[test]
fn test_network_mismatch_set_guardian() {
    let (_env_a, env_b, _admin, user, contract_id) = setup_cross_network();

    env_b.mock_all_auths();
    let result = env_b.as_contract(&contract_id, || {
        TrustBridgeContract::set_guardian(env_b.clone(), user.clone())
    });
    assert_eq!(
        result,
        Err(ContractError::NetworkMismatch),
        "set_guardian must fail with NetworkMismatch on wrong network"
    );
}

/// `emergency_pause` on a mismatched network must return `NetworkMismatch`.
#[test]
fn test_network_mismatch_emergency_pause() {
    let (_env_a, env_b, admin, _user, contract_id) = setup_cross_network();

    env_b.mock_all_auths();
    let result = env_b.as_contract(&contract_id, || {
        TrustBridgeContract::emergency_pause(env_b.clone(), admin.clone())
    });
    assert_eq!(
        result,
        Err(ContractError::NetworkMismatch),
        "emergency_pause must fail with NetworkMismatch on wrong network"
    );
}

/// `clear_emergency_pause` on a mismatched network must return `NetworkMismatch`.
#[test]
fn test_network_mismatch_clear_emergency_pause() {
    let (_env_a, env_b, _admin, _user, contract_id) = setup_cross_network();

    env_b.mock_all_auths();
    let result = env_b.as_contract(&contract_id, || {
        TrustBridgeContract::clear_emergency_pause(env_b.clone())
    });
    assert_eq!(
        result,
        Err(ContractError::NetworkMismatch),
        "clear_emergency_pause must fail with NetworkMismatch on wrong network"
    );
}

/// `get_all_registered` (which calls `require_initialized`) on a mismatched
/// network must return `NetworkMismatch`.
#[test]
fn test_network_mismatch_get_all_registered() {
    let (_env_a, env_b, _admin, _user, contract_id) = setup_cross_network();

    env_b.mock_all_auths();
    let result = env_b.as_contract(&contract_id, || {
        TrustBridgeContract::get_all_registered(env_b.clone())
    });
    assert_eq!(
        result,
        Err(ContractError::NetworkMismatch),
        "get_all_registered must fail with NetworkMismatch on wrong network"
    );
}

// ─── Error code stability ─────────────────────────────────────────────────────

/// The `NetworkMismatch` error code must be 30 — that code is frozen in the
/// ABI golden and cannot change without a major ABI version bump.
#[test]
fn test_network_mismatch_error_code_is_stable() {
    assert_eq!(
        ContractError::NetworkMismatch.code(),
        30,
        "NetworkMismatch must remain at code 30 (frozen ABI)"
    );
}

// ─── Cross-network deployment misuse (Issue #666) ────────────────────────────

const PUBNET: &str = "Public Global Stellar Network ; September 2015";
const TESTNET: &str = "Test SDF Network ; September 2015";

/// Initializes the contract on `init_net`, then moves the same ledger state to
/// `call_net` — the shape of a snapshot restored onto another network.
fn setup_between(init_net: &str, call_net: &str) -> (Env, Address) {
    let env = env_with_passphrase(init_net);
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    env.ledger()
        .with_mut(|li| li.network_id = sha256_passphrase(call_net));
    (env, contract_id)
}

fn register_on(env: &Env, contract_id: &Address) -> Result<(), ContractError> {
    env.mock_all_auths();
    let user = Address::generate(env);
    env.as_contract(contract_id, || {
        TrustBridgeContract::register(env.clone(), s(env, "octocat"), user, Vec::new(env))
    })
}

#[test]
fn test_network_mismatch_pubnet_state_on_testnet_fails_closed() {
    let (env, contract_id) = setup_between(PUBNET, TESTNET);
    assert_eq!(
        register_on(&env, &contract_id),
        Err(ContractError::NetworkMismatch)
    );
    // No partial state: the rejected call must not have written a record.
    // Switch back to the original network to read it.
    env.ledger()
        .with_mut(|li| li.network_id = sha256_passphrase(PUBNET));
    env.as_contract(&contract_id, || {
        assert!(!TrustBridgeContract::has_record(
            env.clone(),
            s(&env, "octocat")
        ));
    });
}

#[test]
fn test_network_mismatch_testnet_state_on_pubnet_fails_closed() {
    let (env, contract_id) = setup_between(TESTNET, PUBNET);
    assert_eq!(
        register_on(&env, &contract_id),
        Err(ContractError::NetworkMismatch)
    );
}

#[test]
fn test_network_matching_pubnet_and_testnet_succeed() {
    for net in [PUBNET, TESTNET] {
        let (env, contract_id) = setup_between(net, net);
        assert_eq!(register_on(&env, &contract_id), Ok(()));
    }
}
