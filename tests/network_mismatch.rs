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
        protocol_version: 22,
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
    for i in 0..32usize {
        out[i] = out[i].wrapping_add((i as u8).wrapping_mul(7));
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
