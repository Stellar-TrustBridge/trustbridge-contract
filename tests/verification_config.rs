//! Integration tests pinning `VerificationConfig.threshold` as a **reserved**
//! field (Issue #589).
//!
//! `config_verification` stores `threshold`, `get_verification_config`
//! round-trips it, and `VerificationConfiguredEvent` mirrors it — but no
//! verify path (`verify`, `batch_verify`, `verify_with_oracle_proof`) reads
//! it: a single authorized verifier verifies in one call regardless of the
//! configured value. These tests lock that behaviour down so any future
//! enforcement is a deliberate, visible change rather than an accident, and
//! so the "no silent ignore" acceptance criterion of #589 stays satisfied.
//!
//! The semantics and operator guidance are documented in
//! `docs/ABI.md` (`config_verification`) and
//! `docs/SECURITY.md#verificationconfigthreshold-is-reserved-issue-589`.

#![cfg(test)]

use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::{Address, Env, String, Symbol, TryFromVal, Vec};
use trustbridge_contract::{Role, TrustBridgeContract};

/// Test fixture: an initialized contract where `admin` is the contract admin
/// and `verifier` holds `Role::Verifier`.
fn setup() -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    env.ledger().set_timestamp(1_000);
    let admin = Address::generate(&env);
    let verifier = Address::generate(&env);
    let user = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), verifier.clone(), Role::Verifier).unwrap();
    });

    (env, admin, verifier, user, contract_id)
}

fn username(env: &Env, name: &str) -> String {
    String::from_str(env, name)
}

/// Configures verification with the given `threshold`. Its own frame — one
/// `require_auth` per frame (recording-auth constraint).
fn configure(env: &Env, contract_id: &Address, admin: &Address, threshold: u32) {
    env.as_contract(contract_id, || {
        TrustBridgeContract::config_verification(
            env.clone(),
            admin.clone(),
            Symbol::new(env, "github_att"),
            0, // no expiry: these tests are about `threshold`, not Issue #218
            threshold,
        )
        .unwrap();
    });
}

// ── Round-trip (acceptance criterion: get_verification_config round-trips) ───

#[test]
fn test_threshold_round_trips_through_getter() {
    for threshold in [0u32, 2, u32::MAX] {
        let (env, admin, _verifier, _user, contract_id) = setup();
        configure(&env, &contract_id, &admin, threshold);

        env.as_contract(&contract_id, || {
            let cfg = TrustBridgeContract::get_verification_config(env.clone()).unwrap();
            assert_eq!(
                cfg.threshold, threshold,
                "threshold must round-trip verbatim"
            );
            assert_eq!(cfg.attestation, Symbol::new(&env, "github_att"));
            assert_eq!(cfg.expires_in, 0);
        });
    }
}

#[test]
fn test_unset_config_reports_none() {
    let (env, _admin, _verifier, _user, contract_id) = setup();

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_verification_config(env.clone()),
            None
        );
    });
}

// ── Reserved semantics: verify paths ignore the configured value ─────────────

#[test]
fn test_single_verifier_verify_ignores_configured_threshold() {
    let (env, admin, verifier, user, contract_id) = setup();
    configure(&env, &contract_id, &admin, 3);

    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "octocat"))
            .unwrap();
    });
    env.as_contract(&contract_id, || {
        assert!(
            TrustBridgeContract::is_verification_active(env.clone(), username(&env, "octocat")),
            "a single verifier must suffice no matter what `threshold` says"
        );
    });
}

#[test]
fn test_batch_verify_ignores_configured_threshold() {
    let (env, admin, verifier, user, contract_id) = setup();
    configure(&env, &contract_id, &admin, 3);

    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
        let mut usernames = Vec::new(&env);
        usernames.push_back(username(&env, "octocat"));
        let summary =
            TrustBridgeContract::batch_verify(env.clone(), verifier.clone(), usernames).unwrap();
        assert_eq!(
            summary.successful, 1,
            "one pass verifies even with threshold=3"
        );
        assert_eq!(summary.failed, 0);
    });
}

// ── Event accuracy (acceptance criterion: event with accurate semantics) ─────

#[test]
fn test_configured_threshold_is_emitted_verbatim_in_event() {
    let (env, admin, _verifier, _user, contract_id) = setup();
    configure(&env, &contract_id, &admin, 3);

    let topic = Symbol::new(&env, "verification_configured_event");
    let target = soroban_sdk::xdr::ScVal::try_from_val(&env, &topic.to_val()).unwrap();
    let filtered = env.events().all().filter_by_contract(&contract_id);
    let event = filtered
        .events()
        .iter()
        .find(|event| match &event.body {
            soroban_sdk::xdr::ContractEventBody::V0(v0) => v0.topics.first() == Some(&target),
        })
        .expect("VerificationConfiguredEvent must be published");

    let soroban_sdk::xdr::ContractEventBody::V0(v0) = &event.body;
    let soroban_sdk::xdr::ScVal::Map(Some(map)) = &v0.data else {
        panic!("event data must be a map");
    };
    let threshold_entry = map
        .0
        .iter()
        .find(|pair| {
            matches!(&pair.key, soroban_sdk::xdr::ScVal::Symbol(s) if s.to_string() == "threshold")
        })
        .expect("event data must carry the threshold field");
    assert_eq!(
        threshold_entry.val,
        soroban_sdk::xdr::ScVal::U32(3),
        "the event must mirror the configured threshold verbatim (informational — reserved)"
    );
}
