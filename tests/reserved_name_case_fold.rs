//! Fixtures for Issue #781: reserved-name rejection must be case-insensitive.
//!
//! `storage::is_reserved` folds case via `utils::eq_ignore_ascii_case`
//! before comparing against the reserved list (see `src/storage.rs`,
//! Issue #213). These fixtures lock that behavior at the public contract
//! boundary (`register`, `rename`, `add_reserved`) so a future change to
//! the comparison (e.g. switching to byte-exact matching, or adding
//! homoglyph normalization) cannot silently reopen a Wave brand
//! impersonation bypass.
//!
//! Run with: `cargo test --test reserved_name_case_fold`

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

use trustbridge_contract::{ContractError, TrustBridgeContract};

fn setup() -> (Env, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    (env, contract_id)
}

fn s(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

/// Every case variant of a reserved name must be rejected on registration,
/// not just the exact casing that was reserved.
#[test]
fn test_reserved_name_rejects_all_case_variants_on_register() {
    let (env, contract_id) = setup();
    let someone = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::add_reserved(env.clone(), s(&env, "stellar")).unwrap();

        for variant in ["stellar", "Stellar", "STELLAR", "StElLaR", "sTELLAr"] {
            let result = TrustBridgeContract::register(
                env.clone(),
                s(&env, variant),
                someone.clone(),
                Vec::new(&env),
            );
            assert_eq!(
                result,
                Err(ContractError::UsernameReserved),
                "variant {variant:?} should be rejected as reserved"
            );
        }
    });
}

/// Reserving a name in one case must block registration of every other
/// case variant, not only exact-match lookups.
#[test]
fn test_reserving_lowercase_blocks_uppercase_registration() {
    let (env, contract_id) = setup();
    let someone = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::add_reserved(env.clone(), s(&env, "wave")).unwrap();

        let result = TrustBridgeContract::register(
            env.clone(),
            s(&env, "WAVE"),
            someone.clone(),
            Vec::new(&env),
        );
        assert_eq!(result, Err(ContractError::UsernameReserved));
    });
}

/// `is_reserved` membership queries are case-insensitive regardless of the
/// casing used when the name was originally reserved.
#[test]
fn test_is_reserved_query_is_case_insensitive() {
    let (env, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::add_reserved(env.clone(), s(&env, "Admin")).unwrap();

        assert!(TrustBridgeContract::is_reserved(env.clone(), s(&env, "admin")));
        assert!(TrustBridgeContract::is_reserved(env.clone(), s(&env, "ADMIN")));
        assert!(TrustBridgeContract::is_reserved(env.clone(), s(&env, "Admin")));
        assert!(TrustBridgeContract::is_reserved(env.clone(), s(&env, "aDmIn")));
        assert!(!TrustBridgeContract::is_reserved(env.clone(), s(&env, "admin2")));
    });
}

/// Attempting to reserve a case variant of an already-reserved name must
/// fail with `AlreadyReserved`, not silently create a second entry that
/// could drift from the first.
#[test]
fn test_cannot_double_reserve_a_case_variant() {
    let (env, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::add_reserved(env.clone(), s(&env, "support")).unwrap();

        let result = TrustBridgeContract::add_reserved(env.clone(), s(&env, "SUPPORT"));
        assert_eq!(result, Err(ContractError::AlreadyReserved));
    });
}

/// Rename into a reserved name's case variant must also be rejected, since
/// `rename` consults the same `is_reserved` check as `register`.
#[test]
fn test_rename_into_reserved_case_variant_is_rejected() {
    let (env, contract_id) = setup();
    let alice = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), s(&env, "alice"), alice.clone(), Vec::new(&env))
            .unwrap();
        TrustBridgeContract::add_reserved(env.clone(), s(&env, "official")).unwrap();

        let result = TrustBridgeContract::rename(
            env.clone(),
            alice.clone(),
            s(&env, "alice"),
            s(&env, "OFFICIAL"),
        );
        assert_eq!(result, Err(ContractError::UsernameReserved));
    });
}

/// Removing a reserved name by a different case variant than it was
/// reserved under must still succeed (case-fold applies symmetrically to
/// removal), and once removed, every case variant becomes registrable.
#[test]
fn test_remove_reserved_by_case_variant_then_register_succeeds() {
    let (env, contract_id) = setup();
    let bob = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::add_reserved(env.clone(), s(&env, "Team")).unwrap();
        TrustBridgeContract::remove_reserved(env.clone(), s(&env, "team")).unwrap();

        assert!(!TrustBridgeContract::is_reserved(env.clone(), s(&env, "TEAM")));

        TrustBridgeContract::register(env.clone(), s(&env, "Team"), bob.clone(), Vec::new(&env))
            .unwrap();
    });
}
