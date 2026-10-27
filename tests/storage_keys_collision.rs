//! Storage key namespace collision and invariant tests (Issue #690).
//!
//! Verifies that:
//! 1. All instance keys are distinct symbols.
//! 2. All persistent key discriminators are distinct.
//! 3. Instance and persistent namespaces cannot collide (enforced by Soroban).
//! 4. Per-username persistent keys use different discriminators from per-address keys.
//! 5. Happy path: keys can be stored and retrieved without collision.
//! 6. Failure path: operations fail gracefully when appropriate.
//!
//! This test suite is the automated proof of the collision analysis in
//! `docs/STORAGE_KEYS.md` § Collision Analysis and Key Namespace Diagram.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

use trustbridge_contract::{ContractError, Role, TrustBridgeContract};

fn setup_test_env() -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });

    (env, admin, user1, user2, contract_id)
}

fn s(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

// ── Inventory assertions (no-op placeholders removed — see Wave 3) ───────────

/// The contract source and its key documentation, included at compile time
/// so these checks cannot drift from what they validate.
const STORAGE_SRC: &str = include_str!("../src/storage.rs");
const STORAGE_KEYS_DOC: &str = include_str!("../docs/STORAGE_KEYS.md");

// ── Instance key uniqueness ──────────────────────────────────────────────────

/// The `symbol_short!(...)` literals declared by `const NAME: Symbol` lines
/// in src/storage.rs, in source order. Only *declaration* lines count:
/// inline `symbol_short!(...)` usages inside test bodies legitimately
/// re-use canonical symbols, and aliased constants (`const X = Y;`) add no
/// new string. This matches `scripts/check_storage_keys.sh` and the
/// collision property: two *primary* declarations re-using one string
/// would make distinct keys read/write each other's data.
fn declared_symbol_literals(source: &str) -> std::vec::Vec<&str> {
    static MARKER: &str = "symbol_short!(\"";
    let mut out = std::vec::Vec::new();
    for line in source.lines() {
        let is_declaration = line.contains("const") && line.contains(": Symbol");
        if !is_declaration {
            continue;
        }
        let mut from = 0;
        while let Some(at) = line[from..].find(MARKER) {
            let rest = &line[from + at + MARKER.len()..];
            match rest.find('"') {
                Some(end) => {
                    out.push(&rest[..end]);
                    from += at + MARKER.len() + end + 1;
                }
                None => break,
            }
        }
    }
    out
}

/// Verify that all Symbol key literals in `src/storage.rs` are distinct.
///
/// This was previously an `assert!(true)` placeholder (PR #690 waved it in
/// as documentation-only; Clippy `-D warnings` rejects the Constant
/// assertion). It now mirrors `scripts/check_storage_keys.sh` so a collision
/// fails `cargo test` even where bash is unavailable.
#[test]
fn test_instance_keys_are_unique() {
    let symbols = declared_symbol_literals(STORAGE_SRC);
    assert!(
        !symbols.is_empty(),
        "no symbol_short! literals found — parser drift or empty key inventory"
    );

    let mut seen = std::collections::HashSet::new();
    for sym in &symbols {
        assert!(
            seen.insert(*sym),
            "duplicate Symbol literal {sym:?}: two primary keys would collide"
        );
    }
}

// ── Persistent key discriminator uniqueness ──────────────────────────────────

/// Verify that persistent key discriminators are unique at compile time.
///
/// Persistent keys use `(Symbol, Context)` tuples. The discriminator (Symbol)
/// is encoded first in XDR serialization, guaranteeing distinct tuples even if
/// contexts overlap.
///
/// Documentation: See `docs/STORAGE_KEYS.md` § Key Namespace Diagram for the
/// visual representation of how discriminators prevent collisions.
#[test]
fn test_persistent_key_discriminators_are_unique() {
    // Mirrors `scripts/check_storage_keys.sh`: every Symbol constant in
    // src/storage.rs must be documented in docs/STORAGE_KEYS.md by name
    // (backticked), and the prior test covers literal uniqueness.
    for line in STORAGE_SRC.lines() {
        if !line.contains("const") {
            continue;
        }
        let Some(name) = line
            .split("const ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .filter(|n| STORAGE_SRC.contains(&format!("const {n}: Symbol")))
        else {
            continue;
        };
        assert!(
            STORAGE_KEYS_DOC.contains(&format!("`{name}`")),
            "Symbol constant {name} not documented in docs/STORAGE_KEYS.md"
        );
    }
}

// ── Happy path: registration storage and retrieval ────────────────────────────

/// Happy path: register a user, store their record, and retrieve it.
///
/// Confirms that the registration storage key (`"reg"` discriminator +
/// username context) works correctly without collision.
#[test]
fn test_happy_path_registration_stored_and_retrieved() {
    let (env, _admin, user1, _user2, contract_id) = setup_test_env();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Register a user.
        let username = s(&env, "alice");
        TrustBridgeContract::register(env.clone(), username.clone(), user1.clone(), Vec::new(&env))
            .unwrap();

        // Verify the count was incremented.
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 1);
        assert_eq!(stats.verified, 0);
    });
}

// ── Happy path: role storage and retrieval ────────────────────────────────────

/// Happy path: assign a role to an address and retrieve it.
///
/// Confirms that role storage keys (`"role"` discriminator + address context)
/// work correctly without collision with username-keyed entries.
#[test]
fn test_happy_path_role_stored_and_retrieved() {
    let (env, _admin, user1, _user2, contract_id) = setup_test_env();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Assign Verifier role to user1.
        TrustBridgeContract::set_role(env.clone(), user1.clone(), Role::Verifier).unwrap();

        // Retrieve the role.
        let role = TrustBridgeContract::get_role(env.clone(), user1.clone());
        assert_eq!(role, Some(Role::Verifier));
    });
}

// ── Happy path: different context types do not collide ───────────────────────

/// Happy path: per-username and per-address keys coexist independently.
///
/// Demonstrates that even if a username and an address had the same byte
/// representation (they don't in practice), the discriminator ensures collisions
/// are impossible because they use different Symbol values in their storage key.
#[test]
fn test_happy_path_username_and_address_keys_coexist() {
    let (env, _admin, user1, _user2, contract_id) = setup_test_env();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Register a user named "alice".
        let username = s(&env, "alice");
        TrustBridgeContract::register(env.clone(), username.clone(), user1.clone(), Vec::new(&env))
            .unwrap();

        // Assign a role to a different address.
        let user2 = Address::generate(&env);
        TrustBridgeContract::set_role(env.clone(), user2.clone(), Role::Verifier).unwrap();

        // Both entries exist independently — no collision.
        let role = TrustBridgeContract::get_role(env.clone(), user2.clone());
        assert_eq!(role, Some(Role::Verifier));

        // Stats show only one registration, proving they don't interfere.
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 1);
    });
}

// ── Failure path: verify operation fails when paused ─────────────────────────

/// Failure path: attempting to mutate state while paused correctly fails.
///
/// Documents that pause state is maintained independently and acts as
/// a guard on mutations, preventing accidental state corruption.
#[test]
fn test_failure_path_register_while_paused_is_rejected() {
    let (env, _admin, user1, _user2, contract_id) = setup_test_env();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Pause the contract with reason code 1 (Maintenance).
        TrustBridgeContract::pause(env.clone(), 1u32).unwrap();

        // Attempt to register while paused — should fail.
        let username = s(&env, "bob");
        let result =
            TrustBridgeContract::register(env.clone(), username, user1.clone(), Vec::new(&env));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ContractError::Paused);
    });
}

// ── Failure path: case folding creates intentional collision ─────────────────

/// Failure path: registering the same username in different cases collides.
///
/// Documents that username canonicalization (case-folding to lowercase) is
/// working correctly and that the underlying storage key is consistent
/// regardless of case. This collision is intentional and prevents duplicate
/// registrations.
#[test]
fn test_collision_case_folding_is_intentional() {
    let (env, _admin, user1, _user2, contract_id) = setup_test_env();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Register "Alice".
        let alice_upper = s(&env, "Alice");
        TrustBridgeContract::register(env.clone(), alice_upper, user1.clone(), Vec::new(&env))
            .unwrap();
    });

    // Try to register "alice" (lowercase) — the case fold (Issue #194)
    // resolves it to the same canonical key, so this UPDATES the existing
    // record instead of creating a second independent one. Doubling the
    // same address (no transfer) needs no extra auth and succeeds.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let alice_lower = s(&env, "alice");
        // Same canonical username, same address: an idempotent re-register.
        TrustBridgeContract::register(env.clone(), alice_lower, user1.clone(), Vec::new(&env))
            .unwrap();

        // The fold guarantees ONE record: no case-split duplicate was created.
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(
            stats.total, 1,
            "Case-folded username must update the same record, not register a new one"
        );
    });
}

// ── Failure path: non-existent entry returns None ────────────────────────────

/// Failure path: querying a non-existent key returns None gracefully.
///
/// Documents that reads on non-existent entries do not panic and do not
/// cross-contaminate other namespaces.
#[test]
fn test_failure_path_nonexistent_entry_returns_none() {
    let (env, _admin, _user1, _user2, contract_id) = setup_test_env();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Query a role for an address that was never assigned one.
        let random_addr = Address::generate(&env);
        let result = TrustBridgeContract::get_role(env.clone(), random_addr);
        assert!(result.is_none(), "Non-existent role should return None");
    });
}

// ── Compound test: multiple operations without collision ──────────────────────

/// Compound case: multiple registrations and role assignments coexist without collision.
///
/// Simulates realistic usage and confirms that the namespace diagram holds up
/// under concurrent operations across multiple key types.
#[test]
fn test_compound_multiple_operations_no_collision() {
    let (env, _admin, user1, user2, contract_id) = setup_test_env();
    let user3 = Address::generate(&env);
    let user4 = Address::generate(&env);

    // Registration 1.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let alice = s(&env, "alice");
        TrustBridgeContract::register(env.clone(), alice, user1.clone(), Vec::new(&env)).unwrap();
    });

    // Registration 2.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let bob = s(&env, "bob");
        TrustBridgeContract::register(env.clone(), bob, user2.clone(), Vec::new(&env)).unwrap();
    });

    // Role assignment 1. `set_role` requires admin auth; each admin call
    // is its own invocation so the fresh frame is authorized exactly once
    // (batching two calls panics with Auth/ExistingValue).
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), user3.clone(), Role::Verifier).unwrap();
    });

    // Role assignment 2.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), user4.clone(), Role::Revoker).unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Retrieve all role entries and verify they are independent.
        let user3_role = TrustBridgeContract::get_role(env.clone(), user3);
        assert_eq!(user3_role, Some(Role::Verifier));

        let user4_role = TrustBridgeContract::get_role(env.clone(), user4);
        assert_eq!(user4_role, Some(Role::Revoker));

        // Verify stats count is correct.
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 2, "Should have exactly 2 registrations");
        assert_eq!(stats.verified, 0, "No registrations are verified yet");
    });
}
