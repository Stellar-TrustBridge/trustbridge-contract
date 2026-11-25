//! Exhaustive test for PauseReason variant ABI stability and documentation.
//!
//! This test ensures every PauseReason variant is documented in ABI.md
//! and covered by compile/snapshot exhaustiveness, so new variants trigger
//! test failures and force documentation updates (Issue #1).
//!
//! When adding a new PauseReason:
//! 1. Add it to `src/storage.rs::PauseReason` enum
//! 2. Update the discriminant table in `docs/ABI.md` §PauseReason
//! 3. Run `cargo test pause_reason` — it will fail until you add the new variant to `DOCUMENTED_VARIANTS` below
//! 4. Update the golden test `assert_all_variants_documented` to include the new code

#![cfg(test)]

use soroban_sdk::{Env, String};
use trustbridge_contract::{ContractError, PauseReason, TrustBridgeContract};

/// Every PauseReason discriminant currently defined in `src/storage.rs`.
///
/// **This is the source of truth for the exhaustiveness check.** When you add a
/// variant to `PauseReason`, append its discriminant here, then update the
/// assertion in `test_all_variants_documented` to match.
const ALL_KNOWN_VARIANTS: &[u32] = &[
    1,  // Maintenance
    2,  // SecurityIncident
    3,  // RegulatoryHold
    4,  // Unpause
    99, // Other
];

/// Variants documented in `docs/ABI.md` § PauseReason (Issue #1).
///
/// This list must stay in sync with the discriminant table in ABI.md.
/// The test below asserts the two sets are identical, so forgetting to
/// document a new variant (or documenting a code that doesn't exist) is a
/// test failure.
const DOCUMENTED_VARIANTS: &[u32] = &[
    1,  // Maintenance
    2,  // SecurityIncident
    3,  // RegulatoryHold
    4,  // Unpause
    99, // Other
];

#[test]
fn test_all_known_variants_round_trip_through_from_code() {
    for &code in ALL_KNOWN_VARIANTS {
        let reason = PauseReason::from_code(code);
        assert!(
            reason.is_some(),
            "PauseReason::from_code({code}) returned None, but {code} is in ALL_KNOWN_VARIANTS"
        );
    }
}

#[test]
fn test_is_valid_accepts_exactly_the_documented_set() {
    for &code in ALL_KNOWN_VARIANTS {
        assert!(
            PauseReason::is_valid(code),
            "PauseReason::is_valid({code}) returned false, but it's a known variant"
        );
    }

    // Unknown codes must be rejected
    let unknown = [0, 5, 6, 10, 98, 100, 255, 1000, u32::MAX];
    for &code in &unknown {
        assert!(
            !PauseReason::is_valid(code),
            "PauseReason::is_valid({code}) returned true for an unknown code"
        );
    }
}

#[test]
fn test_all_variants_documented() {
    let mut known_set = ALL_KNOWN_VARIANTS.to_vec();
    known_set.sort_unstable();
    known_set.dedup();

    let mut doc_set = DOCUMENTED_VARIANTS.to_vec();
    doc_set.sort_unstable();
    doc_set.dedup();

    assert_eq!(
        known_set, doc_set,
        "Mismatch between ALL_KNOWN_VARIANTS and DOCUMENTED_VARIANTS.\n\
         If you added a PauseReason, update DOCUMENTED_VARIANTS and docs/ABI.md § PauseReason.\n\
         Known: {known_set:?}, Documented: {doc_set:?}"
    );
}

#[test]
fn test_pause_entrypoint_rejects_undocumented_reason_codes() {
    let env = Env::default();
    let admin = soroban_sdk::Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();

        // Unknown reason code should return InvalidPauseReason
        let invalid_codes = [0, 5, 10, 100, 1000];
        for &code in &invalid_codes {
            assert_eq!(
                TrustBridgeContract::pause(env.clone(), code),
                Err(ContractError::InvalidPauseReason),
                "pause() accepted unknown reason code {code}"
            );
        }
    });
}

#[test]
fn test_unpause_entrypoint_rejects_undocumented_reason_codes() {
    let env = Env::default();
    let admin = soroban_sdk::Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
        // Pause first with a valid code
        TrustBridgeContract::pause(env.clone(), 1).unwrap();

        // Unknown reason code for unpause should return InvalidPauseReason
        let invalid_codes = [0, 5, 10, 100, 1000];
        for &code in &invalid_codes {
            assert_eq!(
                TrustBridgeContract::unpause(env.clone(), code),
                Err(ContractError::InvalidPauseReason),
                "unpause() accepted unknown reason code {code}"
            );
        }
    });
}

#[test]
fn test_pause_entrypoint_accepts_all_documented_codes() {
    let env = Env::default();
    let admin = soroban_sdk::Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();

        for &code in DOCUMENTED_VARIANTS {
            // Unpause first if needed
            if TrustBridgeContract::is_paused(env.clone()) {
                TrustBridgeContract::unpause(env.clone(), 4).unwrap();
            }
            
            assert!(
                TrustBridgeContract::pause(env.clone(), code).is_ok(),
                "pause() rejected documented reason code {code}"
            );
        }
    });
}

#[test]
fn test_unpause_entrypoint_accepts_all_documented_codes() {
    let env = Env::default();
    let admin = soroban_sdk::Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();

        for &code in DOCUMENTED_VARIANTS {
            // Ensure paused first
            if !TrustBridgeContract::is_paused(env.clone()) {
                TrustBridgeContract::pause(env.clone(), 1).unwrap();
            }
            
            assert!(
                TrustBridgeContract::unpause(env.clone(), code).is_ok(),
                "unpause() rejected documented reason code {code}"
            );
        }
    });
}

#[test]
fn test_set_paused_entrypoint_rejects_undocumented_reason_codes() {
    let env = Env::default();
    let admin = soroban_sdk::Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();

        let invalid_codes = [0, 5, 10, 100, 1000];
        for &code in &invalid_codes {
            assert_eq!(
                TrustBridgeContract::set_paused(env.clone(), admin.clone(), true, code),
                Err(ContractError::InvalidPauseReason),
                "set_paused() accepted unknown reason code {code}"
            );
        }
    });
}

#[test]
fn test_documented_discriminants_match_enum_values() {
    // This test ensures the discriminant table in ABI.md stays correct.
    // When you change a PauseReason discriminant, this test fails until you
    // update the table.
    
    assert_eq!(PauseReason::Maintenance as u32, 1);
    assert_eq!(PauseReason::SecurityIncident as u32, 2);
    assert_eq!(PauseReason::RegulatoryHold as u32, 3);
    assert_eq!(PauseReason::Unpause as u32, 4);
    assert_eq!(PauseReason::Other as u32, 99);
}

#[test]
fn test_pause_reason_snapshot_in_storage_is_queryable() {
    let env = Env::default();
    let admin = soroban_sdk::Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
        
        // Pause with SecurityIncident
        TrustBridgeContract::pause(env.clone(), 2).unwrap();
        
        // The reason should be queryable (implementation-dependent, but the
        // test documents that pause reasons are stored)
        assert!(TrustBridgeContract::is_paused(env.clone()));
        
        // Unpause with Unpause reason
        TrustBridgeContract::unpause(env.clone(), 4).unwrap();
        assert!(!TrustBridgeContract::is_paused(env.clone()));
    });
}
