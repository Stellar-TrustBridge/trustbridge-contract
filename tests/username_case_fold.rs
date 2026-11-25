//! Standalone verification for Issue #194 (case-fold GitHub usernames at the
//! storage-key layer).
//!
//! Kept as its own integration test file, independent of `tests/integration.rs`,
//! so it can be built and run on its own via
//! `cargo test --test username_case_fold` regardless of the state of that
//! other file.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

use trustbridge_contract::{ContractError, TrustBridgeContract};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    (env, admin, contract_id)
}

fn s(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

#[test]
fn test_username_case_variant_looks_up_the_same_record() {
    let (env, _admin, contract_id) = setup();
    let alice = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), s(&env, "Alice"), alice.clone(), Vec::new(&env))
            .unwrap();

        // A case variant lookup must hit the exact same record.
        let via_lower = TrustBridgeContract::get_address(env.clone(), s(&env, "alice")).unwrap();
        let via_upper = TrustBridgeContract::get_address(env.clone(), s(&env, "ALICE")).unwrap();
        let via_mixed = TrustBridgeContract::get_address(env.clone(), s(&env, "aLiCe")).unwrap();

        assert_eq!(via_lower.stellar_address, alice);
        assert_eq!(via_upper.stellar_address, alice);
        assert_eq!(via_mixed.stellar_address, alice);

        // Only one registration exists, not one per case variant.
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 1);

        assert!(TrustBridgeContract::has_record(
            env.clone(),
            s(&env, "alice")
        ));
        assert!(TrustBridgeContract::has_record(
            env.clone(),
            s(&env, "ALICE")
        ));
    });
}

#[test]
fn test_registering_a_case_variant_updates_the_existing_record_not_a_new_one() {
    let (env, _admin, contract_id) = setup();
    let alice = Address::generate(&env);

    // `mock_all_auths` authorizes every signature the invocation asks for,
    // including the existing owner's — this test's job is to prove the
    // second call lands on the *same* storage key (so a real deployment's
    // `old.stellar_address.require_auth()` guard is even reached) rather than
    // silently creating a second, independent registration for the same
    // GitHub login. The auth-rejection path itself is exercised by the
    // broader `remove`/`register` negative-auth suite.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), s(&env, "alice"), alice.clone(), Vec::new(&env))
            .unwrap();
    });

    let new_owner = Address::generate(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "ALICE"),
            new_owner.clone(),
            Vec::new(&env),
        )
        .unwrap();

        // Still one registration overall: the case variant updated the
        // existing entry rather than creating a second one.
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 1);

        let record = TrustBridgeContract::get_address(env.clone(), s(&env, "alice")).unwrap();
        assert_eq!(record.stellar_address, new_owner);
    });
}

#[test]
fn test_export_and_index_show_canonical_username_form() {
    let (env, _admin, contract_id) = setup();
    let alice = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), s(&env, "Alice"), alice.clone(), Vec::new(&env))
            .unwrap();

        let page = TrustBridgeContract::get_registered_paginated(env.clone(), None, 10).unwrap();
        assert_eq!(page.records.len(), 1);
        let (stored_username, _) = page.records.get(0).unwrap();
        assert_eq!(stored_username, s(&env, "alice"));
    });
}

#[test]
fn test_remove_by_case_variant_removes_the_canonical_record() {
    let (env, admin, contract_id) = setup();
    let alice = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), s(&env, "alice"), alice.clone(), Vec::new(&env))
            .unwrap();

        // Remove using a different case than the one used to register.
        TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, "ALICE")).unwrap();

        assert!(!TrustBridgeContract::has_record(
            env.clone(),
            s(&env, "alice")
        ));
        assert!(!TrustBridgeContract::has_record(
            env.clone(),
            s(&env, "Alice")
        ));
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 0);
    });
}

// ── Locale-specific mapping tests (Issue #766) ──────────────────────────────

/// Test that Turkish/Azeri dotted capital I (U+0130) and dotless small i (U+0131)
/// cannot be registered and cannot impersonate standard ASCII usernames.
///
/// In Turkish (`tr_TR`), uppercase 'i' is 'İ' (U+0130) and lowercase 'I' is 'ı' (U+0131).
/// A locale-dependent system could mistakenly fold 'ALICE' to 'alıce' or treat 'alİce'
/// as 'alice'. The contract is strictly ASCII-true and rejects all non-ASCII.
#[test]
fn test_turkish_locale_dotless_and_dotted_i_rejected_and_do_not_collide() {
    let (env, _admin, contract_id) = setup();
    let alice = Address::generate(&env);
    let attacker = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // 1. Happy path: Register standard ASCII "ALICE" (contains uppercase 'I').
        TrustBridgeContract::register(env.clone(), s(&env, "ALICE"), alice.clone(), Vec::new(&env))
            .unwrap();

        // 2. Failure path 1: Register with Turkish dotted capital I ('alİce', U+0130).
        let res_dotted = TrustBridgeContract::register(
            env.clone(),
            s(&env, "al\u{0130}ce"),
            attacker.clone(),
            Vec::new(&env),
        );
        assert_eq!(
            res_dotted.unwrap_err(),
            ContractError::InvalidUsername,
            "Turkish dotted capital I must be rejected at registration"
        );

        // 3. Failure path 2: Register with Turkish dotless small i ('alıce', U+0131).
        let res_dotless = TrustBridgeContract::register(
            env.clone(),
            s(&env, "al\u{0131}ce"),
            attacker.clone(),
            Vec::new(&env),
        );
        assert_eq!(
            res_dotless.unwrap_err(),
            ContractError::InvalidUsername,
            "Turkish dotless small i must be rejected at registration"
        );

        // 4. Lookups using Turkish variants do NOT resolve to the ASCII "alice" record.
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), s(&env, "al\u{0130}ce")),
            None,
            "Turkish dotted I must not resolve to ASCII record"
        );
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), s(&env, "al\u{0131}ce")),
            None,
            "Turkish dotless i must not resolve to ASCII record"
        );
        assert!(!TrustBridgeContract::has_record(
            env.clone(),
            s(&env, "al\u{0130}ce")
        ));
        assert!(!TrustBridgeContract::has_record(
            env.clone(),
            s(&env, "al\u{0131}ce")
        ));

        // 5. Lookups using standard ASCII casing (both lower and upper) hit the record.
        let record_lower = TrustBridgeContract::get_address(env.clone(), s(&env, "alice")).unwrap();
        assert_eq!(record_lower.stellar_address, alice);
        let record_upper = TrustBridgeContract::get_address(env.clone(), s(&env, "ALICE")).unwrap();
        assert_eq!(record_upper.stellar_address, alice);

        // Still exactly one registration.
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 1);
    });
}

/// Test that German eszett and Greek sigma variants cannot impersonate ASCII usernames.
///
/// In Unicode full case folding:
/// - German 'ß' (U+00DF) folds to "ss" (length expansion).
/// - Greek 'Σ' (U+03A3) folds to 'σ' (U+03C3) or 'ς' (U+03C2).
/// In TrustBridge, only ASCII bytes are accepted, preventing length change or alias attacks.
#[test]
fn test_german_eszett_and_greek_sigma_rejected_and_cannot_impersonate() {
    let (env, _admin, contract_id) = setup();
    let user = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // 1. Happy path: Register standard ASCII "GROSS".
        TrustBridgeContract::register(env.clone(), s(&env, "GROSS"), user.clone(), Vec::new(&env))
            .unwrap();

        // 2. Failure path 1: German eszett 'groß' (U+00DF) is rejected.
        let res_eszett = TrustBridgeContract::register(
            env.clone(),
            s(&env, "gro\u{00DF}"),
            user.clone(),
            Vec::new(&env),
        );
        assert_eq!(
            res_eszett.unwrap_err(),
            ContractError::InvalidUsername,
            "German eszett must be rejected"
        );

        // 3. Failure path 2: Capital eszett 'GROẞ' (U+1E9E) is rejected.
        let res_cap_eszett = TrustBridgeContract::register(
            env.clone(),
            s(&env, "GRO\u{1E9E}"),
            user.clone(),
            Vec::new(&env),
        );
        assert_eq!(
            res_cap_eszett.unwrap_err(),
            ContractError::InvalidUsername,
            "German capital eszett must be rejected"
        );

        // 4. Failure path 3: Greek sigma variants are rejected.
        let res_sigma = TrustBridgeContract::register(
            env.clone(),
            s(&env, "\u{03A3}mith"),
            user.clone(),
            Vec::new(&env),
        );
        assert_eq!(
            res_sigma.unwrap_err(),
            ContractError::InvalidUsername,
            "Greek capital sigma must be rejected"
        );

        // 5. Querying 'groß' does not find the ASCII 'gross' record.
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), s(&env, "gro\u{00DF}")),
            None
        );
        assert!(!TrustBridgeContract::has_record(
            env.clone(),
            s(&env, "gro\u{00DF}")
        ));

        // Canonical form is pure ASCII "gross".
        let record = TrustBridgeContract::get_address(env.clone(), s(&env, "gross")).unwrap();
        assert_eq!(record.stellar_address, user);
    });
}

/// Test that letter-like symbols, ligatures, and compatibility variants are rejected.
#[test]
fn test_letterlike_symbols_and_compatibility_variants_rejected() {
    let (env, _admin, contract_id) = setup();
    let user = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Kelvin sign 'K' (U+212A) looks like 'K'
        let res_kelvin = TrustBridgeContract::register(
            env.clone(),
            s(&env, "\u{212A}ate"),
            user.clone(),
            Vec::new(&env),
        );
        assert_eq!(res_kelvin.unwrap_err(), ContractError::InvalidUsername);

        // Latin small letter long s 'ſ' (U+017F) looks like 's' or 'f'
        let res_long_s = TrustBridgeContract::register(
            env.clone(),
            s(&env, "alif\u{017F}e"),
            user.clone(),
            Vec::new(&env),
        );
        assert_eq!(res_long_s.unwrap_err(), ContractError::InvalidUsername);

        // Latin ligature 'fi' (U+FB01)
        let res_fi = TrustBridgeContract::register(
            env.clone(),
            s(&env, "\u{FB01}le"),
            user.clone(),
            Vec::new(&env),
        );
        assert_eq!(res_fi.unwrap_err(), ContractError::InvalidUsername);

        // Fullwidth Latin letter 'Ａ' (U+FF21)
        let res_fullwidth = TrustBridgeContract::register(
            env.clone(),
            s(&env, "\u{FF21}lice"),
            user.clone(),
            Vec::new(&env),
        );
        assert_eq!(res_fullwidth.unwrap_err(), ContractError::InvalidUsername);
    });
}

/// Test that ASCII case folding is strictly deterministic, byte-length preserving,
/// and consistent across all alphanumeric and allowed separator characters.
#[test]
fn test_ascii_folding_is_deterministic_and_byte_length_invariant() {
    let (env, _admin, contract_id) = setup();
    let user = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let mixed = "A1-b2_C3-Z9";
        let lower = "a1-b2_c3-z9";

        TrustBridgeContract::register(env.clone(), s(&env, mixed), user.clone(), Vec::new(&env))
            .unwrap();

        // Stored page reports exact canonical form
        let page = TrustBridgeContract::get_registered_paginated(env.clone(), None, 10).unwrap();
        assert_eq!(page.records.len(), 1);
        let (stored_name, _) = page.records.get(0).unwrap();
        assert_eq!(stored_name, s(&env, lower));
        assert_eq!(stored_name.len(), s(&env, mixed).len());
    });
}
