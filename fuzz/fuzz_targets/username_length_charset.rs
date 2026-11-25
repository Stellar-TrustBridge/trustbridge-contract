#![no_main]

//! Fuzz target for GitHub username length/charset rules on register paths
//! (Issue #848).
//!
//! Structured input: the first byte selects a register path (even =
//! `register`, odd = `register_sponsored`); the remaining bytes are treated as
//! UTF-8 username bytes. Non-UTF-8 inputs are skipped because Soroban `String`
//! cannot represent them. The target asserts that validation and registration
//! agree, that no accepted username exceeds MAX_USERNAME_LEN, and that no
//! input panics. Seed corpus notes live in `fuzz/corpus/username_length_charset/NOTES.md`.

use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};
use trustbridge_contract::utils::{is_valid_github_username, MAX_USERNAME_LEN};
use trustbridge_contract::{ContractError, TrustBridgeContract};

fuzz_target!(|data: &[u8]| {
    let (path, bytes) = match data.split_first() {
        Some((path, bytes)) => (*path, bytes),
        None => return,
    };
    let Ok(text) = core::str::from_utf8(bytes) else {
        return;
    };
    let env = Env::default();
    let username = String::from_str(&env, text);
    let valid = is_valid_github_username(&username);
    if valid {
        assert!((1..=MAX_USERNAME_LEN).contains(&username.len()), "length invariant");
        assert!(bytes.is_ascii(), "charset invariant");
    }

    let admin = Address::generate(&env);
    let registrant = Address::generate(&env);
    let sponsor = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
        let result = if path % 2 == 0 {
            TrustBridgeContract::register(
                env.clone(),
                username.clone(),
                registrant.clone(),
                Vec::new(&env),
            )
        } else {
            TrustBridgeContract::register_sponsored(
                env.clone(),
                username.clone(),
                registrant.clone(),
                sponsor.clone(),
            )
        };
        if !valid {
            assert_eq!(result, Err(ContractError::InvalidUsername));
        } else {
            assert!(
                result.is_ok() || result == Err(ContractError::UsernameReserved),
                "valid username must register (or hit the reserved-name guard)"
            );
        }
    });
});
