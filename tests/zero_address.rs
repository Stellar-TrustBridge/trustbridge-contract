//! Zero-address guard tests against current Soroban host `mock_all_auths` behavior.
//!
//! Issue #300: `is_zero_address` exists because tests used to bypass auth.
//! Host updates can resurrect the hole. These tests ensure the guard remains
//! effective with current SDK mock auth APIs, and fail if someone removes it.
//!
//! The zero-address guard is documented in `docs/SECURITY.md` and `docs/ABI.md`.
//! On a live network, `require_auth` would reject the zero address (nobody
//! holds its private key), but `mock_all_auths` bypasses that check entirely.
//! The explicit `is_zero_address` guard is the only defense in test/sandbox
//! environments.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};
use trustbridge_contract::{ContractError, TrustBridgeContract};

/// The well-known zero/burn G-address: base32 encoding of an all-zero 32-byte
/// ed25519 public key with a valid checksum. No private key can exist for it.
const ZERO_ADDRESS_STRKEY: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";

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

// ── Core zero-address rejection in `register` ───────────────────────────────

/// `register` with the zero address as `stellar_address` must fail with
/// `ZeroAddress`, even when `mock_all_auths` is active.
///
/// This is the primary guard. On a live network, `require_auth` would already
/// reject this address, but `mock_all_auths` bypasses that check — the
/// explicit `is_zero_address` guard before `require_auth` is what actually
/// stops the registration in test and sandbox environments.
#[test]
fn test_zero_address_register_stellar_address_rejected_with_mock_all_auths() {
    let (env, _admin, contract_id) = setup();
    let zero_addr = Address::from_string(&s(&env, ZERO_ADDRESS_STRKEY));

    // Mock all auths — this bypasses the Soroban host's normal auth checks,
    // including the one that would reject the zero address on a live network.
    env.mock_all_auths();

    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::register(
            env.clone(),
            s(&env, "octocat"),
            zero_addr,
            Vec::new(&env),
        );

        assert_eq!(result, Err(ContractError::ZeroAddress));
    });
}

/// `register` with a valid `stellar_address` but the zero address in the
/// fallback list must fail with `ZeroAddress`.
///
/// Issue #287: fallback addresses are also checked before `require_auth`.
#[test]
fn test_zero_address_register_fallback_address_rejected_with_mock_all_auths() {
    let (env, _admin, contract_id) = setup();
    let valid_user = Address::generate(&env);
    let zero_addr = Address::from_string(&s(&env, ZERO_ADDRESS_STRKEY));

    let mut fallbacks = Vec::new(&env);
    fallbacks.push_back(zero_addr);

    env.mock_all_auths();

    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::register(
            env.clone(),
            s(&env, "octocat"),
            valid_user,
            fallbacks,
        );

        assert_eq!(result, Err(ContractError::ZeroAddress));
    });
}

/// `register_sponsored` with the zero address as `stellar_address` must fail
/// with `ZeroAddress`, even when `mock_all_auths` is active.
///
/// Sponsored registration has the same zero-address guard as regular
/// registration. The sponsor cannot bypass it.
#[test]
fn test_zero_address_register_sponsored_rejected_with_mock_all_auths() {
    let (env, _admin, contract_id) = setup();
    let sponsor = Address::generate(&env);
    let zero_addr = Address::from_string(&s(&env, ZERO_ADDRESS_STRKEY));

    env.mock_all_auths();

    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::register_sponsored(
            env.clone(),
            s(&env, "octocat"),
            zero_addr,
            sponsor,
        );

        assert_eq!(result, Err(ContractError::ZeroAddress));
    });
}

// ── Zero-address rejection in address rotation ──────────────────────────────

/// `request_address_rotation` with the zero address as `new_address` must fail
/// with `ZeroAddress`, even when `mock_all_auths` is active.
///
/// Issue #234: the rotation API also checks the new address before auth.
#[test]
fn test_zero_address_rotation_request_rejected_with_mock_all_auths() {
    let (env, _admin, contract_id) = setup();
    let valid_user = Address::generate(&env);
    let zero_addr = Address::from_string(&s(&env, ZERO_ADDRESS_STRKEY));

    env.mock_all_auths();

    // First register a valid user
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "octocat"),
            valid_user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    // Then attempt to rotate to zero address
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::request_address_rotation(
            env.clone(),
            s(&env, "octocat"),
            zero_addr,
        );

        assert_eq!(result, Err(ContractError::ZeroAddress));
    });
}

// ── Re-registration with different address (address update) ─────────────────

/// Re-registering an existing username to the zero address must fail with
/// `ZeroAddress`, even when `mock_all_auths` is active.
///
/// This is the address-update path: when a username is already registered, a
/// second `register` call with a different address requires both addresses to
/// sign. The zero-address guard runs before any auth, so the re-registration
/// to zero is blocked regardless of who signs.
#[test]
fn test_zero_address_reregistration_rejected_with_mock_all_auths() {
    let (env, _admin, contract_id) = setup();
    let original_user = Address::generate(&env);
    let zero_addr = Address::from_string(&s(&env, ZERO_ADDRESS_STRKEY));

    env.mock_all_auths();

    // First registration with a valid address
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "octocat"),
            original_user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    // Attempt re-registration to zero address — guard should block before auth
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::register(
            env.clone(),
            s(&env, "octocat"),
            zero_addr,
            Vec::new(&env),
        );

        assert_eq!(result, Err(ContractError::ZeroAddress));
    });
}

// ── Positive control: valid addresses still work with mock_all_auths ────────

/// Registering a valid (non-zero) address must succeed when `mock_all_auths`
/// is active. This is the positive control: it confirms `mock_all_auths` is
/// working and that the zero-address guard does not reject valid addresses.
#[test]
fn test_valid_address_register_succeeds_with_mock_all_auths() {
    let (env, _admin, contract_id) = setup();
    let valid_user = Address::generate(&env);

    env.mock_all_auths();

    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::register(
            env.clone(),
            s(&env, "octocat"),
            valid_user.clone(),
            Vec::new(&env),
        );

        assert!(result.is_ok(), "Valid address registration must succeed");

        // Confirm the record was actually written
        let record = TrustBridgeContract::get_address(env.clone(), s(&env, "octocat"));
        assert_eq!(record, Some(valid_user));
    });
}

// ── Helper function validation (is_address_zero) ────────────────────────────

/// The `is_address_zero` helper must correctly identify the zero address.
///
/// This is the public read that dashboards and indexers use to pre-validate
/// an address before asking a user to sign. It must agree with the internal
/// `is_zero_address` guard.
#[test]
fn test_is_address_zero_helper_identifies_zero_address() {
    let (env, _admin, contract_id) = setup();
    let zero_addr = Address::from_string(&s(&env, ZERO_ADDRESS_STRKEY));
    let valid_addr = Address::generate(&env);

    env.as_contract(&contract_id, || {
        assert!(
            TrustBridgeContract::is_address_zero(env.clone(), zero_addr),
            "is_address_zero must return true for the zero address"
        );

        assert!(
            !TrustBridgeContract::is_address_zero(env.clone(), valid_addr),
            "is_address_zero must return false for a valid address"
        );
    });
}

// ── Error code stability ─────────────────────────────────────────────────────

/// `ZeroAddress` error must map to code 16, as documented in `docs/ABI.md`.
///
/// Off-chain consumers (dashboard, indexer) rely on this numeric code to
/// classify failures without depending on the Rust enum layout.
#[test]
fn test_zero_address_error_code_is_stable() {
    assert_eq!(ContractError::ZeroAddress.code(), 16);
    assert_eq!(ContractError::from_code(16), Some(ContractError::ZeroAddress));
}

/// `ZeroAddress` error must be classified as Fatal (not retryable).
///
/// This is an input validation failure — retrying with the same zero address
/// will always fail. Off-chain retry logic should not loop on this error.
#[test]
fn test_zero_address_error_is_fatal_not_retryable() {
    use trustbridge_contract::ErrorCategory;

    assert_eq!(ContractError::ZeroAddress.category(), ErrorCategory::Fatal);
    assert!(!ContractError::ZeroAddress.is_retryable());
}

// ── Guard removal regression test ───────────────────────────────────────────

/// If someone removes the `is_zero_address` guard from `register`, this test
/// will fail: `mock_all_auths` will let the zero address through, and the
/// contract will write a record with the zero address as `stellar_address`.
///
/// This is the regression detection test. If it passes, the guard is still in
/// place. If it starts failing (because `register` succeeded instead of
/// returning `ZeroAddress`), someone removed the guard and opened the hole
/// that Issue #300 warns about.
#[test]
fn test_guard_removal_would_allow_zero_address_registration() {
    let (env, _admin, contract_id) = setup();
    let zero_addr = Address::from_string(&s(&env, ZERO_ADDRESS_STRKEY));

    env.mock_all_auths();

    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::register(
            env.clone(),
            s(&env, "guard-test"),
            zero_addr.clone(),
            Vec::new(&env),
        );

        // This assertion is the regression detector. If the guard is removed,
        // `result` will be `Ok(())` instead of `Err(ZeroAddress)`, and this
        // test will fail with a clear message.
        assert_eq!(
            result,
            Err(ContractError::ZeroAddress),
            "Zero-address registration must be rejected by the guard. \
             If this test fails with Ok(()), the is_zero_address guard was removed."
        );

        // Double-check that no record was written
        let record = TrustBridgeContract::get_address(env.clone(), s(&env, "guard-test"));
        assert_eq!(
            record, None,
            "Zero-address registration failure must not write a record"
        );
    });
}

// ── Multiple fallback addresses with one zero ───────────────────────────────

/// If the fallback list contains both valid addresses and the zero address,
/// the guard must still reject the entire call.
#[test]
fn test_zero_address_in_mixed_fallback_list_rejected() {
    let (env, _admin, contract_id) = setup();
    let valid_user = Address::generate(&env);
    let fallback1 = Address::generate(&env);
    let zero_addr = Address::from_string(&s(&env, ZERO_ADDRESS_STRKEY));
    let fallback2 = Address::generate(&env);

    let mut fallbacks = Vec::new(&env);
    fallbacks.push_back(fallback1);
    fallbacks.push_back(zero_addr); // Zero in the middle
    fallbacks.push_back(fallback2);

    env.mock_all_auths();

    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::register(
            env.clone(),
            s(&env, "octocat"),
            valid_user,
            fallbacks,
        );

        assert_eq!(result, Err(ContractError::ZeroAddress));
    });
}

// ── SECURITY.md accuracy validation ─────────────────────────────────────────

/// Validate the one-liner in `docs/SECURITY.md` about zero-address rejection.
///
/// From SECURITY.md:
/// > `stellar_address` must not be the well-known zero/burn address
/// > (`GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF`), or the
/// > call fails with `ZeroAddress` (code 16), checked before `require_auth`.
///
/// This test confirms:
/// 1. The strkey constant matches what's documented
/// 2. The error code is 16 as documented
/// 3. The check happens before auth (mock_all_auths does not bypass it)
#[test]
fn test_security_md_zero_address_documentation_is_accurate() {
    let (env, _admin, contract_id) = setup();
    let zero_addr = Address::from_string(&s(&env, ZERO_ADDRESS_STRKEY));

    // Confirm the strkey we're using matches the documented value
    assert_eq!(
        ZERO_ADDRESS_STRKEY,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
        "Zero address strkey must match SECURITY.md documentation"
    );

    // Confirm the error code matches the documented value
    assert_eq!(
        ContractError::ZeroAddress.code(),
        16,
        "ZeroAddress error code must be 16 as documented in SECURITY.md"
    );

    // Confirm the guard rejects before auth (mock_all_auths does not bypass)
    env.mock_all_auths();

    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::register(
            env.clone(),
            s(&env, "security-doc-test"),
            zero_addr,
            Vec::new(&env),
        );

        assert_eq!(
            result,
            Err(ContractError::ZeroAddress),
            "Zero-address registration must fail as documented in SECURITY.md"
        );
    });
}
