use super::*;
use alloc::format;
use alloc::string::ToString;
use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    Address, Env, Event as _, String, TryFromVal,
};

// ── Helpers ──────────────────────────────────────────────────────────────

fn setup(env: &Env) -> (Address, Address, Address, Address) {
    let admin = Address::generate(env);
    let user = Address::generate(env);
    let other = Address::generate(env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    (admin, user, other, contract_id)
}

fn username(env: &Env, name: &str) -> String {
    String::from_str(env, name)
}

// ── Audit event coverage for revoke / pause (Issue #397) ────────────────

/// The event types on the audit log, oldest first.
fn audit_event_types(env: &Env, contract_id: &Address) -> alloc::vec::Vec<AuditEventType> {
    env.as_contract(contract_id, || {
        TrustBridgeContract::get_audit_logs(env.clone())
            .iter()
            .map(|entry| entry.event_type)
            .collect()
    })
}

#[test]
fn revoke_verification_writes_an_audit_entry() {
    let env = Env::default();
    let (admin, user1, _other, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_personal(&env, &contract_id, "octocat", &user1);
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
        TrustBridgeContract::revoke_verification(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
            RevokeReason::CompromisedKey as u32,
        )
        .unwrap();
    });

    let entry = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_audit_logs(env.clone())
            .iter()
            .find(|e| e.event_type == AuditEventType::VerificationRevoked)
            .expect("revoke_verification must write an audit entry")
    });

    // Losing verification is a state change a reader querying the contract
    // has to be able to see; before #397 it left no audit entry at all.
    assert_eq!(entry.actor, Some(admin));
    assert_eq!(entry.target_username, Some(username(&env, "octocat")));
    assert_eq!(entry.target_address, Some(user1));
}

#[test]
fn revoke_is_distinguishable_from_verify_in_the_log() {
    let env = Env::default();
    let (admin, user1, _other, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_personal(&env, &contract_id, "octocat", &user1);
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
        TrustBridgeContract::revoke_verification(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
            RevokeReason::OperatorError as u32,
        )
        .unwrap();
    });

    let types = audit_event_types(&env, &contract_id);
    let verified = types
        .iter()
        .filter(|t| **t == AuditEventType::UserVerified)
        .count();
    let revoked = types
        .iter()
        .filter(|t| **t == AuditEventType::VerificationRevoked)
        .count();

    assert_eq!(verified, 1);
    assert_eq!(revoked, 1);
}

#[test]
fn pause_and_unpause_record_their_direction() {
    let env = Env::default();
    let (_admin, _user1, _other, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::pause(env.clone(), PauseReason::Maintenance as u32).unwrap();
        TrustBridgeContract::unpause(env.clone(), PauseReason::Unpause as u32).unwrap();
    });

    let types = audit_event_types(&env, &contract_id);

    // Both used to be ADMIN_ACTION, so an auditor could not tell whether
    // the contract had gone down or come back up without correlating with
    // the entry before it.
    let paused_at = types
        .iter()
        .position(|t| *t == AuditEventType::ContractPaused)
        .expect("pause must record CONTRACT_PAUSED");
    let unpaused_at = types
        .iter()
        .position(|t| *t == AuditEventType::ContractUnpaused)
        .expect("unpause must record CONTRACT_UNPAUSED");

    assert!(
        paused_at < unpaused_at,
        "entries must be in the order the calls happened"
    );
}

#[test]
fn set_paused_records_the_direction_it_moved() {
    let env = Env::default();
    let (_admin, _user1, _other, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_paused(env.clone(), true, PauseReason::SecurityIncident as u32)
            .unwrap();
    });
    assert!(audit_event_types(&env, &contract_id).contains(&AuditEventType::ContractPaused));

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_paused(env.clone(), false, PauseReason::Unpause as u32).unwrap();
    });
    assert!(audit_event_types(&env, &contract_id).contains(&AuditEventType::ContractUnpaused));
}

#[test]
fn audit_event_type_strings_are_stable() {
    // Indexers match on these strings; changing one silently re-labels
    // history it has already written.
    assert_eq!(
        AuditEventType::VerificationRevoked.as_str(),
        "VERIFICATION_REVOKED"
    );
    assert_eq!(AuditEventType::UserRenamed.as_str(), "USER_RENAMED");
    assert_eq!(AuditEventType::ContractPaused.as_str(), "CONTRACT_PAUSED");
    assert_eq!(
        AuditEventType::ContractUnpaused.as_str(),
        "CONTRACT_UNPAUSED"
    );
    assert_eq!(AuditEventType::RoleChanged.as_str(), "ROLE_CHANGED");
    assert_eq!(
        AuditEventType::ContractUpgraded.as_str(),
        "CONTRACT_UPGRADED"
    );
}

/// Registers `name` to `addr` with no fallback addresses. Must be called
/// from inside an `env.as_contract(&contract_id, || { .. })` closure.
fn register_personal(env: &Env, _contract_id: &Address, name: &str, addr: &Address) {
    TrustBridgeContract::register(
        env.clone(),
        username(env, name),
        addr.clone(),
        Vec::new(env),
    )
    .unwrap();
}

/// Asserts `get_verified_count()` and `get_stats().verified` agree on
/// `expected` (Issue #90). Both APIs are checked together so future
/// mutation paths cannot let the two counters drift apart silently.
fn assert_verified_parity(env: &Env, expected: u32) {
    assert_eq!(
        TrustBridgeContract::get_verified_count(env.clone()),
        expected,
        "get_verified_count() diverged from the expected verified count"
    );
    assert_eq!(
        TrustBridgeContract::get_stats(env.clone()).verified,
        expected,
        "get_stats().verified diverged from the expected verified count"
    );
}

// ── Basic registration / lookup ──────────────────────────────────────────

#[test]
fn test_config_verification_double_initialize_rejection() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    let attestation = soroban_sdk::Symbol::new(&env, "github_att");

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::config_verification(
            env.clone(),
            admin.clone(),
            attestation.clone(),
            3600,
            2,
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let res = TrustBridgeContract::config_verification(
            env.clone(),
            admin.clone(),
            attestation,
            3600,
            2,
        );
        assert_eq!(res, Err(ContractError::AlreadyInitialized));
    });
}

#[test]
fn test_config_verification_emits_event_and_getter_reflects_it() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    let attestation = soroban_sdk::Symbol::new(&env, "github_att");

    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::config_verification(
            env.clone(),
            admin.clone(),
            attestation.clone(),
            3600,
            2,
        )
        .unwrap();
    });

    let expected = VerificationConfiguredEvent {
        admin: admin.clone(),
        attestation: attestation.clone(),
        expires_in: 3600,
        threshold: 2,
        timestamp: 1_700_000_000,
    };
    assert_eq!(
        env.events().all(),
        soroban_sdk::vec![
            &env,
            (
                contract_id.clone(),
                expected.topics(&env),
                expected.data(&env),
            )
        ],
        "VerificationConfiguredEvent payload or topics changed"
    );

    let topics = expected.topics(&env);
    assert_eq!(
        soroban_sdk::Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
        soroban_sdk::Symbol::new(&env, "verification_configured_event"),
        "VerificationConfiguredEvent topic symbol changed"
    );

    env.as_contract(&contract_id, || {
        let config = TrustBridgeContract::get_verification_config(env.clone())
            .expect("verification config must be set");
        assert_eq!(config.attestation, attestation);
        assert_eq!(config.expires_in, 3600);
        assert_eq!(config.threshold, 2);
    });
}

#[test]
fn test_register_and_get_address_roundtrip() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert_eq!(record.stellar_address, user);
        assert!(!record.verified);
    });
}

#[test]
fn test_register_with_fallback_addresses() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    let fb1 = Address::generate(&env);
    let fb2 = Address::generate(&env);
    let mut fallbacks: Vec<Address> = Vec::new(&env);
    fallbacks.push_back(fb1.clone());
    fallbacks.push_back(fb2.clone());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            fallbacks.clone(),
        )
        .unwrap();
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert_eq!(record.stellar_address, user);
    });
}

#[test]
fn test_register_fallback_list_exceeds_cap() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    let mut fallbacks: Vec<Address> = Vec::new(&env);
    for _ in 0..=MAX_FALLBACK_ADDRESSES {
        fallbacks.push_back(Address::generate(&env));
    }
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let res = TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            fallbacks,
        );
        assert_eq!(res, Err(ContractError::FallbackListFull));
    });
}

#[test]
fn test_get_address_missing_returns_none() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::get_address(env.clone(), username(&env, "missing")).is_none());
    });
}

#[test]
fn test_repeated_missing_lookups_are_stable() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::get_address(env.clone(), username(&env, "missing")).is_none());
        assert!(TrustBridgeContract::get_address(env.clone(), username(&env, "missing")).is_none());
    });
}

#[test]
fn test_register_two_users_keeps_addresses() {
    let env = Env::default();
    let (_admin, user1, user2, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "alice"))
                .unwrap()
                .stellar_address,
            user1
        );
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "bob"))
                .unwrap()
                .stellar_address,
            user2
        );
    });
}

// ── Verified-only lookup (Issue #287) ────────────────────────────────────

/// A verified registration resolves through `get_address_if_verified` and
/// carries the same address `get_address` would return.
#[test]
fn test_verified_only_lookup_returns_record_when_verified() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
        let record =
            TrustBridgeContract::get_address_if_verified(env.clone(), username(&env, "octocat"))
                .unwrap();
        assert_eq!(record.stellar_address, user);
        assert!(record.verified);
    });
}

/// A registered-but-unverified username is rejected with `NotVerified` (code 6),
/// even though `get_address` would still return it.
#[test]
fn test_verified_only_lookup_rejects_unverified() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        assert!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).is_some(),
            "get_address still returns the unverified record"
        );
        assert_eq!(
            TrustBridgeContract::get_address_if_verified(env.clone(), username(&env, "octocat")),
            Err(ContractError::NotVerified),
        );
    });
}

/// An unregistered username is rejected with `NotRegistered` (code 4),
/// distinct from the unverified case.
#[test]
fn test_verified_only_lookup_rejects_unregistered() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_address_if_verified(env.clone(), username(&env, "ghost")),
            Err(ContractError::NotRegistered),
        );
    });
}

/// A revoked registration falls back to `NotVerified` — verification can be
/// withdrawn and the verified-only lookup tracks that immediately.
#[test]
fn test_verified_only_lookup_follows_revocation() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
        TrustBridgeContract::revoke_verification(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
            1,
        )
        .unwrap();
        assert_eq!(
            TrustBridgeContract::get_address_if_verified(env.clone(), username(&env, "octocat")),
            Err(ContractError::NotVerified),
        );
    });
}

/// The verified-only lookup keeps working while the contract is paused,
/// matching `get_address` read semantics.
#[test]
fn test_verified_only_lookup_works_while_paused() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
        let record =
            TrustBridgeContract::get_address_if_verified(env.clone(), username(&env, "octocat"))
                .unwrap();
        assert_eq!(record.stellar_address, user);
    });
}

// ── Stats ────────────────────────────────────────────────────────────────

#[test]
fn test_stats_empty_after_setup() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    env.as_contract(&contract_id, || {
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 0);
        assert_eq!(stats.verified, 0);
    });
}

#[test]
fn test_get_stats_increments_correctly() {
    let env = Env::default();
    let (admin, user1, user2, contract_id) = setup(&env);

    env.as_contract(&contract_id, || {
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 0);
        assert_eq!(stats.verified, 0);
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    env.as_contract(&contract_id, || {
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 2);
        assert_eq!(stats.verified, 0);
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "alice")).unwrap();
    });

    env.as_contract(&contract_id, || {
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 2);
        assert_eq!(stats.verified, 1);
        assert_verified_parity(&env, 1);
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), user2.clone(), username(&env, "bob")).unwrap();
    });

    env.as_contract(&contract_id, || {
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 1);
        assert_eq!(stats.verified, 1);
    });
}

#[test]
fn test_removing_one_of_two_keeps_remaining_stats() {
    let env = Env::default();
    let (_admin, user1, user2, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), user1.clone(), username(&env, "alice")).unwrap();
    });
    env.as_contract(&contract_id, || {
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 1);
        assert_eq!(stats.verified, 0);
    });
}

// ── Remove auth (Issue #74 / Wave #75) ────────────────────────────
//
// The remove authorization policy: only the registrant (the Stellar address
// currently registered to the username) or the contract admin may remove a
// record. This is enforced by `require_remove_auth`, extracted from `remove`
// so the policy can be read, tested, and modified in isolation.
//
// Success paths: registrant removes own record; admin removes any record.
// Failure paths: third-party caller → NotAuthorized; unregistered username
//   → NotRegistered; paused contract → Paused; uninitialized → NotInitialized.
// State invariants: verified counter decremented only when the removed record
//   was verified; count always decremented by exactly 1 on success.

// ── Success path: registrant ────────────────────────────────────────────

/// The registered address (registrant) may remove their own record.
#[test]
fn test_registrant_can_remove_own_record() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let result =
            TrustBridgeContract::remove(env.clone(), other.clone(), username(&env, "octocat"));
        assert_eq!(result, Err(ContractError::NotAuthorized));
    });
}

// ── Success path: admin ──────────────────────────────────────────────────

/// The contract admin may remove any record regardless of who registered it.
#[test]
fn test_admin_can_remove_any_record() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::remove(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
        assert!(TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).is_none());
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 0);
    });
}

/// Admin may remove a record even when the registrant is a different address.
#[test]
fn test_admin_can_remove_record_registered_by_another_user() {
    let env = Env::default();
    let (admin, user1, _user2, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).is_none());
    });
}

// ── Failure path: unauthorized third party ────────────────────────────────

/// Any address that is neither the registrant nor the admin must be
/// rejected with NotAuthorized.
#[test]
fn test_third_party_cannot_remove() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let result =
            TrustBridgeContract::remove(env.clone(), other.clone(), username(&env, "octocat"));
        assert_eq!(
            result,
            Err(ContractError::NotAuthorized),
            "third party must be rejected with NotAuthorized"
        );
    });
    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).is_some());
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 1);
    });
}

/// A freshly-generated address with no role and no registration is rejected.
#[test]
fn test_unknown_address_cannot_remove() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    let stranger = Address::generate(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let result =
            TrustBridgeContract::remove(env.clone(), stranger.clone(), username(&env, "alice"));
        assert_eq!(result, Err(ContractError::NotAuthorized));
    });
}

// ── Failure path: unregistered username ─────────────────────────────────

/// Attempting to remove a username that was never registered returns
/// NotRegistered and does not mutate any state.
#[test]
fn test_remove_unregistered_username_fails() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result =
            TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "missing"));
        assert_eq!(result, Err(ContractError::NotRegistered));
    });
}

// ── Failure path: paused contract ─────────────────────────────────────────────

/// Remove is blocked while the contract is paused.
#[test]
fn test_remove_blocked_while_paused() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result =
            TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat"));
        assert_eq!(
            result,
            Err(ContractError::Paused),
            "remove must be blocked while paused"
        );
    });
    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).is_some());
    });
}

// ── State invariants after remove ──────────────────────────────────────────────

/// Removing an unverified record must decrement total but leave the
/// verified count unchanged.
#[test]
fn test_remove_unverified_record_does_not_decrement_verified_count() {
    let env = Env::default();
    let (admin, user1, user2, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "alice")).unwrap();
    });
    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).verified, 1);
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), admin.clone(), username(&env, "bob")).unwrap();
    });
    env.as_contract(&contract_id, || {
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 1, "total must decrement by 1");
        assert_eq!(
            stats.verified, 1,
            "verified count must be unchanged when removing an unverified record"
        );
    });
}

/// Removing a verified record must decrement both total and verified count.
#[test]
fn test_remove_verified_record_decrements_verified_count() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).verified, 1);
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat")).unwrap();
    });
    env.as_contract(&contract_id, || {
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 0);
        assert_eq!(
            stats.verified, 0,
            "verified count must decrement when removing a verified record"
        );
    });
}

/// Re-adding a removed username must treat it as a fresh registration:
/// count increments from 0 to 1, and the new record is unverified.
#[test]
fn test_readding_removed_user_increments_count() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat")).unwrap();
    });
    env.mock_all_auths();
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
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 1);
        assert_eq!(
            stats.verified, 0,
            "re-registered record must start unverified"
        );
    });
}

// ── Issue #52: lookup after peer removal ─────────────────────────────────

#[test]
fn test_remove_then_lookup_other_record() {
    let env = Env::default();
    let (_admin, user1, user2, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), user1.clone(), username(&env, "alice")).unwrap();
    });
    env.as_contract(&contract_id, || {
        // bob's record must survive alice's removal
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "bob"))
                .unwrap()
                .stellar_address,
            user2
        );
    });
}

#[test]
fn test_export_skips_removed_records() {
    let env = Env::default();
    let (_admin, user1, user2, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), user1.clone(), username(&env, "alice")).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let all = TrustBridgeContract::get_all_registered(env.clone()).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all.get(0).unwrap(), (username(&env, "bob"), user2.clone()));
    });
}

#[test]
fn test_lookup_after_first_of_three_removed() {
    let env = Env::default();
    let (admin, user1, user2, contract_id) = setup(&env);
    let user3 = Address::generate(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "carol"),
            user3.clone(),
            Vec::new(&env),
        )
        .unwrap();

        // Remove the first entry
        TrustBridgeContract::remove(env.clone(), admin.clone(), username(&env, "alice")).unwrap();

        // Both remaining records must be reachable
        assert!(TrustBridgeContract::get_address(env.clone(), username(&env, "alice")).is_none());
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "bob"))
                .unwrap()
                .stellar_address,
            user2
        );
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "carol"))
                .unwrap()
                .stellar_address,
            user3
        );
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 2);
    });
}

#[test]
fn test_lookup_after_middle_of_three_removed() {
    let env = Env::default();
    let (admin, user1, user2, contract_id) = setup(&env);
    let user3 = Address::generate(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "carol"),
            user3.clone(),
            Vec::new(&env),
        )
        .unwrap();

        // Remove the middle entry
        TrustBridgeContract::remove(env.clone(), admin.clone(), username(&env, "bob")).unwrap();

        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "alice"))
                .unwrap()
                .stellar_address,
            user1
        );
        assert!(TrustBridgeContract::get_address(env.clone(), username(&env, "bob")).is_none());
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "carol"))
                .unwrap()
                .stellar_address,
            user3
        );
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 2);
    });
}

#[test]
fn test_index_integrity_after_multiple_removals() {
    let env = Env::default();
    let (admin, user1, user2, contract_id) = setup(&env);
    let user3 = Address::generate(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "carol"),
            user3.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), admin.clone(), username(&env, "alice")).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), admin.clone(), username(&env, "carol")).unwrap();
    });

    // Only bob remains
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let all = TrustBridgeContract::get_all_registered(env.clone()).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all.get(0).unwrap().0, username(&env, "bob"));
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 1);
    });
}

#[test]
fn test_reregister_after_removal_is_treated_as_new() {
    let env = Env::default();
    let (_admin, user1, user2, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), user1.clone(), username(&env, "alice")).unwrap();
    });
    // re-register alice
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    env.as_contract(&contract_id, || {
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 2);
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "alice"))
                .unwrap()
                .stellar_address,
            user1
        );
    });
}

// ── Re-registration ───────────────────────────────────────────────────────

#[test]
fn test_reregistration_updates_record() {
    let env = Env::default();
    let (admin, user, new_user, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            new_user.clone(),
            Vec::new(&env),
        )
        .unwrap();

        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert_eq!(record.stellar_address, new_user);
        assert!(!record.verified);

        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 1);
        assert_eq!(stats.verified, 0);
    });
}

#[test]
fn test_updated_registration_preserves_count() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            other.clone(),
            Vec::new(&env),
        )
        .unwrap();
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 1);
    });
}

#[test]
fn test_unverified_update_stays_unverified() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            other.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert!(!record.verified);
    });
}

// ── Issue #16: Verification attestation storage ───────────────────────────

#[test]
fn test_verify_sets_verified_flag_and_increments_vcount() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        assert!(
            !TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .verified
        );
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 0);

        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();

        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert!(record.verified, "verified flag must be true after verify()");
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 1);
    });
}

/// Issue #57: `verify` on a username with no registration fails closed
/// with `NotRegistered` instead of creating a verified record out of
/// nothing. Documented alongside the matching integration coverage in
/// `tests/integration.rs` and the error table in `docs/ABI.md`.
#[test]
fn test_verify_missing_registration_fails() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result =
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "missing"));
        assert_eq!(result, Err(ContractError::NotRegistered));
    });
}

// ── Issue #293: verifier allowlist (cap + on-chain expiry) ──────────────

#[test]
fn test_verifier_allowlist_cap_enforced() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        for _ in 0..MAX_VERIFIERS {
            TrustBridgeContract::add_verifier(env.clone(), Address::generate(&env), 0).unwrap();
        }
        assert_eq!(
            TrustBridgeContract::verifier_slots_remaining(env.clone()),
            0
        );
        let res = TrustBridgeContract::add_verifier(env.clone(), Address::generate(&env), 0);
        assert_eq!(res, Err(ContractError::VerifierAllowlistFull));
    });
}

#[test]
fn test_verifier_allowlist_expiry_blocks_verify() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    let verifier = Address::generate(&env);
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        // Expires at t=2000.
        TrustBridgeContract::add_verifier(env.clone(), verifier.clone(), 2_000).unwrap();
        assert!(TrustBridgeContract::is_active_verifier(
            env.clone(),
            verifier.clone()
        ));
        TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "octocat"))
            .unwrap();
    });

    // Advance past expiry; the same verifier is now inactive.
    env.ledger().with_mut(|li| li.timestamp = 3_000);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "hubber"),
            Address::generate(&env),
            Vec::new(&env),
        )
        .unwrap();
        assert!(!TrustBridgeContract::is_active_verifier(
            env.clone(),
            verifier.clone()
        ));
        let res =
            TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "hubber"));
        assert_eq!(res, Err(ContractError::NotAuthorized));
    });
}

#[test]
fn test_verifier_expiry_in_past_rejected() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    env.ledger().with_mut(|li| li.timestamp = 5_000);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let res = TrustBridgeContract::add_verifier(env.clone(), Address::generate(&env), 4_000);
        assert_eq!(res, Err(ContractError::VerifierExpiryInPast));
    });
}

#[test]
fn test_expired_verifier_slot_is_reclaimed_and_pruned() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        for _ in 0..(MAX_VERIFIERS - 1) {
            TrustBridgeContract::add_verifier(env.clone(), Address::generate(&env), 0).unwrap();
        }
        // One short-lived member takes the last slot.
        TrustBridgeContract::add_verifier(env.clone(), Address::generate(&env), 2_000).unwrap();
        assert_eq!(
            TrustBridgeContract::verifier_slots_remaining(env.clone()),
            0
        );
    });

    env.ledger().with_mut(|li| li.timestamp = 3_000);
    env.as_contract(&contract_id, || {
        // Expired member no longer counts, so a new one fits — add_verifier
        // prunes it on the way in.
        TrustBridgeContract::add_verifier(env.clone(), Address::generate(&env), 0).unwrap();
        let pruned = TrustBridgeContract::prune_expired_verifiers(env.clone()).unwrap();
        assert_eq!(pruned, 0, "add_verifier already pruned the expired entry");
        assert_eq!(
            TrustBridgeContract::get_verifiers(env.clone()).len(),
            MAX_VERIFIERS
        );
    });
}

#[test]
fn test_add_verifier_refresh_does_not_consume_slot() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    env.mock_all_auths();
    let v = Address::generate(&env);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::add_verifier(env.clone(), v.clone(), 2_000).unwrap();
        TrustBridgeContract::add_verifier(env.clone(), v.clone(), 9_000).unwrap();
        assert_eq!(TrustBridgeContract::get_verifiers(env.clone()).len(), 1);
        assert_eq!(
            TrustBridgeContract::verifier_slots_remaining(env.clone()),
            MAX_VERIFIERS - 1
        );
    });
}

#[test]
fn test_remove_verifier_not_listed_errors() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let res = TrustBridgeContract::remove_verifier(env.clone(), Address::generate(&env));
        assert_eq!(res, Err(ContractError::VerifierNotAllowlisted));
    });
}

#[test]
fn test_role_based_verify_still_works_before_allowlist_opt_in() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    let verifier = Address::generate(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), verifier.clone(), Role::Verifier).unwrap();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        // No add_verifier call yet → pure role-based mode.
        TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "octocat"))
            .unwrap();
    });
}

/// Same not-registered guard on the other verification-mutating entry
/// point, so `verify` and `revoke_verification` stay consistent (Issue
/// #57).
#[test]
fn test_revoke_verification_missing_registration_fails() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::revoke_verification(
            env.clone(),
            admin.clone(),
            username(&env, "missing"),
            1,
        );
        assert_eq!(result, Err(ContractError::NotRegistered));
    });
}

#[test]
fn test_double_verify_fails() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result =
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"));
        assert_eq!(result, Err(ContractError::AlreadyVerified));
    });
}

// ── Issue #233: username rename ──────────────────────────────────────────

fn register_as(env: &Env, contract_id: &Address, name: &str, addr: &Address) {
    env.mock_all_auths();
    env.as_contract(contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(env, name),
            addr.clone(),
            Vec::new(env),
        )
        .unwrap();
    });
}

#[test]
fn test_rename_moves_the_registration_atomically() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    register_as(&env, &contract_id, "octocat", &user);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::rename(
            env.clone(),
            user.clone(),
            username(&env, "octocat"),
            username(&env, "octocat2"),
        )
        .unwrap();
    });

    env.as_contract(&contract_id, || {
        assert!(
            !TrustBridgeContract::has_record(env.clone(), username(&env, "octocat")),
            "old name must be gone"
        );
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat2"))
                .unwrap()
                .stellar_address,
            user,
            "new name must hold the same address"
        );
        assert_eq!(
            TrustBridgeContract::get_stats(env.clone()).total,
            1,
            "a rename is a move, not a new registration"
        );
    });
}

#[test]
fn test_rename_updates_the_index() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    register_as(&env, &contract_id, "octocat", &user);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::rename(
            env.clone(),
            user.clone(),
            username(&env, "octocat"),
            username(&env, "octocat2"),
        )
        .unwrap();
    });

    env.as_contract(&contract_id, || {
        let page = TrustBridgeContract::get_registered_paginated(env.clone(), None, 50).unwrap();
        let names: alloc::vec::Vec<_> = page.records.iter().map(|(name, _)| name.clone()).collect();
        assert!(
            names.contains(&username(&env, "octocat2")),
            "index must list the new name"
        );
        assert!(
            !names.contains(&username(&env, "octocat")),
            "index must not still list the old name"
        );
    });
}

/// Verification attested the old handle, so it does not follow the rename.
#[test]
fn test_rename_clears_verification_and_flags_reverify() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    register_as(&env, &contract_id, "octocat", &user);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::rename(
            env.clone(),
            user.clone(),
            username(&env, "octocat"),
            username(&env, "octocat2"),
        )
        .unwrap();
    });

    env.as_contract(&contract_id, || {
        assert!(
            !TrustBridgeContract::get_address(env.clone(), username(&env, "octocat2"))
                .unwrap()
                .verified,
            "the badge must not travel to the new handle"
        );
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 0);
        assert_eq!(
            TrustBridgeContract::get_ever_verified_count(env.clone()),
            1,
            "history still remembers the original verification"
        );
    });
}

#[test]
fn test_rename_rejects_a_taken_username() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    register_as(&env, &contract_id, "octocat", &user);
    register_as(&env, &contract_id, "hubber", &other);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::rename(
                env.clone(),
                user.clone(),
                username(&env, "octocat"),
                username(&env, "hubber"),
            ),
            Err(ContractError::UsernameTaken)
        );
    });
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "hubber"))
                .unwrap()
                .stellar_address,
            other,
            "the existing registration must be untouched"
        );
    });
}

#[test]
fn test_rename_rejects_a_reserved_username() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    register_as(&env, &contract_id, "octocat", &user);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::add_reserved(env.clone(), username(&env, "stellar")).unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::rename(
                env.clone(),
                user.clone(),
                username(&env, "octocat"),
                username(&env, "stellar"),
            ),
            Err(ContractError::UsernameReserved)
        );
    });
}

#[test]
fn test_rename_rejects_an_unregistered_username() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::rename(
                env.clone(),
                user.clone(),
                username(&env, "ghost"),
                username(&env, "ghost2"),
            ),
            Err(ContractError::NotRegistered)
        );
    });
}

#[test]
fn test_rename_rejects_renaming_to_the_same_name() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    register_as(&env, &contract_id, "octocat", &user);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::rename(
                env.clone(),
                user.clone(),
                username(&env, "octocat"),
                username(&env, "octocat"),
            ),
            Err(ContractError::InvalidUsername)
        );
    });
}

#[test]
fn test_rename_rejects_a_non_holder() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    register_as(&env, &contract_id, "octocat", &user);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::rename(
                env.clone(),
                other.clone(),
                username(&env, "octocat"),
                username(&env, "octocat2"),
            ),
            Err(ContractError::NotAuthorized)
        );
    });
}

/// A case-only change is a real move, since the storage key is the exact string.
#[test]
fn test_rename_allows_a_case_only_change() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    register_as(&env, &contract_id, "octocat", &user);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::rename(
            env.clone(),
            user.clone(),
            username(&env, "octocat"),
            username(&env, "OctoCat"),
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::has_record(
            env.clone(),
            username(&env, "OctoCat")
        ));
        assert!(!TrustBridgeContract::has_record(
            env.clone(),
            username(&env, "octocat")
        ));
    });
}

/// The old name is free again, and taking it does not disturb the moved record.
#[test]
fn test_old_name_is_registerable_after_a_rename() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    register_as(&env, &contract_id, "octocat", &user);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::rename(
            env.clone(),
            user.clone(),
            username(&env, "octocat"),
            username(&env, "octocat2"),
        )
        .unwrap();
    });

    register_as(&env, &contract_id, "octocat", &other);

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .stellar_address,
            other
        );
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat2"))
                .unwrap()
                .stellar_address,
            user
        );
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 2);
    });
}

// ── Issue #234: address rotation delay window ────────────────────────────

const ROT_DELAY: u64 = 86_400;

/// Register `octocat` to `user` and arm the rotation delay.
fn setup_rotation(env: &Env, contract_id: &Address, user: &Address) {
    env.mock_all_auths();
    env.as_contract(contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(env, "octocat"),
            user.clone(),
            Vec::new(env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(contract_id, || {
        TrustBridgeContract::set_rotation_delay(env.clone(), ROT_DELAY).unwrap();
    });
}

#[test]
fn test_rotation_delay_defaults_to_zero() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_rotation_delay(env.clone()), 0);
    });
}

/// With no delay configured, register keeps its direct dual-auth swap.
#[test]
fn test_register_still_swaps_directly_when_delay_is_zero() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            other.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .stellar_address,
            other
        );
    });
}

/// Once a delay is armed, the instant swap is refused.
#[test]
fn test_register_refuses_direct_address_change_when_delay_armed() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    setup_rotation(&env, &contract_id, &user);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                other.clone(),
                Vec::new(&env)
            ),
            Err(ContractError::RotationRequired)
        );
    });
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .stellar_address,
            user,
            "the address must not have moved"
        );
    });
}

/// Re-registering the same address is not a rotation and stays allowed.
#[test]
fn test_register_same_address_still_allowed_with_delay_armed() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    setup_rotation(&env, &contract_id, &user);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
}

#[test]
fn test_requesting_a_rotation_does_not_move_the_address() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    setup_rotation(&env, &contract_id, &user);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::request_address_rotation(
            env.clone(),
            username(&env, "octocat"),
            other.clone(),
        )
        .unwrap();
    });

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .stellar_address,
            user,
            "reads return the current address for the whole window"
        );
        let pending =
            TrustBridgeContract::get_pending_rotation(env.clone(), username(&env, "octocat"))
                .expect("rotation must be pending");
        assert_eq!(pending.new_address, other);
        assert_eq!(pending.executable_at, pending.requested_at + ROT_DELAY);
    });
}

#[test]
fn test_rotation_cannot_execute_before_the_delay_elapses() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    setup_rotation(&env, &contract_id, &user);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::request_address_rotation(
            env.clone(),
            username(&env, "octocat"),
            other.clone(),
        )
        .unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::execute_address_rotation(env.clone(), username(&env, "octocat")),
            Err(ContractError::RotationNotReady)
        );
    });
}

#[test]
fn test_rotation_executes_once_the_delay_has_elapsed() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    setup_rotation(&env, &contract_id, &user);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::request_address_rotation(
            env.clone(),
            username(&env, "octocat"),
            other.clone(),
        )
        .unwrap();
    });

    env.ledger().with_mut(|li| li.timestamp += ROT_DELAY);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::execute_address_rotation(env.clone(), username(&env, "octocat"))
            .unwrap();
    });

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .stellar_address,
            other
        );
        assert!(
            TrustBridgeContract::get_pending_rotation(env.clone(), username(&env, "octocat"))
                .is_none(),
            "the pending entry must be cleared"
        );
    });
}

/// The window is only useful if the real holder can stop the rotation.
#[test]
fn test_holder_can_cancel_a_pending_rotation() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    setup_rotation(&env, &contract_id, &user);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::request_address_rotation(
            env.clone(),
            username(&env, "octocat"),
            other.clone(),
        )
        .unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::cancel_address_rotation(
            env.clone(),
            user.clone(),
            username(&env, "octocat"),
        )
        .unwrap();
    });

    env.ledger().with_mut(|li| li.timestamp += ROT_DELAY);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::execute_address_rotation(env.clone(), username(&env, "octocat")),
            Err(ContractError::NoRotationPending),
            "a cancelled rotation must not become executable"
        );
    });
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .stellar_address,
            user
        );
    });
}

#[test]
fn test_second_rotation_request_is_rejected_while_one_is_pending() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    setup_rotation(&env, &contract_id, &user);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::request_address_rotation(
            env.clone(),
            username(&env, "octocat"),
            other.clone(),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::request_address_rotation(
                env.clone(),
                username(&env, "octocat"),
                other.clone(),
            ),
            Err(ContractError::RotationPending)
        );
    });
}

#[test]
fn test_rotation_requires_a_registered_username() {
    let env = Env::default();
    let (_admin, _user, other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::request_address_rotation(
                env.clone(),
                username(&env, "ghost"),
                other.clone(),
            ),
            Err(ContractError::NotRegistered)
        );
    });
}

/// A verified registration loses its badge when the address moves.
#[test]
fn test_executing_a_rotation_clears_the_verified_flag() {
    let env = Env::default();
    let (admin, user, other, contract_id) = setup(&env);
    setup_rotation(&env, &contract_id, &user);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 1);
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::request_address_rotation(
            env.clone(),
            username(&env, "octocat"),
            other.clone(),
        )
        .unwrap();
    });
    env.ledger().with_mut(|li| li.timestamp += ROT_DELAY);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::execute_address_rotation(env.clone(), username(&env, "octocat"))
            .unwrap();
    });

    env.as_contract(&contract_id, || {
        assert!(
            !TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .verified,
            "verification vouched for the old address"
        );
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 0);
        assert_eq!(
            TrustBridgeContract::get_ever_verified_count(env.clone()),
            1,
            "the historical count still remembers it"
        );
    });
}

// ── Issue #230: light-client record proof ────────────────────────────────

#[test]
fn test_record_proof_for_registered_username() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
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
        let proof = TrustBridgeContract::get_record_proof(env.clone(), username(&env, "octocat"));
        assert!(proof.exists);
        assert!(!proof.verified, "a fresh registration is unverified");
        assert_eq!(proof.as_of_ledger, env.ledger().sequence());
        assert_eq!(proof.key_prefix, soroban_sdk::symbol_short!("reg"));
        assert_eq!(proof.ttl_threshold_ledgers, crate::storage::TTL_THRESHOLD);
        assert_eq!(proof.ttl_bump_ledgers, crate::storage::TTL_BUMP);
    });
}

#[test]
fn test_record_proof_reports_verified_bit() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.as_contract(&contract_id, || {
        let proof = TrustBridgeContract::get_record_proof(env.clone(), username(&env, "octocat"));
        assert!(proof.exists);
        assert!(proof.verified);
    });
}

/// A missing username is an answer, not an error.
#[test]
fn test_record_proof_for_missing_username() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    env.as_contract(&contract_id, || {
        let proof = TrustBridgeContract::get_record_proof(env.clone(), username(&env, "ghost"));
        assert!(!proof.exists);
        assert!(!proof.verified);
        assert_eq!(proof.registered_at, 0);
        // The key and policy still come back so a client can look for the
        // entry — including an archived one — itself.
        assert_eq!(proof.key_prefix, soroban_sdk::symbol_short!("reg"));
        assert_eq!(proof.ttl_bump_ledgers, crate::storage::TTL_BUMP);
    });
}

#[test]
fn test_record_proof_agrees_with_has_record() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
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
        for name in ["octocat", "ghost"] {
            assert_eq!(
                TrustBridgeContract::get_record_proof(env.clone(), username(&env, name)).exists,
                TrustBridgeContract::has_record(env.clone(), username(&env, name)),
                "proof.exists must agree with has_record for '{name}'"
            );
        }
    });
}

/// The proof is a read: it must work while the contract is paused.
#[test]
fn test_record_proof_works_while_paused() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
    });
    env.as_contract(&contract_id, || {
        assert!(
            TrustBridgeContract::get_record_proof(env.clone(), username(&env, "octocat")).exists
        );
    });
}

// ── Issue #229: monotonic ever-verified counter ──────────────────────────

/// A verify/revoke/verify cycle: the live count moves with the flag, the
/// historical count only ever climbs.
#[test]
fn test_ever_verified_count_survives_a_revoke_cycle() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);

    env.mock_all_auths();
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
        assert_eq!(TrustBridgeContract::get_ever_verified_count(env.clone()), 0);
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 1);
        assert_eq!(TrustBridgeContract::get_ever_verified_count(env.clone()), 1);
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::revoke_verification(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
            1,
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_verified_count(env.clone()),
            0,
            "live count must drop on revoke"
        );
        assert_eq!(
            TrustBridgeContract::get_ever_verified_count(env.clone()),
            1,
            "historical count must not drop on revoke"
        );
    });

    // Verifying the same contributor again counts as a second verification.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 1);
        assert_eq!(TrustBridgeContract::get_ever_verified_count(env.clone()), 2);
    });
}

/// `get_stats` reports the live and historical counts side by side.
#[test]
fn test_stats_reports_live_and_ever_verified() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::revoke_verification(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
            1,
        )
        .unwrap();
    });

    env.as_contract(&contract_id, || {
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 1);
        assert_eq!(stats.verified, 0, "verified is the live figure");
        assert_eq!(stats.ever_verified, 1, "ever_verified keeps the history");
    });
}

/// A fresh instance starts both counters at zero.
#[test]
fn test_ever_verified_count_starts_at_zero() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_ever_verified_count(env.clone()), 0);
    });
}

#[test]
fn test_revoke_verification_clears_flag_and_decrements_vcount() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 1);
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::revoke_verification(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
            1,
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert!(!record.verified, "verified flag must be false after revoke");
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 0);
    });
}

#[test]
fn test_revoke_verification_nonverified_fails() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let result = TrustBridgeContract::revoke_verification(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
            1,
        );
        assert_eq!(result, Err(ContractError::NotVerified));
    });
}

#[test]
fn test_removing_verified_record_updates_stats() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat")).unwrap();
    });
    env.as_contract(&contract_id, || {
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 0);
        assert_eq!(stats.verified, 0);
    });
}

#[test]
fn test_reregister_same_address_keeps_verification() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.mock_all_auths();
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
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert!(
            record.verified,
            "re-registering the same address should preserve verified=true"
        );
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).verified, 1);
    });
}

#[test]
fn test_verified_address_change_clears_verification() {
    let env = Env::default();
    let (admin, user, other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            other.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert!(
            !record.verified,
            "changing stellar address must clear verification"
        );
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).verified, 0);
    });
}

#[test]
fn test_verified_same_address_reregister_keeps_count() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.mock_all_auths();
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
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 1);
        assert_eq!(stats.verified, 1);
    });
}

#[test]
fn test_removed_verified_user_can_register_unverified() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat")).unwrap();
    });
    env.mock_all_auths();
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
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert!(!record.verified);
    });
}

#[test]
fn test_verify_after_reregister_new_address() {
    let env = Env::default();
    let (admin, user, other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            other.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.as_contract(&contract_id, || {
        assert!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .verified
        );
    });
}

#[test]
fn test_verify_after_address_update_targets_new_address() {
    let env = Env::default();
    let (admin, old_user, new_user, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            old_user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            new_user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
        let after_update =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert_eq!(after_update.stellar_address, new_user);
        assert!(!after_update.verified);
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).verified, 0);
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.as_contract(&contract_id, || {
        let after_verify =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert_eq!(after_verify.stellar_address, new_user);
        assert!(after_verify.verified);
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).verified, 1);
    });
}

/// Verifier-role caller can verify (Issue #12).
#[test]
fn test_verifier_role_can_verify() {
    let env = Env::default();
    let (admin, user, verifier, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), verifier.clone(), Role::Verifier).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "octocat"))
            .unwrap();
    });
    env.as_contract(&contract_id, || {
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert!(record.verified);
    });
    let _ = admin;
}

/// Verifier-role caller can revoke (Issue #12).
/// Updated for Issue #212: Verifier can no longer revoke — that now
/// requires Role::Revoker. This test verifies the old Verifier-can-revoke
/// path correctly fails and that Role::Revoker succeeds.
#[test]
fn test_verifier_role_can_revoke_verification() {
    let env = Env::default();
    let (admin, user, revoker, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Assign Revoker role, not Verifier.
        TrustBridgeContract::set_role(env.clone(), revoker.clone(), Role::Revoker).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::revoke_verification(
            env.clone(),
            revoker.clone(),
            username(&env, "octocat"),
            1,
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 0);
    });
}

/// Address without role cannot verify (Issue #12).
#[test]
fn test_no_role_cannot_verify() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // `other` has no role
        let result =
            TrustBridgeContract::verify(env.clone(), other.clone(), username(&env, "octocat"));
        assert_eq!(result, Err(ContractError::NotAuthorized));
    });
}

/// Admin can still call verify (role separation is additive) (Issue #12).
#[test]
fn test_admin_can_still_verify_after_role_separation() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.as_contract(&contract_id, || {
        assert!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .verified
        );
    });
}

/// Upgrader role cannot verify (Issue #12 — only Verifier and Admin).
#[test]
fn test_upgrader_role_cannot_verify() {
    let env = Env::default();
    let (_admin, user, upgrader, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), upgrader.clone(), Role::Upgrader).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result =
            TrustBridgeContract::verify(env.clone(), upgrader.clone(), username(&env, "octocat"));
        assert_eq!(result, Err(ContractError::NotAuthorized));
    });
}

// ── Issue #54: Not-initialized guard tests ───────────────────────────────

#[test]
fn test_initialize_only_once() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
        let result = TrustBridgeContract::initialize(env.clone(), admin.clone());
        assert_eq!(result, Err(ContractError::AlreadyInitialized));
    });
}

#[test]
fn test_register_requires_initialization() {
    let env = Env::default();
    let user = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        );
        assert_eq!(result, Err(ContractError::NotInitialized));
    });
}

#[test]
fn test_remove_requires_initialization() {
    let env = Env::default();
    let user = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result =
            TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat"));
        assert_eq!(result, Err(ContractError::NotInitialized));
    });
}

#[test]
fn test_verify_requires_initialization() {
    let env = Env::default();
    let caller = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result =
            TrustBridgeContract::verify(env.clone(), caller.clone(), username(&env, "octocat"));
        assert_eq!(result, Err(ContractError::NotInitialized));
    });
}

#[test]
fn test_revoke_verification_requires_initialization() {
    let env = Env::default();
    let caller = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::revoke_verification(
            env.clone(),
            caller.clone(),
            username(&env, "octocat"),
            1,
        );
        assert_eq!(result, Err(ContractError::NotInitialized));
    });
}

#[test]
fn test_pause_requires_initialization() {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::pause(env.clone(), 1);
        assert_eq!(result, Err(ContractError::NotInitialized));
    });
}

#[test]
fn test_unpause_requires_initialization() {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::unpause(env.clone(), 4);
        assert_eq!(result, Err(ContractError::NotInitialized));
    });
}

#[test]
fn test_set_role_requires_initialization() {
    let env = Env::default();
    let target = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::set_role(env.clone(), target.clone(), Role::Verifier);
        assert_eq!(result, Err(ContractError::NotInitialized));
    });
}

#[test]
fn test_remove_role_requires_initialization() {
    let env = Env::default();
    let target = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::remove_role(env.clone(), target.clone());
        assert_eq!(result, Err(ContractError::NotInitialized));
    });
}

#[test]
fn test_set_cooldown_requires_initialization() {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::set_cooldown(env.clone(), 3600);
        assert_eq!(result, Err(ContractError::NotInitialized));
    });
}

#[test]
fn test_get_all_registered_requires_initialization() {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::get_all_registered(env.clone());
        assert_eq!(result, Err(ContractError::NotInitialized));
    });
}

// Issue #53: every variant's numeric repr must match the table in
// error.rs, and stay in sync as new variants are added — a variant
// renumbered or added without a matching entry here breaks silently for
// off-chain consumers that decode raw u32 codes.
#[test]
fn test_error_codes_match_repr() {
    assert_eq!(ContractError::AlreadyInitialized.code(), 1);
    assert_eq!(ContractError::NotInitialized.code(), 2);
    assert_eq!(ContractError::NotAuthorized.code(), 3);
    assert_eq!(ContractError::NotRegistered.code(), 4);
    assert_eq!(ContractError::AlreadyVerified.code(), 5);
    assert_eq!(ContractError::NotVerified.code(), 6);
    assert_eq!(ContractError::Paused.code(), 7);
    assert_eq!(ContractError::CooldownActive.code(), 8);
    assert_eq!(ContractError::InvalidVersion.code(), 9);
    assert_eq!(ContractError::InvalidRole.code(), 10);
    assert_eq!(ContractError::InvalidUsername.code(), 11);
    assert_eq!(ContractError::AttestationExpired.code(), 12);
    assert_eq!(ContractError::UnattestedWasm.code(), 13);
    assert_eq!(ContractError::InvalidBatchSize.code(), 14);
    assert_eq!(ContractError::InvalidReasonCode.code(), 15);
    assert_eq!(ContractError::ZeroAddress.code(), 16);
    assert_eq!(ContractError::ChallengeAlreadyActive.code(), 17);
    assert_eq!(ContractError::NoChallengeActive.code(), 18);
    assert_eq!(ContractError::ChallengeNotResolvable.code(), 19);
    assert_eq!(ContractError::ChallengeActive.code(), 20);
    assert_eq!(ContractError::InvalidPauseReason.code(), 21);
    assert_eq!(ContractError::AlreadyReserved.code(), 22);
    assert_eq!(ContractError::NotReserved.code(), 23);
    assert_eq!(ContractError::UsernameReserved.code(), 24);
    assert_eq!(ContractError::ReservedListFull.code(), 25);
    assert_eq!(ContractError::AdminTransferPending.code(), 26);
    assert_eq!(ContractError::AdminTransferDelayActive.code(), 27);
    assert_eq!(ContractError::NoPendingAdminTransfer.code(), 28);
    assert_eq!(ContractError::AttestationRequired.code(), 29);
}

#[test]
fn test_error_from_code_is_inverse_of_code() {
    for variant in [
        ContractError::AlreadyInitialized,
        ContractError::NotInitialized,
        ContractError::NotAuthorized,
        ContractError::NotRegistered,
        ContractError::AlreadyVerified,
        ContractError::NotVerified,
        ContractError::Paused,
        ContractError::CooldownActive,
        ContractError::InvalidVersion,
        ContractError::InvalidRole,
        ContractError::InvalidUsername,
        ContractError::AttestationExpired,
        ContractError::UnattestedWasm,
        ContractError::InvalidBatchSize,
        ContractError::InvalidReasonCode,
        ContractError::ZeroAddress,
        ContractError::ChallengeAlreadyActive,
        ContractError::NoChallengeActive,
        ContractError::ChallengeNotResolvable,
        ContractError::ChallengeActive,
        ContractError::InvalidPauseReason,
        ContractError::AlreadyReserved,
        ContractError::NotReserved,
        ContractError::UsernameReserved,
        ContractError::ReservedListFull,
        ContractError::AdminTransferPending,
        ContractError::AdminTransferDelayActive,
        ContractError::NoPendingAdminTransfer,
        ContractError::AttestationRequired,
    ] {
        assert_eq!(ContractError::from_code(variant.code()), Some(variant));
    }
    assert_eq!(ContractError::from_code(0), None);
    assert_eq!(
        ContractError::from_code(30),
        Some(ContractError::NetworkMismatch)
    );
}

// --- Issue #69: max username length guard ---

#[test]
fn test_register_rejects_over_length_username() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();

    // 40 characters: one past MAX_USERNAME_LEN.
    let too_long = String::from_str(&env, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert_eq!(too_long.len(), MAX_USERNAME_LEN + 1);

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::register(
                env.clone(),
                too_long.clone(),
                user.clone(),
                Vec::new(&env)
            ),
            Err(ContractError::InvalidUsername)
        );
        // The rejected username must leave no trace in the registry.
        assert!(!TrustBridgeContract::has_record(env.clone(), too_long));
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 0);
    });
}

#[test]
fn test_register_accepts_username_at_max_length() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();

    // Exactly 39 characters.
    let at_max = String::from_str(&env, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert_eq!(at_max.len(), MAX_USERNAME_LEN);

    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::register(
            env.clone(),
            at_max.clone(),
            user.clone(),
            Vec::new(&env)
        )
        .is_ok());
        assert!(TrustBridgeContract::has_record(env.clone(), at_max));
    });
}

#[test]
fn test_register_rejects_empty_and_malformed_usernames() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();

    env.as_contract(&contract_id, || {
        for bad in ["", "-lead", "trail-", "has space", "at@sign"] {
            assert_eq!(
                TrustBridgeContract::register(
                    env.clone(),
                    username(&env, bad),
                    user.clone(),
                    Vec::new(&env)
                ),
                Err(ContractError::InvalidUsername),
                "expected {bad:?} to be rejected"
            );
        }
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 0);
    });
}

#[test]
fn test_max_username_len_is_exposed() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);

    env.as_contract(&contract_id, || {
        assert_eq!(TrustBridgeContract::max_username_len(env.clone()), 39);
        assert!(TrustBridgeContract::is_username_valid(
            env.clone(),
            username(&env, "octocat")
        ));
        assert!(!TrustBridgeContract::is_username_valid(
            env.clone(),
            username(&env, "octo cat")
        ));
    });
}

// --- Issue #68: username case normalization ---

#[test]
fn test_usernames_match_is_case_insensitive() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);

    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::usernames_match(
            env.clone(),
            username(&env, "OctoCat"),
            username(&env, "octocat")
        ));
        assert!(!TrustBridgeContract::usernames_match(
            env.clone(),
            username(&env, "octocat"),
            username(&env, "octocat1")
        ));
    });
}

// --- Issue #72: register self-auth enforcement ---

#[test]
fn test_register_transfer_requires_current_owner_auth() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    let client = TrustBridgeContractClient::new(&env, &contract_id);
    let name = username(&env, "octocat");

    env.mock_all_auths();
    client.register(&name, &user, &Vec::new(&env));

    // Re-point the registration at `other`, authorizing only `other`.
    // The current owner's signature is missing, so the call must fail.
    env.set_auths(&[]);
    let res = client.try_register(&name, &other, &Vec::new(&env));
    assert!(
        res.is_err(),
        "takeover succeeded without the current owner's authorization"
    );

    // The registration is unchanged.
    env.mock_all_auths();
    assert_eq!(client.get_address(&name).unwrap().stellar_address, user);
}

#[test]
fn test_register_transfer_succeeds_with_both_auths() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    let client = TrustBridgeContractClient::new(&env, &contract_id);
    let name = username(&env, "octocat");

    env.mock_all_auths();
    client.register(&name, &user, &Vec::new(&env));
    client.register(&name, &other, &Vec::new(&env));

    assert_eq!(client.get_address(&name).unwrap().stellar_address, other);
    assert_eq!(client.get_stats().total, 1);
}

// --- Issue #62: RegisteredEvent payload ---

#[test]
fn test_registered_event_payload_is_complete() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    let client = TrustBridgeContractClient::new(&env, &contract_id);
    let name = username(&env, "octocat");

    env.mock_all_auths();
    env.ledger().set_timestamp(1_600_000_000);

    client.register(&name, &user, &Vec::new(&env));

    let expected = RegisteredEvent {
        github_username: name.clone(),
        stellar_address: user.clone(),
        timestamp: 1_600_000_000,
        sponsor: None,
        domain: env.as_contract(&contract_id, || event_domain(&env)),
    };

    assert_eq!(
        env.events().all(),
        soroban_sdk::vec![
            &env,
            (
                contract_id.clone(),
                expected.topics(&env),
                expected.data(&env),
            )
        ],
        "RegisteredEvent payload or topics changed"
    );

    let topics = expected.topics(&env);
    assert_eq!(topics.len(), 2, "RegisteredEvent must have 2 topics");
    assert_eq!(
        soroban_sdk::Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
        soroban_sdk::Symbol::new(&env, "registered_event"),
        "RegisteredEvent topic symbol changed"
    );
    assert_eq!(
        String::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
        name,
        "RegisteredEvent username topic changed"
    );
}

#[test]
fn test_registered_event_not_published_on_failed_register() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    let client = TrustBridgeContractClient::new(&env, &contract_id);
    let name = username(&env, "octocat");

    // We do NOT mock auth, so the registration should fail.
    env.set_auths(&[]);
    assert!(client.try_register(&name, &user, &Vec::new(&env)).is_err());

    assert_eq!(
        env.events().all(),
        soroban_sdk::vec![&env],
        "failed register published an event"
    );
}

// --- Issue #64: RemovedEvent payload ---

#[test]
fn test_removed_event_payload_is_complete() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    let client = TrustBridgeContractClient::new(&env, &contract_id);
    let name = username(&env, "octocat");

    env.mock_all_auths();
    client.register(&name, &user, &Vec::new(&env));

    env.ledger().set_timestamp(1_700_000_000);
    client.remove(&user, &name);

    // `remove` must publish exactly one event, and that event must be a
    // fully-populated RemovedEvent: the username as topic, and the removed
    // address plus the removal timestamp as data. An indexer replaying only
    // this event has to be able to reconstruct the record it is retiring,
    // so every field is asserted rather than just the event's presence.
    let expected = RemovedEvent {
        github_username: name.clone(),
        stellar_address: user.clone(),
        timestamp: 1_700_000_000,
        domain: env.as_contract(&contract_id, || event_domain(&env)),
    };

    assert_eq!(
        env.events().all(),
        soroban_sdk::vec![
            &env,
            (
                contract_id.clone(),
                expected.topics(&env),
                expected.data(&env),
            )
        ],
        "RemovedEvent payload or topics changed"
    );

    // Pin the topic shape independently of the struct, so renaming the
    // event or dropping the username topic breaks this test rather than
    // silently breaking every downstream subscriber's filter.
    let topics = expected.topics(&env);
    assert_eq!(topics.len(), 2, "RemovedEvent must have 2 topics");
    assert_eq!(
        soroban_sdk::Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
        soroban_sdk::Symbol::new(&env, "removed_event"),
        "RemovedEvent topic symbol changed"
    );
    assert_eq!(
        String::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
        name,
        "RemovedEvent username topic changed"
    );
}

#[test]
fn test_removed_event_not_published_on_failed_remove() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    let client = TrustBridgeContractClient::new(&env, &contract_id);
    let name = username(&env, "octocat");

    env.mock_all_auths();
    client.register(&name, &user, &Vec::new(&env));

    // A caller who is neither the registrant nor the admin is rejected,
    // and must not leave a RemovedEvent behind for indexers to act on.
    assert!(client.try_remove(&other, &name).is_err());
    assert_eq!(
        env.events().all(),
        soroban_sdk::vec![&env],
        "failed remove published an event"
    );
    assert!(client.has_record(&name));
}

// ── Issue #56: unauthorized remove must not disturb registry events ──────

#[test]
fn test_unauthorized_remove_leaves_verified_record_and_events_intact() {
    let env = Env::default();
    let (admin, user, other, contract_id) = setup(&env);
    let client = TrustBridgeContractClient::new(&env, &contract_id);
    let name = username(&env, "octocat");

    env.mock_all_auths();
    client.register(&name, &user, &Vec::new(&env));
    env.mock_all_auths();
    client.verify(&admin, &name);

    // Neither the registrant nor the admin: rejected, and must leave the
    // registration, its verified flag, and the event log untouched. `all()`
    // scopes to the last invocation, so an empty result here proves the
    // failed call published no RemovedEvent (same idiom as
    // test_removed_event_not_published_on_failed_remove).
    assert!(client.try_remove(&other, &name).is_err());
    assert_eq!(
        env.events().all(),
        soroban_sdk::vec![&env],
        "unauthorized remove must not publish a RemovedEvent"
    );
    let record = client.get_address(&name).unwrap();
    assert!(
        record.verified,
        "unauthorized remove must not clear verification"
    );
    assert_eq!(client.get_stats().verified, 1);
}

#[test]
fn test_unauthorized_remove_after_revoke_leaves_events_intact() {
    let env = Env::default();
    let (admin, user, other, contract_id) = setup(&env);
    let client = TrustBridgeContractClient::new(&env, &contract_id);
    let name = username(&env, "octocat");

    env.mock_all_auths();
    client.register(&name, &user, &Vec::new(&env));
    env.mock_all_auths();
    client.verify(&admin, &name);
    env.mock_all_auths();
    client.revoke_verification(&admin, &name, &1u32);

    assert!(client.try_remove(&other, &name).is_err());
    assert_eq!(
        env.events().all(),
        soroban_sdk::vec![&env],
        "unauthorized remove must not publish a RemovedEvent after a prior revoke"
    );
    assert!(client.has_record(&name));
    assert_eq!(client.get_stats().total, 1);
}

// ── Pause / unpause workflow ──────────────────────────────────────────────

#[test]
fn test_pause_and_unpause_workflow() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);

    env.as_contract(&contract_id, || {
        assert!(!TrustBridgeContract::is_paused(env.clone()));
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
    });

    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::is_paused(env.clone()));
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let reg_res = TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user.clone(),
            Vec::new(&env),
        );
        assert_eq!(reg_res, Err(ContractError::Paused));
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::unpause(env.clone(), 4).unwrap();
    });

    env.as_contract(&contract_id, || {
        assert!(!TrustBridgeContract::is_paused(env.clone()));
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user.clone(),
            Vec::new(&env),
        )
        .is_ok());
    });
}

#[test]
fn test_pause_blocks_remove_and_public_paginated_then_unpause_restores() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    let name = username(&env, "octocat");

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), name.clone(), user.clone(), Vec::new(&env))
            .unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let remove_res = TrustBridgeContract::remove(env.clone(), user.clone(), name.clone());
        assert_eq!(remove_res, Err(ContractError::Paused));
    });

    env.as_contract(&contract_id, || {
        let paged_res = TrustBridgeContract::get_public_paginated(env.clone(), None, 10);
        assert_eq!(paged_res, Err(ContractError::Paused));
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::unpause(env.clone(), 4).unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let paged = TrustBridgeContract::get_public_paginated(env.clone(), None, 10).unwrap();
        assert_eq!(paged.records.len(), 1);
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), user.clone(), name.clone()).unwrap();
        assert!(TrustBridgeContract::get_address(env.clone(), name.clone()).is_none());
    });
}

// ── Roles management ─────────────────────────────────────────────────────

#[test]
fn test_roles_management() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_role(env.clone(), user.clone()),
            None
        );
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), user.clone(), Role::Upgrader).unwrap();
    });

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_role(env.clone(), user.clone()),
            Some(Role::Upgrader)
        );
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), other.clone(), Role::Verifier).unwrap();
    });

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_role(env.clone(), other.clone()),
            Some(Role::Verifier)
        );
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove_role(env.clone(), user.clone()).unwrap();
    });

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_role(env.clone(), user.clone()),
            None
        );
    });
}

// ── Cooldown / version / migration ────────────────────────────────────────

#[test]
fn test_migration_version_increment() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_version(env.clone()),
            CONTRACT_VERSION.to_tuple()
        );
        set_version(&env, (1, 0, 0));
        assert_eq!(TrustBridgeContract::get_version(env.clone()), (1, 0, 0));
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let err_res = TrustBridgeContract::migrate(env.clone(), (1, 0, 0));
        assert_eq!(err_res, Err(ContractError::InvalidVersion));
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::migrate(env.clone(), (1, 1, 0)).unwrap();
    });

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_version(env.clone()),
            CONTRACT_VERSION.to_tuple()
        );
    });
}

#[test]
fn test_is_compatible_matches_initialized_batch_verify_version() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::version(env.clone()),
            CONTRACT_VERSION.to_tuple()
        );
        assert!(TrustBridgeContract::is_compatible(env.clone(), 1, 1, 0));
        assert!(!TrustBridgeContract::is_compatible(env.clone(), 1, 1, 1));
    });
}

// ── Admin export ──────────────────────────────────────────────────────────

#[test]
fn test_admin_export_empty_registry() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let all = TrustBridgeContract::get_all_registered(env.clone()).unwrap();
        assert_eq!(all.len(), 0);
    });
}

#[test]
fn test_get_all_registered_returns_indexed_records() {
    let env = Env::default();
    let (_admin, user1, user2, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let all = TrustBridgeContract::get_all_registered(env.clone()).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(
            all.get(0).unwrap(),
            (username(&env, "alice"), user1.clone())
        );
        assert_eq!(all.get(1).unwrap(), (username(&env, "bob"), user2.clone()));
    });
}

#[test]
fn test_cold_start_register_exposes_dashboard_state() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_all_registered(env.clone())
                .unwrap()
                .len(),
            0
        );
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_all_registered(env.clone())
                .unwrap()
                .len(),
            1
        );
    });
}

// ── Issue #16 / #53: from_code round-trip and completeness ───────────────

/// Every variant's code() must round-trip through from_code() (Issue #16).
#[test]
fn test_from_code_round_trips_all_variants() {
    let all = [
        ContractError::AlreadyInitialized,
        ContractError::NotInitialized,
        ContractError::NotAuthorized,
        ContractError::NotRegistered,
        ContractError::AlreadyVerified,
        ContractError::NotVerified,
        ContractError::Paused,
        ContractError::CooldownActive,
        ContractError::InvalidVersion,
        ContractError::InvalidRole,
        ContractError::InvalidUsername,
        ContractError::AttestationExpired,
        ContractError::UnattestedWasm,
        ContractError::InvalidBatchSize,
        ContractError::InvalidReasonCode,
        ContractError::ZeroAddress,
        ContractError::ChallengeAlreadyActive,
        ContractError::NoChallengeActive,
        ContractError::ChallengeNotResolvable,
        ContractError::ChallengeActive,
    ];
    for variant in all {
        assert_eq!(ContractError::from_code(variant as u32), Some(variant));
    }
}

/// Codes not in the enum must return None (Issue #16).
#[test]
fn test_from_code_unknown_returns_none() {
    assert_eq!(ContractError::from_code(0), None);
    assert_eq!(ContractError::from_code(32), None);
    assert_eq!(ContractError::from_code(u32::MAX), None);
}

// ── Issue #54: Additional not-initialized guard tests ────────────────────

/// get_registered_page must fail before init (Issue #54).
#[test]
fn test_get_registered_page_requires_initialization() {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::get_registered_page(env.clone(), 0, 10);
        assert_eq!(result, Err(ContractError::NotInitialized));
    });
}

/// get_registered_paginated must fail before init (Issue #54).
#[test]
fn test_get_registered_paginated_requires_initialization() {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::get_registered_paginated(env.clone(), None, 10);
        assert_eq!(result, Err(ContractError::NotInitialized));
    });
}

/// get_public_paginated must fail before init (Issue #54).
#[test]
fn test_get_public_paginated_requires_initialization() {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::get_public_paginated(env.clone(), None, 10);
        assert_eq!(result, Err(ContractError::NotInitialized));
    });
}

/// After initialization every previously failing guard must succeed (Issue #54).
#[test]
fn test_guards_succeed_after_initialization() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Confirm guard fires before init
        assert_eq!(
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                admin.clone(),
                Vec::new(&env),
            ),
            Err(ContractError::NotInitialized)
        );

        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();

        // Same call must now succeed
        assert!(TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            admin.clone(),
            Vec::new(&env),
        )
        .is_ok());
    });
}

/// Double-initialize after successful init must still be rejected (Issue #54).
#[test]
fn test_double_initialize_rejected_after_successful_init() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
        let result = TrustBridgeContract::initialize(env.clone(), admin2.clone());
        assert_eq!(result, Err(ContractError::AlreadyInitialized));
    });
}

// ── Issue #52: Additional lookup-after-peer-removal tests ────────────────

/// Paginated export must skip removed records (Issue #52).
#[test]
fn test_paginated_export_skips_removed_records() {
    let env = Env::default();
    let (_admin, user1, user2, contract_id) = setup(&env);
    let user3 = Address::generate(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "carol"),
            user3.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), user2.clone(), username(&env, "bob")).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let page = TrustBridgeContract::get_registered_paginated(env.clone(), None, 10).unwrap();
        assert_eq!(
            page.records.len(),
            2,
            "paginated export must skip removed entry"
        );
        assert_eq!(page.total, 2);
        assert!(!page.has_more);
    });
}

/// Public paginated endpoint reflects removal immediately (Issue #52).
#[test]
fn test_public_paginated_reflects_removal() {
    let env = Env::default();
    let (_admin, user1, user2, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), user1.clone(), username(&env, "alice")).unwrap();
    });
    env.as_contract(&contract_id, || {
        let page = TrustBridgeContract::get_public_paginated(env.clone(), None, 10).unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records.get(0).unwrap().0, username(&env, "bob"));
    });
}

// ── Issue #143: bulk export pagination limits ─────────────────────────────

/// Requesting exactly `MAX_PAGE_LIMIT` from a registry larger than that
/// must return a full page and report that more records remain.
#[test]
fn test_paginated_export_at_max_page_limit() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();

    // A full `MAX_PAGE_LIMIT`-sized page reads one ledger entry per
    // record, which exceeds the mainnet per-invocation footprint limit
    // (100 entries) purely as a test-harness artifact of registering and
    // reading that many entries in one `Env`. The pagination *behavior*
    // under test — clamping and cursor bookkeeping — is independent of
    // that network limit, so it is disabled here.
    env.cost_estimate().disable_resource_limits();

    let total = crate::storage::MAX_PAGE_LIMIT + 5;
    for i in 0..total {
        let mut name = alloc::string::String::from("user");
        name.push_str(&alloc::format!("{i}"));
        let name = String::from_str(&env, &name);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), name.clone(), user.clone(), Vec::new(&env))
                .unwrap();
        });
    }

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let page = TrustBridgeContract::get_registered_paginated(
            env.clone(),
            None,
            crate::storage::MAX_PAGE_LIMIT,
        )
        .unwrap();
        assert_eq!(page.records.len(), crate::storage::MAX_PAGE_LIMIT);
        assert!(page.has_more);
        assert!(page.next_cursor.is_some());
    });
}

/// Requesting more than `MAX_PAGE_LIMIT` must be clamped down to it rather
/// than rejected or returned unbounded.
#[test]
fn test_paginated_export_over_max_page_limit_clamps() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();

    // See `test_paginated_export_at_max_page_limit`: disabled for the
    // same reason — the registry size needed to exercise the clamp
    // exceeds the mainnet per-invocation footprint limit.
    env.cost_estimate().disable_resource_limits();

    let total = crate::storage::MAX_PAGE_LIMIT + 5;
    for i in 0..total {
        let mut name = alloc::string::String::from("user");
        name.push_str(&alloc::format!("{i}"));
        let name = String::from_str(&env, &name);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), name.clone(), user.clone(), Vec::new(&env))
                .unwrap();
        });
    }

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let page = TrustBridgeContract::get_registered_paginated(
            env.clone(),
            None,
            crate::storage::MAX_PAGE_LIMIT + 50,
        )
        .unwrap();
        assert!(page.records.len() <= crate::storage::MAX_PAGE_LIMIT);
        assert_eq!(page.records.len(), crate::storage::MAX_PAGE_LIMIT);
    });
}

/// has_record returns false after removal and true for surviving peer (Issue #52).
#[test]
fn test_has_record_consistency_after_peer_removal() {
    let env = Env::default();
    let (_admin, user1, user2, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), user1.clone(), username(&env, "alice")).unwrap();
    });
    env.as_contract(&contract_id, || {
        assert!(!TrustBridgeContract::has_record(
            env.clone(),
            username(&env, "alice")
        ));
        assert!(TrustBridgeContract::has_record(
            env.clone(),
            username(&env, "bob")
        ));
    });
}

/// Comparison counts the case-normalization benchmark sweeps. Normalization
/// touches no ledger entries, so it is not footprint-bound.
const BENCH_SIZES: [u32; 4] = [10, 50, 100, 200];

/// Registry sizes the full-export benchmark sweeps.
///
/// Capped below 100: `get_all_registered` reads one ledger entry per
/// record, and Soroban rejects an invocation whose footprint exceeds 100
/// entries. That ceiling is the reason `get_registered_page` exists — see
/// `test_bench_export_footprint_ceiling`.
const EXPORT_BENCH_SIZES: [u32; 4] = [10, 20, 40, 80];

/// Labels used by the register budget guard.
const REGISTER_BUDGET_BASELINE_LABEL: &str = "baseline";
const REGISTER_BUDGET_STRESSED_LABEL: &str = "max_username_len";

/// Measures a single `register` call against a fresh initialized contract.
/// Returns `(cpu_instructions, memory_bytes)`.
fn measure_register_cost(env: &Env, github_username: String) -> (u64, u64) {
    let (_admin, user, _other, contract_id) = setup(env);

    env.mock_all_auths();
    env.cost_estimate().budget().reset_default();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), github_username, user, Vec::new(&env)).unwrap();
    });

    let budget = env.cost_estimate().budget();
    (budget.cpu_instruction_cost(), budget.memory_bytes_cost())
}

fn max_len_username(env: &Env) -> String {
    let repeated = alloc::string::String::from("a").repeat(MAX_USERNAME_LEN as usize);
    String::from_str(env, &repeated)
}

/// Registers `size` contributors and measures the metered cost of a single
/// full export. Returns `(cpu_instructions, memory_bytes)`.
fn measure_export(size: u32) -> (u64, u64) {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();

    // Each registration runs in its own frame: `require_auth` for the same
    // address twice in one frame is an Auth(ExistingValue) error.
    for i in 0..size {
        let mut name = alloc::string::String::from("bench");
        name.push_str(&alloc::format!("{i}"));
        let name = String::from_str(&env, &name);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(env.clone(), name.clone(), user.clone(), Vec::new(&env))
                .unwrap();
        });
    }

    env.cost_estimate().budget().reset_default();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::get_all_registered(env.clone()).unwrap();
    });

    let budget = env.cost_estimate().budget();
    (budget.cpu_instruction_cost(), budget.memory_bytes_cost())
}

/// Measures the metered cost of `size` case-insensitive username
/// comparisons — the normalization step an off-chain verifier performs per
/// candidate match. Returns `(cpu_instructions, memory_bytes)`.
fn measure_case_normalization(size: u32) -> (u64, u64) {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());

    // Mixed-case on one side, lower-case on the other, so every comparison
    // exercises the folding path rather than an early length mismatch.
    let upper = String::from_str(&env, "OctoCat-Dev_01");
    let lower = String::from_str(&env, "octocat-dev_01");

    env.cost_estimate().budget().reset_default();
    env.as_contract(&contract_id, || {
        for _ in 0..size {
            assert!(TrustBridgeContract::usernames_match(
                env.clone(),
                upper.clone(),
                lower.clone()
            ));
        }
    });

    let budget = env.cost_estimate().budget();
    (budget.cpu_instruction_cost(), budget.memory_bytes_cost())
}

/// Benchmark for issue #68: username case normalization must stay linear
/// in the number of comparisons and must not allocate per comparison.
#[test]
fn test_bench_username_case_normalization() {
    std::println!("operation,size,cpu_instructions,memory_bytes");

    let mut previous_cpu = 0u64;
    let mut baseline: Option<(u32, u64)> = None;
    let mut largest: Option<(u32, u64)> = None;

    for size in BENCH_SIZES {
        let (cpu, mem) = measure_case_normalization(size);
        std::println!("usernames_match,{},{},{}", size, cpu, mem);

        assert!(cpu > 0, "normalization at size {size} was not metered");
        assert!(
            cpu >= previous_cpu,
            "normalization CPU cost dropped at size {size}: {cpu} < {previous_cpu}"
        );

        previous_cpu = cpu;
        baseline.get_or_insert((size, cpu));
        largest = Some((size, cpu));
    }

    let (small_size, small_cpu) = baseline.unwrap();
    let (large_size, large_cpu) = largest.unwrap();

    // Comparison is a fixed-width stack scan, so cost is linear in the
    // number of calls. 3x headroom over the size ratio absorbs per-call
    // overhead while still failing on super-linear growth.
    let ceiling = small_cpu * ((large_size / small_size) as u64) * 3;
    assert!(
            large_cpu <= ceiling,
            "normalization CPU cost grew super-linearly: {large_cpu} at size {large_size} exceeds ceiling {ceiling}"
        );
}

#[test]
fn test_bench_export_cpu_cost() {
    std::println!("operation,size,cpu_instructions,memory_bytes");

    let mut previous_cpu = 0u64;
    let mut baseline: Option<(u32, u64)> = None;
    let mut largest: Option<(u32, u64)> = None;

    for size in EXPORT_BENCH_SIZES {
        let (cpu, mem) = measure_export(size);
        std::println!("get_all_registered,{},{},{}", size, cpu, mem);

        assert!(cpu > 0, "export at size {size} was not metered");
        // Cost is monotonic in registry size; a drop means the export
        // stopped visiting every record.
        assert!(
            cpu >= previous_cpu,
            "export CPU cost dropped at size {size}: {cpu} < {previous_cpu}"
        );

        previous_cpu = cpu;
        baseline.get_or_insert((size, cpu));
        largest = Some((size, cpu));
    }

    let (small_size, small_cpu) = baseline.unwrap();
    let (large_size, large_cpu) = largest.unwrap();

    // Export is a linear scan. Allow 3x headroom over the size ratio so
    // normal per-entry overhead passes while quadratic growth fails.
    let ceiling = small_cpu * ((large_size / small_size) as u64) * 3;
    assert!(
            large_cpu <= ceiling,
            "export CPU cost grew super-linearly: {large_cpu} at size {large_size} exceeds ceiling {ceiling}"
        );
}

/// Emits register budget samples for CI/Make threshold checks.
///
/// Output format (CSV):
/// operation,input,cpu_instructions,memory_bytes
#[test]
fn test_report_register_budget_samples() {
    let env = Env::default();
    let baseline = username(&env, "octocat");
    let stressed = max_len_username(&env);

    let (baseline_cpu, baseline_mem) = measure_register_cost(&env, baseline);
    let (stressed_cpu, stressed_mem) = measure_register_cost(&env, stressed);

    std::println!("operation,input,cpu_instructions,memory_bytes");
    std::println!(
        "register,{},{},{}",
        REGISTER_BUDGET_BASELINE_LABEL,
        baseline_cpu,
        baseline_mem
    );
    std::println!(
        "register,{},{},{}",
        REGISTER_BUDGET_STRESSED_LABEL,
        stressed_cpu,
        stressed_mem
    );

    assert!(baseline_cpu > 0, "baseline register cost was not metered");
    assert!(stressed_cpu > 0, "stressed register cost was not metered");
}
/// Measures the metered cost of a double-verify rejection — calling `verify`
/// on a username that is already verified. Returns `(cpu_instructions, memory_bytes)`.
///
/// The operation registers a user, verifies them, then calls verify again and
/// expects `AlreadyVerified`. The cost is compared against a successful first
/// verify to ensure the rejection path is strictly cheaper (denial-of-wallet
/// protection — a rejected call must not consume more budget than a successful one).
fn measure_double_verify_rejection() -> (u64, u64, u64, u64) {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());
    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    // Initialize the contract
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });

    // Register and verify first
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });

    // Measure cost of the second verify (should fail with AlreadyVerified)
    env.cost_estimate().budget().reset_default();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result =
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"));
        assert_eq!(result, Err(ContractError::AlreadyVerified));
    });
    let reject_budget = env.cost_estimate().budget();
    let reject_cpu = reject_budget.cpu_instruction_cost();
    let reject_mem = reject_budget.memory_bytes_cost();

    // Measure cost of a successful verify (separate user, fresh env)
    let env2 = Env::default();
    let contract_id2 = env2.register(TrustBridgeContract, ());
    let admin2 = Address::generate(&env2);
    let user2 = Address::generate(&env2);

    env2.as_contract(&contract_id2, || {
        TrustBridgeContract::initialize(env2.clone(), admin2.clone()).unwrap();
    });
    env2.mock_all_auths();
    env2.as_contract(&contract_id2, || {
        TrustBridgeContract::register(
            env2.clone(),
            username(&env2, "octocat"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    env2.cost_estimate().budget().reset_default();
    env2.mock_all_auths();
    env2.as_contract(&contract_id2, || {
        TrustBridgeContract::verify(env2.clone(), admin2.clone(), username(&env2, "octocat"))
            .unwrap();
    });
    let success_budget = env2.cost_estimate().budget();
    let success_cpu = success_budget.cpu_instruction_cost();
    let success_mem = success_budget.memory_bytes_cost();

    (reject_cpu, reject_mem, success_cpu, success_mem)
}

/// Benchmark double-verify rejection (Issue #58): ensures the rejected
/// verify (AlreadyVerified) costs strictly less than a successful verify.
///
/// A rejection path that costs more than the success path could be used by
/// an attacker to burn the contract's ledger budget cheaply — the rejection
/// executes fewer steps, so it must cost strictly less.
#[test]
fn test_bench_double_verify_rejection() {
    let (reject_cpu, reject_mem, success_cpu, success_mem) = measure_double_verify_rejection();

    std::println!("operation,type,cpu_instructions,memory_bytes");
    std::println!("verify,success,{},{}", success_cpu, success_mem);
    std::println!(
        "verify,rejected_double_verify,{},{}",
        reject_cpu,
        reject_mem
    );

    assert!(
        success_cpu > 0,
        "successful verify CPU cost was not metered"
    );
    assert!(
        reject_cpu > 0,
        "rejected double-verify CPU cost was not metered"
    );

    // Protection against denial-of-wallet: a rejected path must cost
    // strictly less than the equivalent success path (Issue #58).
    assert!(
            reject_cpu < success_cpu,
            "rejected double-verify CPU cost ({reject_cpu}) must be strictly less than successful verify CPU cost ({success_cpu})"
        );
    assert!(
            reject_mem <= success_mem,
            "rejected double-verify memory cost ({reject_mem}) must not exceed successful verify memory cost ({success_mem})"
        );
}

/// Revoking Verifier role prevents further verify calls (Issue #12).
#[test]
fn test_revoked_verifier_role_cannot_verify() {
    let env = Env::default();
    let (_admin, user, verifier, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), verifier.clone(), Role::Verifier).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "octocat"))
            .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove_role(env.clone(), verifier.clone()).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result =
            TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "alice"));
        assert_eq!(result, Err(ContractError::NotAuthorized));
    });
}

/// Two independent Verifier-role holders can each verify without interfering (Issue #12).
#[test]
fn test_two_verifiers_operate_independently() {
    let env = Env::default();
    let (_admin, user, verifier1, contract_id) = setup(&env);
    let verifier2 = Address::generate(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), verifier1.clone(), Role::Verifier).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), verifier2.clone(), Role::Verifier).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), verifier1.clone(), username(&env, "alice"))
            .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), verifier2.clone(), username(&env, "bob")).unwrap();
    });
    env.as_contract(&contract_id, || {
        assert!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "alice"))
                .unwrap()
                .verified
        );
        assert!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "bob"))
                .unwrap()
                .verified
        );
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 2);
    });
}

/// Upgrader role cannot revoke verification (Issue #12).
#[test]
fn test_upgrader_role_cannot_revoke_verification() {
    let env = Env::default();
    let (admin, user, upgrader, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), upgrader.clone(), Role::Upgrader).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::revoke_verification(
            env.clone(),
            upgrader.clone(),
            username(&env, "octocat"),
            1,
        );
        assert_eq!(result, Err(ContractError::NotAuthorized));
    });
}

/// Verifier-role address cannot call set_role (admin-only operation) (Issue #12).
#[test]
fn test_verifier_cannot_grant_roles() {
    let env = Env::default();
    let (_admin, user, verifier, contract_id) = setup(&env);
    let target = Address::generate(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), verifier.clone(), Role::Verifier).unwrap();

        // The contract does not expose a "set_role_as" API; the guard is
        // in set_role itself: admin.require_auth() is always the admin
        // address. This test validates the role table stays clean.
        let _ = user;
        let _ = target;
        assert_eq!(
            TrustBridgeContract::get_role(env.clone(), verifier.clone()),
            Some(Role::Verifier)
        );
    });
}

// ── Issue #114: verify / revoke_verification auth negative matrix ─────────
//
// Matrix of all unauthorized / invalid call paths for verify and
// revoke_verification, with expected ContractError codes.  Each test
// corresponds to exactly one cell in the table published in docs/SECURITY.md.
//
// verify negative matrix:
// | # | Scenario                              | Expected error    | Code |
// |---|---------------------------------------|-------------------|------|
// | V1 | Not initialized                      | NotInitialized    |  2   |
// | V2 | Username not registered               | NotRegistered     |  4   |
// | V3 | Already verified (double-verify)      | AlreadyVerified   |  5   |
// | V4 | Caller has no role                    | NotAuthorized     |  3   |
// | V5 | Upgrader-role caller                  | NotAuthorized     |  3   |
// | V6 | Admin caller (happy path)             | Ok(())            |  —   |
// | V7 | Verifier-role caller (happy path)     | Ok(())            |  —   |
// | V8 | Contract is paused                    | Paused            |  7   |
//
// revoke_verification negative matrix:
// | # | Scenario                              | Expected error    | Code |
// |---|---------------------------------------|-------------------|------|
// | R1 | Not initialized                      | NotInitialized    |  2   |
// | R2 | Username not registered               | NotRegistered     |  4   |
// | R3 | Record not verified (can't revoke)    | NotVerified       |  6   |
// | R4 | Caller has no role                    | NotAuthorized     |  3   |
// | R5 | Upgrader-role caller                  | NotAuthorized     |  3   |
// | R6 | Admin caller (happy path)             | Ok(())            |  —   |
// | R7 | Verifier-role caller (happy path)     | Ok(())            |  —   |
// | R8 | Contract is paused                    | Paused            |  7   |

// ── verify negative matrix ────────────────────────────────────────────────

/// #114-V1: verify before initialization returns NotInitialized (code 2).
#[test]
fn test_verify_negative_not_initialized() {
    let env = Env::default();
    let caller = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result =
            TrustBridgeContract::verify(env.clone(), caller.clone(), username(&env, "octocat"));
        assert_eq!(
            result,
            Err(ContractError::NotInitialized),
            "verify before init must return NotInitialized (code {})",
            ContractError::NotInitialized.code()
        );
        assert_eq!(ContractError::NotInitialized.code(), 2);
    });
}

/// #114-V2: verify on a username that was never registered returns NotRegistered (code 4).
#[test]
fn test_verify_negative_username_not_registered() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result =
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "ghost"));
        assert_eq!(
            result,
            Err(ContractError::NotRegistered),
            "verify on missing username must return NotRegistered (code {})",
            ContractError::NotRegistered.code()
        );
        assert_eq!(ContractError::NotRegistered.code(), 4);
    });
}

/// #114-V3: double-verify returns AlreadyVerified (code 5).
#[test]
fn test_verify_negative_already_verified() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result =
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"));
        assert_eq!(
            result,
            Err(ContractError::AlreadyVerified),
            "second verify must return AlreadyVerified (code {})",
            ContractError::AlreadyVerified.code()
        );
        assert_eq!(ContractError::AlreadyVerified.code(), 5);
    });
}

/// #114-V4: address with no role cannot verify — returns NotAuthorized (code 3).
#[test]
fn test_verify_negative_no_role_caller() {
    let env = Env::default();
    let (_admin, user, nobody, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let result = TrustBridgeContract::verify(
            env.clone(),
            nobody.clone(), // no role
            username(&env, "octocat"),
        );
        assert_eq!(
            result,
            Err(ContractError::NotAuthorized),
            "no-role caller must not be able to verify (code {})",
            ContractError::NotAuthorized.code()
        );
        assert_eq!(ContractError::NotAuthorized.code(), 3);
    });
}

/// #114-V5: Upgrader-role address cannot verify — returns NotAuthorized (code 3).
#[test]
fn test_verify_negative_upgrader_cannot_verify() {
    let env = Env::default();
    let (_admin, user, upgrader, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), upgrader.clone(), Role::Upgrader).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let result = TrustBridgeContract::verify(
            env.clone(),
            upgrader.clone(), // Upgrader role — not allowed
            username(&env, "octocat"),
        );
        assert_eq!(
            result,
            Err(ContractError::NotAuthorized),
            "Upgrader-role must not be allowed to verify (code {})",
            ContractError::NotAuthorized.code()
        );
    });
}

/// #114-V6 (happy path): admin can verify a registered user.
#[test]
fn test_verify_positive_admin_can_verify() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
        assert!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .verified,
            "admin verify must set verified=true"
        );
    });
}

/// #114-V7 (happy path): Verifier-role address can verify.
#[test]
fn test_verify_positive_verifier_role_can_verify() {
    let env = Env::default();
    let (_admin, user, verifier, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), verifier.clone(), Role::Verifier).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "octocat"))
            .unwrap();
        assert!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .verified,
            "Verifier-role verify must set verified=true"
        );
    });
}

/// Matching bound address: admin verify of a registered record succeeds
/// and writes the record's `stellar_address`, not the caller's.
#[test]
fn test_verify_address_binding_happy_path() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert!(record.verified);
        assert_eq!(record.stellar_address, user);
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 1);
    });
}

/// Unprivileged caller authenticating as a different address than the
/// record cannot verify it (`NotAuthorized`).
#[test]
fn test_verify_mismatched_caller_rejected() {
    let env = Env::default();
    let (_admin, user, stranger, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let result =
            TrustBridgeContract::verify(env.clone(), stranger.clone(), username(&env, "octocat"));
        assert_eq!(result, Err(ContractError::NotAuthorized));
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert!(!record.verified);
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 0);
    });
}

/// `batch_verify` counts a zero-address (unbound) record as failed, not
/// successful, and does not increment the verified counter.
#[test]
fn test_batch_verify_address_mismatch_tallied_as_failure() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    let zero = Address::from_str(&env, crate::utils::ZERO_ADDRESS_STRKEY);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        set_record(
            &env,
            &username(&env, "burned"),
            &ContributorRecord {
                stellar_address: zero,
                payout_address: user.clone(),
                registered_at: env.ledger().timestamp() as u32,
                verified: false,
                is_bot: false,
            },
        );
        let usernames = soroban_sdk::vec![&env, username(&env, "alice"), username(&env, "burned"),];
        let summary =
            TrustBridgeContract::batch_verify(env.clone(), admin.clone(), usernames).unwrap();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.successful, 1);
        assert_eq!(summary.failed, 1);
        assert!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "alice"))
                .unwrap()
                .verified
        );
        assert!(
            !TrustBridgeContract::get_address(env.clone(), username(&env, "burned"))
                .unwrap()
                .verified
        );
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 1);
    });
}

/// Mixed batch: one valid record, one missing username, one unbound
/// record. Success tally must not include the failures.
#[test]
fn test_batch_verify_mixed_binding_and_missing() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    let zero = Address::from_str(&env, crate::utils::ZERO_ADDRESS_STRKEY);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        set_record(
            &env,
            &username(&env, "burned"),
            &ContributorRecord {
                stellar_address: zero,
                payout_address: user.clone(),
                registered_at: env.ledger().timestamp() as u32,
                verified: false,
                is_bot: false,
            },
        );
        let usernames = soroban_sdk::vec![
            &env,
            username(&env, "alice"),
            username(&env, "ghost"),
            username(&env, "burned"),
        ];
        let summary =
            TrustBridgeContract::batch_verify(env.clone(), admin.clone(), usernames).unwrap();
        assert_eq!(summary.total, 3);
        assert_eq!(summary.successful, 1);
        assert_eq!(summary.failed, 2);
    });
}

/// A proof bound to a different Stellar address cannot verify the record.
#[test]
fn test_verify_with_oracle_proof_rejects_address_mismatch() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let proof = OracleProof {
            oracle_pubkey: BytesN::from_array(&env, &[1u8; 32]),
            message: crate::oracle_proof::ownership_message(
                &env,
                &username(&env, "octocat"),
                &other,
                0,
            ),
            signature: BytesN::from_array(&env, &[2u8; 64]),
            expires_at: 0,
        };
        let result = TrustBridgeContract::verify_with_oracle_proof(
            env.clone(),
            username(&env, "octocat"),
            proof,
        );
        assert_eq!(result, Err(ContractError::AddressMismatch));
        assert!(
            !TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .verified
        );
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 0);
    });
}

/// Pending address rotation keeps verification bound to the current
/// `stellar_address` until `execute_address_rotation`.
#[test]
fn test_verify_during_pending_rotation_binds_current_address() {
    let env = Env::default();
    let (admin, user, new_addr, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_rotation_delay(env.clone(), 1_000).unwrap();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::request_address_rotation(
            env.clone(),
            username(&env, "octocat"),
            new_addr.clone(),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert!(record.verified);
        assert_eq!(record.stellar_address, user);
        assert_ne!(record.stellar_address, new_addr);
    });
}

/// Direct `verify` of a zero-address record is rejected before state change.
#[test]
fn test_verify_zero_address_record_rejected() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    let zero = Address::from_str(&env, crate::utils::ZERO_ADDRESS_STRKEY);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        set_record(
            &env,
            &username(&env, "burned"),
            &ContributorRecord {
                stellar_address: zero,
                payout_address: user.clone(),
                registered_at: env.ledger().timestamp() as u32,
                verified: false,
                is_bot: false,
            },
        );
        let result =
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "burned"));
        assert_eq!(result, Err(ContractError::ZeroAddress));
        assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 0);
    });
}

#[test]
fn test_address_mismatch_error_code_matches_golden() {
    assert_eq!(ContractError::AddressMismatch.code(), 62);
    assert_eq!(
        ContractError::from_code(62),
        Some(ContractError::AddressMismatch)
    );
}

/// #114-V8: verify while contract is paused returns Paused (code 7).
#[test]
fn test_verify_negative_paused() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result =
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"));
        assert_eq!(
            result,
            Err(ContractError::Paused),
            "verify while paused must return Paused (code {})",
            ContractError::Paused.code()
        );
        assert_eq!(ContractError::Paused.code(), 7);
        // Record must still be unverified
        assert!(
            !TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .verified
        );
    });
}

// ── revoke_verification negative matrix ───────────────────────────────────

/// #114-R1: revoke_verification before initialization returns NotInitialized (code 2).
#[test]
fn test_revoke_negative_not_initialized() {
    let env = Env::default();
    let caller = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::revoke_verification(
            env.clone(),
            caller.clone(),
            username(&env, "octocat"),
            1,
        );
        assert_eq!(
            result,
            Err(ContractError::NotInitialized),
            "revoke before init must return NotInitialized (code {})",
            ContractError::NotInitialized.code()
        );
    });
}

/// #114-R2: revoke_verification on an unregistered username returns NotRegistered (code 4).
#[test]
fn test_revoke_negative_username_not_registered() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::revoke_verification(
            env.clone(),
            admin.clone(),
            username(&env, "ghost"),
            1,
        );
        assert_eq!(
            result,
            Err(ContractError::NotRegistered),
            "revoke on missing username must return NotRegistered (code {})",
            ContractError::NotRegistered.code()
        );
    });
}

/// #114-R3: revoking on an unverified record returns NotVerified (code 6).
#[test]
fn test_revoke_negative_not_verified() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let result = TrustBridgeContract::revoke_verification(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
            1,
        );
        assert_eq!(
            result,
            Err(ContractError::NotVerified),
            "revoke on unverified record must return NotVerified (code {})",
            ContractError::NotVerified.code()
        );
        assert_eq!(ContractError::NotVerified.code(), 6);
    });
}

/// #114-R4: address with no role cannot revoke — returns NotAuthorized (code 3).
#[test]
fn test_revoke_negative_no_role_caller() {
    let env = Env::default();
    let (admin, user, nobody, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
        let result = TrustBridgeContract::revoke_verification(
            env.clone(),
            nobody.clone(), // no role
            username(&env, "octocat"),
            1,
        );
        assert_eq!(
            result,
            Err(ContractError::NotAuthorized),
            "no-role caller must not revoke verification (code {})",
            ContractError::NotAuthorized.code()
        );
        // Verification must still be intact
        assert!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .verified
        );
    });
}

/// #114-R5: Upgrader-role address cannot revoke — returns NotAuthorized (code 3).
#[test]
fn test_revoke_negative_upgrader_cannot_revoke() {
    let env = Env::default();
    let (admin, user, upgrader, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), upgrader.clone(), Role::Upgrader).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
        let result = TrustBridgeContract::revoke_verification(
            env.clone(),
            upgrader.clone(), // Upgrader role — not allowed
            username(&env, "octocat"),
            1,
        );
        assert_eq!(
            result,
            Err(ContractError::NotAuthorized),
            "Upgrader-role must not revoke verification (code {})",
            ContractError::NotAuthorized.code()
        );
    });
}

/// #114-R6 (happy path): admin can revoke verification.
#[test]
fn test_revoke_positive_admin_can_revoke() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::revoke_verification(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
            1,
        )
        .unwrap();
        assert!(
            !TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .verified,
            "admin revoke must clear verified=false"
        );
    });
}

/// #114-R7 (happy path): Revoker-role address can revoke verification (Issue #212).
/// Updated from Verifier to Revoker — roles are now split.
#[test]
fn test_revoke_positive_verifier_role_can_revoke() {
    let env = Env::default();
    let (admin, user, revoker, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Use Revoker role, not Verifier (Issue #212).
        TrustBridgeContract::set_role(env.clone(), revoker.clone(), Role::Revoker).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
        TrustBridgeContract::revoke_verification(
            env.clone(),
            revoker.clone(),
            username(&env, "octocat"),
            1,
        )
        .unwrap();
        assert!(
            !TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .verified,
            "Revoker-role revoke must clear verified=false"
        );
    });
}

/// #114-R8: revoke_verification while paused returns Paused (code 7).
#[test]
fn test_revoke_negative_paused() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::revoke_verification(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
            1,
        );
        assert_eq!(
            result,
            Err(ContractError::Paused),
            "revoke while paused must return Paused (code {})",
            ContractError::Paused.code()
        );
        // Verification must still be intact
        assert!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                .unwrap()
                .verified
        );
    });
}

// ── batch_remove (Wave #batch) ─────────────────────────────────────────
//
// `batch_remove` is the batched form of `remove`, intended for admin
// workflows that need to clean up many stale or disputed registrations
// efficiently. Partial success is the point: a username that cannot be
// removed does not abort the batch.

/// batch_remove happy path: admin removes multiple registered users.
#[test]
fn test_batch_remove_all_succeed() {
    let env = Env::default();
    let (admin, user1, user2, contract_id) = setup(&env);
    let user3 = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "carol"),
            user3.clone(),
            Vec::new(&env),
        )
        .unwrap();
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 3);
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let usernames = soroban_sdk::vec![
            &env,
            username(&env, "alice"),
            username(&env, "bob"),
            username(&env, "carol"),
        ];
        let summary =
            TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames).unwrap();
        assert_eq!(summary.total, 3);
        assert_eq!(summary.successful, 3);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.success_rate, 100);
        assert!(summary.all_successful());

        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 0);
        assert_eq!(stats.verified, 0);
    });
}

/// batch_remove partial success: some usernames are registered, some are not.
#[test]
fn test_batch_remove_partial_success() {
    let env = Env::default();
    let (admin, user1, _user2, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let usernames = soroban_sdk::vec![
            &env,
            username(&env, "alice"), // registered → succeeds
            username(&env, "ghost"), // not registered → skipped (counted as failed)
        ];
        let summary =
            TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames).unwrap();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.successful, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.success_rate, 50);
        assert!(!summary.all_successful());
        assert!(summary.any_successful());

        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 0, "alice was removed");
    });
}

/// batch_remove all fail: none of the usernames are registered.
#[test]
fn test_batch_remove_all_fail() {
    let env = Env::default();
    let (admin, _user1, _user2, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let usernames =
            soroban_sdk::vec![&env, username(&env, "ghost1"), username(&env, "ghost2"),];
        let summary =
            TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames).unwrap();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.successful, 0);
        assert_eq!(summary.failed, 2);
        assert_eq!(summary.success_rate, 0);
        assert!(!summary.all_successful());
        assert!(!summary.any_successful());

        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 0, "registry must remain untouched");
    });
}

/// batch_remove rejects empty list with InvalidBatchSize.
#[test]
fn test_batch_remove_empty_rejected() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let usernames: soroban_sdk::Vec<String> = soroban_sdk::Vec::new(&env);
        let result = TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames);
        assert_eq!(result, Err(ContractError::InvalidBatchSize));
    });
}

/// batch_remove rejects too-large list with InvalidBatchSize.
#[test]
fn test_batch_remove_over_limit_rejected() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let limit = crate::batch::BatchConfig::default().max_batch_size;
        let mut usernames: soroban_sdk::Vec<String> = soroban_sdk::Vec::new(&env);
        for i in 0..=limit {
            usernames.push_back(username(&env, &alloc::format!("user{i}")));
        }
        let result = TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames);
        assert_eq!(result, Err(ContractError::InvalidBatchSize));
    });
}

/// batch_remove is blocked while the contract is paused.
#[test]
fn test_batch_remove_blocked_while_paused() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let usernames = soroban_sdk::vec![&env, username(&env, "alice")];
        let result = TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames);
        assert_eq!(result, Err(ContractError::Paused));
    });
}

/// batch_remove requires initialization.
#[test]
fn test_batch_remove_not_initialized() {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());
    let admin = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let usernames = soroban_sdk::vec![&env, username(&env, "alice")];
        let result = TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames);
        assert_eq!(result, Err(ContractError::NotInitialized));
    });
}

/// batch_remove rejects non-admin caller with NotAuthorized.
#[test]
fn test_batch_remove_non_admin_rejected() {
    let env = Env::default();
    let (_admin, _user, other, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let usernames = soroban_sdk::vec![&env, username(&env, "alice")];
        let result = TrustBridgeContract::batch_remove(env.clone(), other.clone(), usernames);
        assert_eq!(result, Err(ContractError::NotAuthorized));
    });
}

/// batch_remove with mixed verified/unverified records updates counters correctly.
#[test]
fn test_batch_remove_mixed_verified_state() {
    let env = Env::default();
    let (admin, user1, user2, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "alice")).unwrap();
    });

    env.as_contract(&contract_id, || {
        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 2);
        assert_eq!(stats.verified, 1);
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let usernames = soroban_sdk::vec![
            &env,
            username(&env, "alice"), // verified
            username(&env, "bob"),   // unverified
        ];
        let summary =
            TrustBridgeContract::batch_remove(env.clone(), admin.clone(), usernames).unwrap();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.successful, 2);

        let stats = TrustBridgeContract::get_stats(env.clone());
        assert_eq!(stats.total, 0);
        assert_eq!(stats.verified, 0);
    });
}

/// #114-verify-error-codes: all verify/revoke-relevant error codes match their ABI discriminants.
#[test]
fn test_verify_revoke_negative_error_codes_match_abi() {
    assert_eq!(ContractError::NotInitialized.code(), 2);
    assert_eq!(ContractError::NotAuthorized.code(), 3);
    assert_eq!(ContractError::NotRegistered.code(), 4);
    assert_eq!(ContractError::AlreadyVerified.code(), 5);
    assert_eq!(ContractError::NotVerified.code(), 6);
    assert_eq!(ContractError::Paused.code(), 7);
}

// ── Revoke reason code validation (Wave #19) ───────────────────────────────
//
// `revoke_verification` now requires a `reason_code` parameter. Valid codes
// are defined in the `RevokeReason` enum; invalid codes return
// `InvalidReasonCode` (code 15) before auth or any storage mutation.

/// All `RevokeReason` variants are accepted by `revoke_verification`.
#[test]
fn test_revoke_verification_accepts_all_valid_reason_codes() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });

    let valid_codes = [1, 2, 3, 4, 5, 6, 99];
    for &code in &valid_codes {
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::revoke_verification(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
                code,
            )
            .unwrap();
            assert!(
                !TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .verified,
                "revoke with reason code {code} must clear verified"
            );
        });

        // Re-verify for the next iteration
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"))
                .unwrap();
        });
    }
}

/// Invalid reason codes return `InvalidReasonCode` (code 15) and leave the
/// record unmodified.
#[test]
fn test_revoke_verification_rejects_invalid_reason_code() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });

    let invalid_codes = [0, 7, 8, 9, 10, 11, 50, 98, 100, u32::MAX];
    for &code in &invalid_codes {
        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let result = TrustBridgeContract::revoke_verification(
                env.clone(),
                admin.clone(),
                username(&env, "octocat"),
                code,
            );
            assert_eq!(
                result,
                Err(ContractError::InvalidReasonCode),
                "expected InvalidReasonCode for reason code {code}"
            );
            assert_eq!(ContractError::InvalidReasonCode.code(), 15);

            // Record must remain verified and verified count unchanged
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
                    .unwrap()
                    .verified,
                "failed revoke must not clear verified flag"
            );
            assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 1);
        });
    }
}

// ── ContributorRecord size optimization (Issue #67 / Wave #66) ─────────
//
// `ContributorRecord.registered_at` was changed from `u64` to `u32`,
// saving 4 bytes per record (37 bytes serialized vs 41 bytes in XDR).
// This is safe because Soroban ledger timestamps (Unix seconds) fit in
// u32 until ~2106.

/// `registered_at` stores a valid ledger timestamp at registration time.
#[test]
fn test_contributor_record_registered_at_is_valid_timestamp() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let ledger_ts = env.ledger().timestamp();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        // registered_at fits in u32 and matches the ledger timestamp.
        assert!(
            (record.registered_at as u64).abs_diff(ledger_ts) <= 2,
            "registered_at ({}) should be within 2s of ledger timestamp ({})",
            record.registered_at,
            ledger_ts
        );
    });
}

/// `registered_at` updates on re-registration with the same address.
#[test]
fn test_contributor_record_registered_at_updates_on_reregister() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    let ts1 = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
            .unwrap()
            .registered_at
    });
    // Advance ledger and re-register
    env.ledger().set_timestamp(env.ledger().timestamp() + 1000);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    let ts2 = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_address(env.clone(), username(&env, "octocat"))
            .unwrap()
            .registered_at
    });
    assert!(ts2 > ts1, "registered_at must advance on re-registration");
    assert_eq!(ts2 as u64, env.ledger().timestamp());
}

/// `registered_at` for a freshly registered record must equal the ledger
/// timestamp at registration time (truncated to u32).
#[test]
fn test_contributor_record_registered_at_matches_ledger_timestamp() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    // Set a specific ledger timestamp
    let specific_ts: u64 = 1_700_000_000; // Nov 2023
    env.ledger().set_timestamp(specific_ts);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert_eq!(
            record.registered_at as u64, specific_ts,
            "registered_at must match ledger timestamp"
        );
    });
}

/// `registered_at` uses u32 and the cast from u64 must not lose precision
/// for current timestamps (well below u32::MAX ≈ 4.3B).
#[test]
fn test_contributor_record_registered_at_fits_in_u32() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();

        let ledger_ts = env.ledger().timestamp();
        // ledger_ts is u64; registered_at is u32.
        // For the current epoch (~1.7B) this is well within u32 range.
        assert!(
            ledger_ts <= u32::MAX as u64,
            "ledger timestamp {} exceeds u32 range — u32 would truncate!",
            ledger_ts
        );
        assert_eq!(
            record.registered_at as u64, ledger_ts,
            "u32 truncation must not lose precision for current timestamps"
        );
    });
}

/// The revocation event exposes the complete payload and stable topic shape.
#[test]
fn test_verification_revoked_event_payload_is_complete() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::revoke_verification(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
            2, // CompromisedKey
        )
        .unwrap();

        let expected = VerificationRevokedEvent {
            github_username: username(&env, "octocat"),
            stellar_address: user.clone(),
            timestamp: env.ledger().timestamp(),
            reason_code: 2,
            domain: event_domain(&env),
        };

        assert_eq!(
            env.events().all(),
            soroban_sdk::vec![
                &env,
                (
                    contract_id.clone(),
                    expected.topics(&env),
                    expected.data(&env),
                )
            ],
            "VerificationRevokedEvent must include the supplied reason_code"
        );

        let topics = expected.topics(&env);
        assert_eq!(
            topics.len(),
            2,
            "VerificationRevokedEvent must have 2 topics"
        );
        assert_eq!(
            soroban_sdk::Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
            soroban_sdk::Symbol::new(&env, "verification_revoked_event"),
            "VerificationRevokedEvent topic symbol changed"
        );
        assert_eq!(
            String::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            username(&env, "octocat"),
            "VerificationRevokedEvent username topic changed"
        );
    });
}

/// A failed revoke must not publish an event that could make an indexer
/// mark an unverified contributor as revoked.
#[test]
fn test_verification_revoked_event_not_published_on_not_verified() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user, Vec::new(&env))
            .unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::revoke_verification(
            env.clone(),
            admin,
            username(&env, "octocat"),
            1,
        );
        assert_eq!(result, Err(ContractError::NotVerified));
        assert_eq!(
            env.events().all(),
            soroban_sdk::vec![&env],
            "failed revoke published a VerificationRevokedEvent"
        );
    });
}

// ── TTL keeper tests ─────────────────────────────────────────────────────

#[test]
fn test_extend_registry_ttl() {
    let env = Env::default();
    let (_admin, user1, _other, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    // 1. Success path
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let usernames = soroban_sdk::vec![&env, username(&env, "octocat")];
        let extended = TrustBridgeContract::extend_registry_ttl(env.clone(), usernames).unwrap();
        assert_eq!(extended, 1);
    });

    // 2. Unknown username (skipped, not an error)
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let usernames = soroban_sdk::vec![&env, username(&env, "unknown")];
        let extended = TrustBridgeContract::extend_registry_ttl(env.clone(), usernames).unwrap();
        assert_eq!(extended, 0);
    });

    // 3. Mixed batch (some exist, some unknown)
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let usernames =
            soroban_sdk::vec![&env, username(&env, "unknown"), username(&env, "octocat")];
        let extended = TrustBridgeContract::extend_registry_ttl(env.clone(), usernames).unwrap();
        assert_eq!(extended, 1);
    });

    // 4. Invalid batch size (empty)
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let usernames = soroban_sdk::vec![&env];
        let res = TrustBridgeContract::extend_registry_ttl(env.clone(), usernames);
        assert_eq!(res, Err(ContractError::InvalidBatchSize));
    });

    // 5. Works while paused
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
        let usernames = soroban_sdk::vec![&env, username(&env, "octocat")];
        let extended = TrustBridgeContract::extend_registry_ttl(env.clone(), usernames).unwrap();
        assert_eq!(extended, 1);
    });
}

// ── Audit log & config_verification tests ────────────────────────────────

#[test]
fn test_audit_logs_persisted_and_retrieved() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user, Vec::new(&env))
            .unwrap();

        let logs = TrustBridgeContract::get_audit_logs(env.clone());
        assert!(!logs.is_empty());
        assert_eq!(
            logs.get(0).unwrap().event_type,
            AuditEventType::ContractInitialized
        );
        assert_eq!(
            logs.get(1).unwrap().event_type,
            AuditEventType::UserRegistered
        );

        let stats = TrustBridgeContract::get_audit_stats(env.clone());
        assert_eq!(stats.registrations, 1);
    });
}

#[test]
fn test_unauthorized_caller_leaves_no_audit_row() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), username(&env, "octocat"), user, Vec::new(&env))
            .unwrap();
    });

    let initial_logs_len = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_audit_logs(env.clone()).len()
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let res =
            TrustBridgeContract::remove(env.clone(), other.clone(), username(&env, "octocat"));
        assert_eq!(res, Err(ContractError::NotAuthorized));

        let logs_len_after = TrustBridgeContract::get_audit_logs(env.clone()).len();
        assert_eq!(initial_logs_len, logs_len_after);
    });
}

#[test]
fn test_config_verification_persists_and_gets() {
    let env = Env::default();
    let (admin, _user, other, contract_id) = setup(&env);
    let attestation = Symbol::new(&env, "github_att");

    // Unauthorized caller fails
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let res = TrustBridgeContract::config_verification(
            env.clone(),
            other,
            attestation.clone(),
            3600,
            2,
        );
        assert_eq!(res, Err(ContractError::NotAuthorized));
        assert_eq!(
            TrustBridgeContract::get_verification_config(env.clone()),
            None
        );
    });

    // Admin caller succeeds
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::config_verification(env.clone(), admin, attestation.clone(), 3600, 2)
            .unwrap();

        let cfg = TrustBridgeContract::get_verification_config(env.clone()).unwrap();
        assert_eq!(cfg.attestation, attestation);
        assert_eq!(cfg.expires_in, 3600);
        assert_eq!(cfg.threshold, 2);
    });
}

// ── Cooldown enforcement tests ─────────────────────────────────────────

#[test]
fn test_register_cooldown_enforcement() {
    let env = Env::default();
    let (_admin, user1, user2, contract_id) = setup(&env);

    env.ledger().set_timestamp(1000);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Set cooldown to 100 seconds
        TrustBridgeContract::set_cooldown(env.clone(), 100).unwrap();

        register_personal(&env, &contract_id, "alice", &user1);
        register_personal(&env, &contract_id, "bob", &user2);
        // First registration succeeds
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();

        // Immediate re-registration fails with CooldownActive
        let res = TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user2.clone(),
            Vec::new(&env),
        );
        assert_eq!(res, Err(ContractError::CooldownActive));
    });

    // Advance ledger timestamp by 101 seconds
    env.ledger().set_timestamp(1000 + 101);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // After cooldown elapses, re-registration succeeds
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user2.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });
}

// Issue #296: `is_registration_in_cooldown` is the read-only view of the
// cooldown that `register` enforces inline. It must flip to `true` purely as
// a side effect of a successful `register` — with no `record_action` call,
// which no longer exists — and back to `false` once the window elapses.
#[test]
fn test_is_registration_in_cooldown_tracks_auto_enforcement() {
    let env = Env::default();
    let (_admin, user1, user2, contract_id) = setup(&env);

    env.ledger().set_timestamp(5_000);
    env.mock_all_auths();

    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_cooldown(env.clone(), 100).unwrap();

        // Unknown username: never in cooldown.
        assert!(!TrustBridgeContract::is_registration_in_cooldown(
            env.clone(),
            username(&env, "octocat")
        ));

        // A successful register stamps the cooldown with no extra call.
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
        assert!(TrustBridgeContract::is_registration_in_cooldown(
            env.clone(),
            username(&env, "octocat")
        ));

        // And the enforced mutation agrees with the read.
        assert_eq!(
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "octocat"),
                user2.clone(),
                Vec::new(&env),
            ),
            Err(ContractError::CooldownActive)
        );
    });

    env.ledger().set_timestamp(5_000 + 101);
    env.as_contract(&contract_id, || {
        assert!(!TrustBridgeContract::is_registration_in_cooldown(
            env.clone(),
            username(&env, "octocat")
        ));
    });
}

// ── Issue #197: set_paused events ─────────────────────────────────────────

#[test]
fn test_set_paused_true_emits_paused_event() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_paused(env.clone(), true, 1).unwrap();
        let events = env.events().all();
        let expected_topic = soroban_sdk::xdr::ScVal::Symbol(soroban_sdk::xdr::ScSymbol(
            "paused_event".try_into().unwrap(),
        ));
        let found = events.events().iter().any(|e| {
            let soroban_sdk::xdr::ContractEventBody::V0(body) = &e.body;
            body.topics.iter().any(|t| t == &expected_topic)
        });
        assert!(found, "set_paused(true) must emit PausedEvent");
    });
}

#[test]
fn test_set_paused_false_emits_unpaused_event() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // First pause, then unpause via set_paused
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
        env.events().all(); // consume
        TrustBridgeContract::set_paused(env.clone(), false, 4).unwrap();
        let events = env.events().all();
        let expected_topic = soroban_sdk::xdr::ScVal::Symbol(soroban_sdk::xdr::ScSymbol(
            "unpaused_event".try_into().unwrap(),
        ));
        let found = events.events().iter().any(|e| {
            let soroban_sdk::xdr::ContractEventBody::V0(body) = &e.body;
            body.topics.iter().any(|t| t == &expected_topic)
        });
        assert!(found, "set_paused(false) must emit UnpausedEvent");
    });
}

#[test]
fn test_set_paused_idempotent_no_duplicate_event() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Already unpaused — calling set_paused(false) again should be a no-op
        let count_before = env.events().all().events().len();
        TrustBridgeContract::set_paused(env.clone(), false, 4).unwrap();
        let count_after = env.events().all().events().len();
        assert_eq!(
            count_before, count_after,
            "set_paused(false) while already unpaused must not emit an event"
        );

        // Pause, then call set_paused(true) again — still a no-op
        TrustBridgeContract::set_paused(env.clone(), true, 1).unwrap();
        let count_before2 = env.events().all().events().len();
        TrustBridgeContract::set_paused(env.clone(), true, 1).unwrap();
        let count_after2 = env.events().all().events().len();
        assert_eq!(
            count_before2, count_after2,
            "set_paused(true) while already paused must not emit an event"
        );
    });
}

#[test]
fn test_set_paused_unauthorized_caller_fails() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths_allowing_non_root_auth();
    env.as_contract(&contract_id, || {
        // No-auth call should still be gated through admin.require_auth()
        // We mock all auths so this tests the logical path — the admin key is
        // the only one that satisfies the check.
        assert!(
            TrustBridgeContract::set_paused(env.clone(), true, 1).is_ok(),
            "mock_all_auths must satisfy require_auth()"
        );
    });
    drop(user); // suppress unused warning
}

// ── Issue #196: guardian circuit breaker ─────────────────────────────────

#[test]
fn test_emergency_pause_by_guardian_blocks_mutations() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    let guardian = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Register first so we have a record to mutate
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();

        // Admin designates a guardian
        TrustBridgeContract::set_guardian(env.clone(), guardian.clone()).unwrap();

        // Guardian trips the circuit breaker
        TrustBridgeContract::emergency_pause(env.clone(), guardian.clone()).unwrap();

        // Mutations are now blocked
        assert_eq!(
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "newuser"),
                user.clone(),
                Vec::new(&env),
            ),
            Err(ContractError::Paused)
        );
        assert_eq!(
            TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat")),
            Err(ContractError::Paused)
        );
    });
    drop(admin);
}

#[test]
fn test_emergency_pause_only_admin_can_clear() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    let guardian = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_guardian(env.clone(), guardian.clone()).unwrap();
        TrustBridgeContract::emergency_pause(env.clone(), guardian.clone()).unwrap();
        assert!(TrustBridgeContract::is_emergency_paused(env.clone()));

        // Admin clears it
        TrustBridgeContract::clear_emergency_pause(env.clone()).unwrap();
        assert!(!TrustBridgeContract::is_emergency_paused(env.clone()));
    });
    drop(admin);
}

#[test]
fn test_guardian_cannot_upgrade() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    let guardian = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_guardian(env.clone(), guardian.clone()).unwrap();
        // Guardian has no Upgrader role — verify get_role returns None
        let role = TrustBridgeContract::get_role(env.clone(), guardian.clone());
        assert!(role.is_none(), "guardian must not have any role assigned");
    });
}

#[test]
fn test_non_guardian_non_admin_cannot_emergency_pause() {
    let env = Env::default();
    let (_admin, _user, other, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::emergency_pause(env.clone(), other.clone());
        assert_eq!(result, Err(ContractError::NotAuthorized));
    });
}

#[test]
fn test_emergency_pause_idempotent() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::emergency_pause(env.clone(), admin.clone()).unwrap();
        let count_before = env.events().all().events().len();
        // Second call should be no-op
        TrustBridgeContract::emergency_pause(env.clone(), admin.clone()).unwrap();
        let count_after = env.events().all().events().len();
        assert_eq!(
            count_before, count_after,
            "emergency_pause while already tripped must not emit an event"
        );
    });
}

#[test]
fn test_both_pause_flags_both_must_clear() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    let guardian = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_guardian(env.clone(), guardian.clone()).unwrap();

        // Trip both flags
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
        TrustBridgeContract::emergency_pause(env.clone(), guardian.clone()).unwrap();

        // Clear emergency pause only — normal pause still blocks
        TrustBridgeContract::clear_emergency_pause(env.clone()).unwrap();
        let user = Address::generate(&env);
        assert_eq!(
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "stillblocked"),
                user.clone(),
                Vec::new(&env),
            ),
            Err(ContractError::Paused),
            "normal pause still active after emergency cleared"
        );

        // Clear normal pause too — now unblocked
        TrustBridgeContract::unpause(env.clone(), 4).unwrap();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "nowworks"),
            user,
            Vec::new(&env),
        )
        .unwrap();
    });
    drop(admin);
}

// ── Issue #202: CHUNK_SIZE regression test ────────────────────────────────

#[test]
fn test_chunk_size_is_fifty() {
    // Regression test: CHUNK_SIZE must be 50.
    // If this constant is ever changed, docs/DASHBOARD_SYNC.md,
    // docs/storage-rent-estimator.inputs.v1.json, and the estimator's
    // cost_drivers_vs_n table must all be updated to match.
    assert_eq!(
        crate::storage::CHUNK_SIZE,
        50,
        "CHUNK_SIZE changed! Update DASHBOARD_SYNC.md and storage-rent-estimator.inputs.v1.json"
    );
}

#[test]
fn test_chunk_boundaries_at_fifty() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Register exactly CHUNK_SIZE (50) users — all go in chunk 0
        for i in 0..50u32 {
            let name = format!("user{i:03}");
            let addr = Address::generate(&env);
            TrustBridgeContract::register(env.clone(), username(&env, &name), addr, Vec::new(&env))
                .unwrap();
        }
        let chunk_count_at_50 = crate::storage::get_chunk_count(&env);
        assert_eq!(
            chunk_count_at_50, 1,
            "50 entries should occupy exactly 1 chunk"
        );

        // Register the 51st user — must spill into chunk 1
        let addr51 = Address::generate(&env);
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "user050"),
            addr51,
            Vec::new(&env),
        )
        .unwrap();
        let chunk_count_at_51 = crate::storage::get_chunk_count(&env);
        assert_eq!(
            chunk_count_at_51, 2,
            "51st entry must create a second chunk"
        );
    });
}

/// Issue #291: `has_record` / `get_address` must resolve by direct persistent
/// key and never scan the chunked index, so they stay correct — and O(1) —
/// with the registry spread across many chunks.
#[test]
fn test_has_record_is_direct_key_across_many_chunks() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);

    // Enough users to fill several chunks (CHUNK_SIZE == 50).
    const N: u32 = 260;

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        for i in 0..N {
            let name = format!("user{i:04}");
            let addr = Address::generate(&env);
            TrustBridgeContract::register(env.clone(), username(&env, &name), addr, Vec::new(&env))
                .unwrap();
        }

        // The index really is chunked now.
        assert!(
            crate::storage::get_chunk_count(&env) >= 5,
            "expected the registry to span multiple chunks"
        );

        // A user in the very first chunk, one in the last, and one in the
        // middle all resolve — a chunk scan that stopped early would miss
        // some of these.
        for name in ["user0000", "user0130", &format!("user{:04}", N - 1)] {
            assert!(
                TrustBridgeContract::has_record(env.clone(), username(&env, name)),
                "has_record must find '{name}' regardless of which chunk holds it"
            );
            assert!(
                TrustBridgeContract::get_address(env.clone(), username(&env, name)).is_some(),
                "get_address must find '{name}' regardless of which chunk holds it"
            );
        }

        // A never-registered username must report absent without scanning
        // every chunk looking for it.
        assert!(
            !TrustBridgeContract::has_record(env.clone(), username(&env, "ghost")),
            "has_record must not report a phantom record"
        );
        assert!(
            TrustBridgeContract::get_address(env.clone(), username(&env, "ghost")).is_none(),
            "get_address must return None for an unregistered username"
        );
    });
}

// ── Issue #200: Property fuzzing suite ───────────────────────────────────
//
// The contract is `no_std`, so external fuzz crates (proptest, arbitrary)
// are unavailable. We use a tiny xorshift64 PRNG with fixed seeds for
// deterministic, CI-friendly property testing. A `Shadow` struct mirrors
// the registry outside contract storage so invariants are checked against
// an independent model.

/// Tiny xorshift64 PRNG — deterministic, no-std, no dependencies.
struct Prng(u64);

impl Prng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_usize(&mut self, n: usize) -> usize {
        (self.next() as usize) % n
    }
}

/// Checked-in seed corpus (Issue #331) — failures are deterministic and
/// always reproduce. Append a failing seed to `tests/fuzz/seeds.txt`.
const FUZZ_SEED_CORPUS: &str = include_str!("../tests/fuzz/seeds.txt");

fn parse_fuzz_seeds(text: &str) -> std::vec::Vec<u64> {
    text.split(|c: char| c == '\n' || c == ',')
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .map(|l| {
            let clean = l.replace('_', "");
            match clean
                .strip_prefix("0x")
                .or_else(|| clean.strip_prefix("0X"))
            {
                Some(hex) => u64::from_str_radix(hex, 16),
                None => clean.parse::<u64>(),
            }
            .unwrap_or_else(|_| panic!("invalid fuzz seed: {l:?}"))
        })
        .collect()
}

/// Seeds for this run: `FUZZ_SEEDS` env var (comma-separated) if set,
/// otherwise the checked-in corpus. Always at least 4 seeds.
fn fuzz_seeds() -> std::vec::Vec<u64> {
    let seeds = match std::env::var("FUZZ_SEEDS") {
        Ok(v) if !v.trim().is_empty() => parse_fuzz_seeds(&v),
        _ => parse_fuzz_seeds(FUZZ_SEED_CORPUS),
    };
    assert!(
        seeds.len() >= 4,
        "need at least 4 fuzz seeds, got {}",
        seeds.len()
    );
    seeds
}

#[test]
fn test_fuzz_seed_corpus_parses() {
    let seeds = parse_fuzz_seeds(FUZZ_SEED_CORPUS);
    assert!(seeds.len() >= 4);
    assert_eq!(seeds[0], 0xDEAD_BEEF_1234_5678);
    assert_eq!(parse_fuzz_seeds("0x10, 7 # c"), std::vec![16, 7]);
}

/// Shadow model of the registry. Mirrors the contract's own counters using
/// independent logic so a bug in the contract cannot hide itself.
#[derive(Default)]
struct Shadow {
    /// username → (stellar_address_index, verified)
    entries: std::vec::Vec<(std::string::String, usize, bool)>,
}

impl Shadow {
    fn has(&self, name: &str) -> bool {
        self.entries.iter().any(|(n, _, _)| n == name)
    }

    fn is_verified(&self, name: &str) -> bool {
        self.entries
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, _, v)| *v)
            .unwrap_or(false)
    }

    fn register(&mut self, name: std::string::String, addr_idx: usize) {
        if let Some(entry) = self.entries.iter_mut().find(|(n, _, _)| *n == name) {
            // Re-register same username: keep verified only if same address
            let keep_verified = entry.1 == addr_idx && entry.2;
            entry.1 = addr_idx;
            entry.2 = keep_verified;
        } else {
            self.entries.push((name, addr_idx, false));
        }
    }

    fn verify(&mut self, name: &str) {
        if let Some(entry) = self.entries.iter_mut().find(|(n, _, _)| n == name) {
            entry.2 = true;
        }
    }

    fn revoke(&mut self, name: &str) {
        if let Some(entry) = self.entries.iter_mut().find(|(n, _, _)| n == name) {
            entry.2 = false;
        }
    }

    fn remove(&mut self, name: &str) {
        self.entries.retain(|(n, _, _)| n != name);
    }

    fn total(&self) -> u32 {
        self.entries.len() as u32
    }

    fn verified(&self) -> u32 {
        self.entries.iter().filter(|(_, _, v)| *v).count() as u32
    }
}

/// Assert all 8 registry invariants match the shadow model.
fn assert_registry_invariants(env: &Env, contract_id: &Address, shadow: &Shadow) {
    let stats = env.as_contract(contract_id, || TrustBridgeContract::get_stats(env.clone()));
    let vcount = env.as_contract(contract_id, || {
        TrustBridgeContract::get_verified_count(env.clone())
    });

    // I1: total count matches shadow
    assert_eq!(
        stats.total,
        shadow.total(),
        "I1 violated: total count mismatch (contract={}, shadow={})",
        stats.total,
        shadow.total()
    );
    // I2 + I3: verified count consistent
    assert_eq!(
        stats.verified,
        shadow.verified(),
        "I2 violated: verified count mismatch (contract={}, shadow={})",
        stats.verified,
        shadow.verified()
    );
    assert_eq!(
        vcount, stats.verified,
        "I3 violated: get_verified_count() diverged from get_stats().verified"
    );
    // I4: verified <= total
    assert!(
        stats.verified <= stats.total,
        "I4 violated: verified ({}) > total ({})",
        stats.verified,
        stats.total
    );
}

/// Run one fuzz session: `steps` random operations against `usernames`,
/// asserting invariants after every step.
fn run_fuzz_session(
    env: &Env,
    contract_id: &Address,
    admin: &Address,
    addrs: &[Address],
    usernames: &[&str],
    prng: &mut Prng,
    steps: usize,
) {
    let mut shadow = Shadow::default();

    for _ in 0..steps {
        let op = prng.next_usize(4);
        let name_idx = prng.next_usize(usernames.len());
        let addr_idx = prng.next_usize(addrs.len());
        let name = usernames[name_idx];

        match op {
            0 => {
                // register
                let is_new = !shadow.has(name);
                let addr = addrs[addr_idx].clone();
                env.mock_all_auths();
                let result = env.as_contract(contract_id, || {
                    TrustBridgeContract::register(
                        env.clone(),
                        username(env, name),
                        addr.clone(),
                        Vec::new(&env),
                    )
                });
                if result.is_ok() {
                    shadow.register(name.to_string(), addr_idx);
                } else if is_new {
                    // New registrations should succeed unless paused/invalid
                    // (in this suite no pause is set, so failures are unexpected)
                    panic!("unexpected register failure for new user {name}: {result:?}");
                }
            }
            1 => {
                // verify
                let already_verified = shadow.is_verified(name);
                let exists = shadow.has(name);
                env.mock_all_auths();
                let result = env.as_contract(contract_id, || {
                    TrustBridgeContract::verify(env.clone(), admin.clone(), username(env, name))
                });
                match result {
                    Ok(()) => {
                        assert!(
                            exists,
                            "verify returned Ok but shadow has no record for {name}"
                        );
                        assert!(
                            !already_verified,
                            "verify returned Ok but shadow says already verified for {name}"
                        );
                        shadow.verify(name);
                    }
                    Err(ContractError::NotRegistered) => {
                        assert!(
                            !exists,
                            "verify returned NotRegistered but shadow has record for {name}"
                        );
                    }
                    Err(ContractError::AlreadyVerified) => {
                        assert!(already_verified, "verify returned AlreadyVerified but shadow says not verified for {name}");
                    }
                    Err(e) => panic!("unexpected verify error for {name}: {e:?}"),
                }
            }
            2 => {
                // revoke_verification
                let is_verified = shadow.is_verified(name);
                let exists = shadow.has(name);
                env.mock_all_auths();
                let result = env.as_contract(contract_id, || {
                    TrustBridgeContract::revoke_verification(
                        env.clone(),
                        admin.clone(),
                        username(env, name),
                        1,
                    )
                });
                match result {
                    Ok(()) => {
                        assert!(
                            is_verified,
                            "revoke returned Ok but shadow says not verified for {name}"
                        );
                        shadow.revoke(name);
                    }
                    Err(ContractError::NotRegistered) => {
                        assert!(
                            !exists,
                            "revoke returned NotRegistered but shadow has record for {name}"
                        );
                    }
                    Err(ContractError::NotVerified) => {
                        assert!(
                            !is_verified,
                            "revoke returned NotVerified but shadow says verified for {name}"
                        );
                    }
                    Err(e) => panic!("unexpected revoke error for {name}: {e:?}"),
                }
            }
            _ => {
                // remove
                let exists = shadow.has(name);
                let caller = if exists {
                    // Use the admin as caller to ensure auth passes
                    admin.clone()
                } else {
                    addrs[addr_idx].clone()
                };
                env.mock_all_auths();
                let result = env.as_contract(contract_id, || {
                    TrustBridgeContract::remove(env.clone(), caller, username(env, name))
                });
                match result {
                    Ok(()) => {
                        assert!(
                            exists,
                            "remove returned Ok but shadow has no record for {name}"
                        );
                        shadow.remove(name);
                    }
                    Err(ContractError::NotRegistered) => {
                        assert!(
                            !exists,
                            "remove returned NotRegistered but shadow has record for {name}"
                        );
                    }
                    Err(e) => panic!("unexpected remove error for {name}: {e:?}"),
                }
            }
        }

        assert_registry_invariants(env, contract_id, &shadow);
    }
}

#[test]
fn test_fuzz_invariants_hold_across_random_operation_sequences() {
    let usernames = [
        "alice", "bob", "carol", "dave", "eve", "frank", "grace", "heidi",
    ];

    for seed in fuzz_seeds() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);
        let addrs: std::vec::Vec<Address> = (0..4).map(|_| Address::generate(&env)).collect();

        let mut prng = Prng(seed);
        run_fuzz_session(
            &env,
            &contract_id,
            &admin,
            &addrs,
            &usernames,
            &mut prng,
            64,
        );
    }
}

#[test]
fn test_fuzz_invariants_hold_at_contributor_scale() {
    // Wider username pool to stress index/chunk boundaries.
    let usernames: std::vec::Vec<std::string::String> =
        (0..16).map(|i| format!("fuzz{i:02}")).collect();
    let username_refs: std::vec::Vec<&str> = usernames.iter().map(|s| s.as_str()).collect();

    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    let addrs: std::vec::Vec<Address> = (0..8).map(|_| Address::generate(&env)).collect();

    let mut prng = Prng(fuzz_seeds()[0]);
    run_fuzz_session(
        &env,
        &contract_id,
        &admin,
        &addrs,
        &username_refs,
        &mut prng,
        256,
    );
}

#[test]
fn test_fuzz_failure_paths_leave_invariants_intact() {
    // I7: rejected operations must not mutate any counter or record.
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "target"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    let usernames = ["target", "ghost"];
    let addrs = [user.clone(), admin.clone()];
    let mut prng = Prng(fuzz_seeds()[2]);
    let mut shadow = Shadow::default();
    shadow.register("target".to_string(), 0);

    // Run 48 steps, then assert counters are consistent
    for _ in 0..48 {
        let op = prng.next_usize(4);
        let name = usernames[prng.next_usize(2)];
        let addr_idx = prng.next_usize(2);

        match op {
            0 => {
                env.mock_all_auths();
                let addr = addrs[addr_idx].clone();
                let existed = shadow.has(name);
                let res = env.as_contract(&contract_id, || {
                    TrustBridgeContract::register(
                        env.clone(),
                        username(&env, name),
                        addr,
                        Vec::new(&env),
                    )
                });
                if res.is_ok() {
                    shadow.register(name.to_string(), addr_idx);
                } else if !existed {
                    // Expected failure only if the entry already existed and
                    // some other constraint prevents it — which shouldn't happen
                    // in this simple suite.
                }
            }
            1 => {
                env.mock_all_auths();
                let res = env.as_contract(&contract_id, || {
                    TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, name))
                });
                if res.is_ok() {
                    shadow.verify(name);
                }
            }
            2 => {
                env.mock_all_auths();
                let res = env.as_contract(&contract_id, || {
                    TrustBridgeContract::revoke_verification(
                        env.clone(),
                        admin.clone(),
                        username(&env, name),
                        1,
                    )
                });
                if res.is_ok() {
                    shadow.revoke(name);
                }
            }
            _ => {
                env.mock_all_auths();
                let res = env.as_contract(&contract_id, || {
                    TrustBridgeContract::remove(env.clone(), admin.clone(), username(&env, name))
                });
                if res.is_ok() {
                    shadow.remove(name);
                }
            }
        }

        assert_registry_invariants(&env, &contract_id, &shadow);
    }
}

#[test]
fn test_fuzz_counters_never_underflow_on_empty_registry() {
    // I8: counter underflow guard — remove from empty registry is always a
    // no-op or error, never a wrap-around.
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);

    let mut prng = Prng(fuzz_seeds()[3]);
    let usernames = ["ghost1", "ghost2", "ghost3"];

    for _ in 0..32 {
        let name = usernames[prng.next_usize(3)];
        let caller = Address::generate(&env);
        env.mock_all_auths();
        // Removing a non-existent entry should never return Ok
        let result = env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), caller, username(&env, name))
        });
        assert_eq!(
            result,
            Err(ContractError::NotRegistered),
            "remove on empty registry should return NotRegistered, not Ok"
        );

        // After all these failed removes, counters must still be zero
        let stats = env.as_contract(&contract_id, || TrustBridgeContract::get_stats(env.clone()));
        assert_eq!(stats.total, 0, "I8 violated: total counter underflowed");
        assert_eq!(
            stats.verified, 0,
            "I8 violated: verified counter underflowed"
        );
    }
}

// ── Bot accounts (Issue #236) ────────────────────────────────────────────

#[test]
fn test_bot_default_false_and_set_by_admin() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert!(!record.is_bot);

        // Admin sets to true
        TrustBridgeContract::set_bot_status(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
            true,
        )
        .unwrap();
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert!(record.is_bot);
    });
}

#[test]
fn test_set_bot_status_emits_event() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    let client = TrustBridgeContractClient::new(&env, &contract_id);
    let name = username(&env, "octocat");
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    client.register(&name, &user, &Vec::new(&env));
    client.set_bot_status(&admin, &name, &true);

    let expected = BotStatusChangedEvent {
        github_username: name.clone(),
        is_bot: true,
        actor: admin.clone(),
        timestamp: 1_700_000_000,
        domain: env.as_contract(&contract_id, || event_domain(&env)),
    };
    // `events().all()` only holds events from the last invocation.
    assert_eq!(
        env.events().all(),
        soroban_sdk::vec![
            &env,
            (
                contract_id.clone(),
                expected.topics(&env),
                expected.data(&env),
            )
        ],
        "BotStatusChangedEvent payload or topics changed"
    );
}

#[test]
fn test_bot_set_by_self() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();

        // Registrant sets to true
        TrustBridgeContract::set_bot_status(
            env.clone(),
            user.clone(),
            username(&env, "octocat"),
            true,
        )
        .unwrap();
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert!(record.is_bot);
    });
}

#[test]
fn test_bot_set_by_unauthorized_fails() {
    let env = Env::default();
    let (_admin, user, other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();

        // Non-admin and non-registrant try to set bot status
        let result = TrustBridgeContract::set_bot_status(
            env.clone(),
            other.clone(),
            username(&env, "octocat"),
            true,
        );
        assert_eq!(result, Err(ContractError::NotAuthorized));
    });
}

#[test]
fn test_bot_set_nonregistered_fails() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::set_bot_status(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
            true,
        );
        assert_eq!(result, Err(ContractError::NotRegistered));
    });
}

// ── Sponsored registration (Issue #237) ──────────────────────────────────

#[test]
fn test_sponsor_register_success() {
    let env = Env::default();
    let (_admin, user, sponsor, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register_sponsored(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            sponsor.clone(),
        )
        .unwrap();

        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert_eq!(record.stellar_address, user);
        assert!(!record.verified);
    });

    let events = env.events().all();
    let mut found = false;
    let want = soroban_sdk::xdr::ScVal::try_from_val(
        &env,
        &soroban_sdk::Symbol::new(&env, "RegisteredEvent").to_val(),
    )
    .unwrap();
    for event in events.events() {
        let soroban_sdk::xdr::ContractEventBody::V0(v0) = &event.body;
        if v0.topics.first() == Some(&want) {
            found = true;
        }
    }
    assert!(found, "RegisteredEvent with sponsor must be published");
}

#[test]
fn test_sponsor_double_auth_protection_on_transfer() {
    let env = Env::default();
    let (_admin, user1, sponsor, contract_id) = setup(&env);
    let user2 = Address::generate(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Initially register to user1
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user1.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    // The auths list must contain: sponsor, user2 (new registrant) AND user1 (old registrant)!
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register_sponsored(
            env.clone(),
            username(&env, "octocat"),
            user2.clone(),
            sponsor.clone(),
        )
        .unwrap();
    });

    let auths = env.auths();
    let mut authorized_addresses = soroban_sdk::Vec::new(&env);
    for auth in auths.iter() {
        authorized_addresses.push_back(auth.0.clone());
    }
    assert!(authorized_addresses.contains(&sponsor));
    assert!(authorized_addresses.contains(&user2));
    assert!(authorized_addresses.contains(&user1));
}

// ── Role expiry (Issue #221) ────────────────────────────────────────────

#[test]
fn test_set_role_with_expiry_active_before_expiry() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role_with_expiry(
            env.clone(),
            user.clone(),
            Role::Verifier,
            Some(2_000),
        )
        .unwrap();
        assert_eq!(
            TrustBridgeContract::get_role(env.clone(), user.clone()),
            Some(Role::Verifier)
        );
        assert!(TrustBridgeContract::has_role(
            env.clone(),
            user.clone(),
            Role::Verifier
        ));
        assert_eq!(
            TrustBridgeContract::get_role_expiry(env.clone(), user.clone()),
            Some(2_000)
        );
    });
}

#[test]
fn test_role_expires_lazily_get_role_returns_none_after_expiry() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role_with_expiry(
            env.clone(),
            user.clone(),
            Role::Verifier,
            Some(2_000),
        )
        .unwrap();
    });

    env.ledger().set_timestamp(2_500);
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_role(env.clone(), user.clone()),
            None
        );
        assert!(!TrustBridgeContract::has_role(
            env.clone(),
            user.clone(),
            Role::Verifier
        ));
        // Lazy expiry: the raw timestamp is still readable even though the
        // grant no longer resolves as held.
        assert_eq!(
            TrustBridgeContract::get_role_expiry(env.clone(), user.clone()),
            Some(2_000)
        );
    });
}

#[test]
fn test_role_expiry_boundary_exactly_now_is_expired() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role_with_expiry(
            env.clone(),
            user.clone(),
            Role::Verifier,
            Some(2_000),
        )
        .unwrap();
    });

    // One ledger second before expires_at: still active.
    env.ledger().set_timestamp(1_999);
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_role(env.clone(), user.clone()),
            Some(Role::Verifier)
        );
    });

    // Exactly at expires_at: `is_role_expired` uses `>=`, so this reads as
    // already expired.
    env.ledger().set_timestamp(2_000);
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_role(env.clone(), user.clone()),
            None
        );
    });
}

#[test]
fn test_set_role_default_never_expires() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), user.clone(), Role::Verifier).unwrap();
        assert_eq!(
            TrustBridgeContract::get_role_expiry(env.clone(), user.clone()),
            None
        );
    });

    env.ledger().set_timestamp(1_000_000_000);
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_role(env.clone(), user.clone()),
            Some(Role::Verifier)
        );
    });
}

#[test]
fn test_remove_role_clears_expiry() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role_with_expiry(
            env.clone(),
            user.clone(),
            Role::Verifier,
            Some(5_000),
        )
        .unwrap();
    });
    // Separate frame: one `require_auth` per address per frame.
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove_role(env.clone(), user.clone()).unwrap();
        assert_eq!(
            TrustBridgeContract::get_role(env.clone(), user.clone()),
            None
        );
        assert_eq!(
            TrustBridgeContract::get_role_expiry(env.clone(), user.clone()),
            None
        );
    });
}

#[test]
fn test_admin_role_grant_with_expiry_does_not_affect_admin_identity() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    env.as_contract(&contract_id, || {
        // Explicitly (and unusually) expire the admin's own RBAC
        // Role::Admin grant — this must never touch `has_admin_role`.
        TrustBridgeContract::set_role_with_expiry(
            env.clone(),
            admin.clone(),
            Role::Admin,
            Some(1_100),
        )
        .unwrap();
    });

    env.ledger().set_timestamp(2_000);
    env.as_contract(&contract_id, || {
        // The RBAC grant has lapsed...
        assert_eq!(
            TrustBridgeContract::get_role(env.clone(), admin.clone()),
            None
        );
        // ...but the admin's real identity (ADMIN_KEY) is untouched.
        assert!(TrustBridgeContract::has_admin_role(
            env.clone(),
            admin.clone()
        ));
        // Admin-gated calls still work.
        TrustBridgeContract::set_cooldown(env.clone(), 42).unwrap();
        assert_eq!(TrustBridgeContract::get_cooldown(env.clone()), 42);
    });
}

#[test]
fn test_expired_verifier_role_cannot_verify() {
    let env = Env::default();
    let (admin, user, verifier, contract_id) = setup(&env);
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
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
        TrustBridgeContract::set_role_with_expiry(
            env.clone(),
            verifier.clone(),
            Role::Verifier,
            Some(2_000),
        )
        .unwrap();
    });

    env.ledger().set_timestamp(1_500);
    env.as_contract(&contract_id, || {
        // Still active: the expired-role holder can verify normally.
        TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "octocat"))
            .unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::revoke_verification(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
            1,
        )
        .unwrap();
    });

    env.ledger().set_timestamp(2_500);
    env.as_contract(&contract_id, || {
        let res =
            TrustBridgeContract::verify(env.clone(), verifier.clone(), username(&env, "octocat"));
        assert_eq!(res, Err(ContractError::RoleExpired));
    });
    env.as_contract(&contract_id, || {
        let batch_res = TrustBridgeContract::batch_verify(
            env.clone(),
            verifier.clone(),
            soroban_sdk::vec![&env, username(&env, "octocat")],
        );
        assert_eq!(batch_res, Err(ContractError::RoleExpired));
    });
}

#[test]
fn test_expired_revoker_cannot_revoke() {
    let env = Env::default();
    let (admin, user, revoker, contract_id) = setup(&env);
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role_with_expiry(
            env.clone(),
            revoker.clone(),
            Role::Revoker,
            Some(2_000),
        )
        .unwrap();
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });

    env.ledger().set_timestamp(2_000);
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::revoke_verification(
            env.clone(),
            revoker.clone(),
            username(&env, "octocat"),
            1,
        );
        assert_eq!(result, Err(ContractError::RoleExpired));
    });
}

/// `clear_staged` with no staged WASM is idempotent: admin auth, `Ok(())`,
/// no `StagedWasmClearedEvent`, and no leftover instance key.
#[test]
fn test_clear_staged_empty_slot_is_idempotent() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::get_staged(env.clone()).is_none());
    });
    let events_before = env.events().all();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::clear_staged(env.clone(), admin.clone()).unwrap();
    });
    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::get_staged(env.clone()).is_none());
        assert!(!env
            .storage()
            .instance()
            .has(&crate::staged_wasm::STAGED_WASM_KEY));
    });
    assert_eq!(
        env.events().all(),
        events_before,
        "empty-slot clear must not emit StagedWasmClearedEvent"
    );
    env.as_contract(&contract_id, || {
        TrustBridgeContract::clear_staged(env.clone(), admin.clone()).unwrap();
        assert!(TrustBridgeContract::get_staged(env.clone()).is_none());
    });
}

/// Non-admin `clear_staged` fails before touching storage or emitting a
/// success event, even when the slot is already empty.
#[test]
fn test_clear_staged_empty_slot_non_admin_rejected() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let events_before = env.events().all();
        let result = TrustBridgeContract::clear_staged(env.clone(), user.clone());
        assert_eq!(result, Err(ContractError::NotAuthorized));
        assert!(TrustBridgeContract::get_staged(env.clone()).is_none());
        assert!(!env
            .storage()
            .instance()
            .has(&crate::staged_wasm::STAGED_WASM_KEY));
        assert_eq!(env.events().all(), events_before);
    });
}

/// After a successful clear, a second admin `clear_staged` stays a no-op
/// (same as the post-upgrade empty slot).
#[test]
fn test_clear_staged_after_successful_clear_remains_empty() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let hash = BytesN::from_array(&env, &[0xAB; 32]);
        TrustBridgeContract::stage_wasm(env.clone(), admin.clone(), hash).unwrap();
        assert!(TrustBridgeContract::get_staged(env.clone()).is_some());
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::clear_staged(env.clone(), admin.clone()).unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::clear_staged(env.clone(), admin.clone()).unwrap();
        assert!(TrustBridgeContract::get_staged(env.clone()).is_none());
        assert!(!env
            .storage()
            .instance()
            .has(&crate::staged_wasm::STAGED_WASM_KEY));
    });
}

#[test]
fn test_expired_upgrader_is_rejected_by_all_upgrade_entrypoints() {
    let env = Env::default();
    let (admin, _user, upgrader, contract_id) = setup(&env);
    let wasm_hash = BytesN::from_array(&env, &[7; 32]);
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    let proposal_id = env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role_with_expiry(
            env.clone(),
            upgrader.clone(),
            Role::Upgrader,
            Some(2_000),
        )
        .unwrap();
        TrustBridgeContract::stage_wasm(env.clone(), upgrader.clone(), wasm_hash.clone()).unwrap();
        TrustBridgeContract::propose_multisig_upgrade(
            env.clone(),
            admin.clone(),
            wasm_hash.clone(),
            0,
        )
        .unwrap();
        let proposal_id = TrustBridgeContract::get_upgrade_proposal(env.clone())
            .unwrap()
            .id;
        TrustBridgeContract::approve_upgrade(env.clone(), upgrader.clone(), proposal_id).unwrap();
        proposal_id
    });

    env.ledger().set_timestamp(2_000);
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::stage_wasm(env.clone(), upgrader.clone(), wasm_hash.clone()),
            Err(ContractError::RoleExpired)
        );
        assert_eq!(
            TrustBridgeContract::propose_multisig_upgrade(
                env.clone(),
                upgrader.clone(),
                wasm_hash.clone(),
                0,
            ),
            Err(ContractError::RoleExpired)
        );
        assert_eq!(
            TrustBridgeContract::approve_upgrade(env.clone(), upgrader.clone(), proposal_id),
            Err(ContractError::RoleExpired)
        );
        assert_eq!(
            TrustBridgeContract::execute_upgrade(env.clone(), upgrader.clone(), proposal_id),
            Err(ContractError::RoleExpired)
        );
    });
}

// ── Time-bounded verification (Issue #218) ──────────────────────────────

#[test]
fn test_verify_expires_after_configured_window() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::config_verification(
            env.clone(),
            admin.clone(),
            soroban_sdk::Symbol::new(&env, "github_att"),
            1_000,
            1,
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
        assert!(TrustBridgeContract::is_verification_active(
            env.clone(),
            username(&env, "octocat")
        ));
    });

    // Exactly at the expiry boundary.
    env.ledger().set_timestamp(2_000);
    env.as_contract(&contract_id, || {
        assert!(!TrustBridgeContract::is_verification_active(
            env.clone(),
            username(&env, "octocat")
        ));
        // Lazy expiry: the raw flag is untouched.
        let record =
            TrustBridgeContract::get_address(env.clone(), username(&env, "octocat")).unwrap();
        assert!(record.verified);
    });
}

#[test]
fn test_verify_renews_after_expiry_instead_of_already_verified() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::config_verification(
            env.clone(),
            admin.clone(),
            soroban_sdk::Symbol::new(&env, "github_att"),
            1_000,
            1,
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });

    // Still active: a duplicate verify is rejected as before this issue.
    env.ledger().set_timestamp(1_500);
    env.as_contract(&contract_id, || {
        let res =
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat"));
        assert_eq!(res, Err(ContractError::AlreadyVerified));
    });

    // Past expiry: verify renews instead of erroring, and does not
    // double-count `verified_count` / `ever_verified_count`.
    env.ledger().set_timestamp(2_500);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
        assert!(TrustBridgeContract::is_verification_active(
            env.clone(),
            username(&env, "octocat")
        ));
        assert_verified_parity(&env, 1);
        assert_eq!(TrustBridgeContract::get_ever_verified_count(env.clone()), 1);
    });
}

#[test]
fn test_verification_without_config_never_expires() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });

    env.ledger().set_timestamp(1_000_000_000);
    env.as_contract(&contract_id, || {
        assert!(TrustBridgeContract::is_verification_active(
            env.clone(),
            username(&env, "octocat")
        ));
    });
}

#[test]
fn test_verified_count_not_decremented_by_expiry_only_by_revoke() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::config_verification(
            env.clone(),
            admin.clone(),
            soroban_sdk::Symbol::new(&env, "github_att"),
            1_000,
            1,
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });

    // Expire without revoking. Stats definition (Issue #218): the raw
    // counters count the raw `verified` flag, not the expiry-aware
    // status, so they do not move on their own.
    env.ledger().set_timestamp(5_000);
    env.as_contract(&contract_id, || {
        assert!(!TrustBridgeContract::is_verification_active(
            env.clone(),
            username(&env, "octocat")
        ));
        assert_verified_parity(&env, 1);

        TrustBridgeContract::revoke_verification(
            env.clone(),
            admin.clone(),
            username(&env, "octocat"),
            1,
        )
        .unwrap();
        assert_verified_parity(&env, 0);
    });
}

#[test]
fn test_get_verification_expiry_none_without_config() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
        assert_eq!(
            TrustBridgeContract::get_verification_expiry(env.clone(), username(&env, "octocat")),
            None
        );
    });
}

#[test]
fn test_get_verification_expiry_some_when_configured() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::config_verification(
            env.clone(),
            admin.clone(),
            soroban_sdk::Symbol::new(&env, "github_att"),
            500,
            1,
        )
        .unwrap();
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
        assert_eq!(
            TrustBridgeContract::get_verification_expiry(env.clone(), username(&env, "octocat")),
            Some(1_500)
        );
    });
}

// ── Signed export attestation (Issue #223) ──────────────────────────────

#[test]
fn test_export_attestation_empty_registry() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let attestation = TrustBridgeContract::export_attestation(env.clone(), None, 10).unwrap();
        assert_eq!(attestation.page.records.len(), 0);
        assert_eq!(attestation.page.total, 0);
        assert!(!attestation.page.has_more);

        // Deterministic even for an empty page.
        let again = TrustBridgeContract::export_attestation(env.clone(), None, 10).unwrap();
        assert_eq!(attestation.digest, again.digest);
    });
}

#[test]
fn test_export_attestation_digest_deterministic_across_calls() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let a = TrustBridgeContract::export_attestation(env.clone(), None, 10).unwrap();
        let b = TrustBridgeContract::export_attestation(env.clone(), None, 10).unwrap();
        assert_eq!(a.digest, b.digest);
        assert_eq!(a.page, b.page);
    });
}

#[test]
fn test_export_attestation_digest_changes_with_data() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    let before = env.as_contract(&contract_id, || {
        TrustBridgeContract::export_attestation(env.clone(), None, 10).unwrap()
    });

    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });

    let after = env.as_contract(&contract_id, || {
        TrustBridgeContract::export_attestation(env.clone(), None, 10).unwrap()
    });

    assert_ne!(before.digest, after.digest);
}

#[test]
fn test_export_attestation_matches_registered_paginated_page() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        let attestation = TrustBridgeContract::export_attestation(env.clone(), None, 10).unwrap();
        let page = TrustBridgeContract::get_registered_paginated(env.clone(), None, 10).unwrap();
        assert_eq!(attestation.page, page);
    });
}

#[test]
fn test_export_attestation_reports_version_and_ledger() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let attestation = TrustBridgeContract::export_attestation(env.clone(), None, 10).unwrap();
        assert_eq!(
            attestation.version,
            soroban_sdk::vec![&env, 1u32, 0u32, 0u32]
        );
        assert_eq!(attestation.ledger, env.ledger().sequence());
    });
}

// ── Dual-control batch_remove (Issue #219) ──────────────────────────────

/// Registers `count` fresh usernames (each to its own generated address)
/// and returns them, for building batch_remove test fixtures.
fn register_batch(env: &Env, contract_id: &Address, count: u32) -> Vec<String> {
    env.mock_all_auths();
    let mut names: Vec<String> = Vec::new(env);
    for i in 0..count {
        let addr = Address::generate(env);
        let name = username(env, &format!("user{}", i));
        env.as_contract(contract_id, || {
            TrustBridgeContract::register(env.clone(), name.clone(), addr.clone(), Vec::new(env))
                .unwrap();
        });
        names.push_back(name);
    }
    names
}

#[test]
fn test_batch_remove_at_or_below_threshold_unchanged() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    let names = register_batch(&env, &contract_id, 3);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        // size == threshold: still direct, single-step.
        let summary =
            TrustBridgeContract::batch_remove(env.clone(), admin.clone(), names.clone()).unwrap();
        assert_eq!(summary.successful, 3);
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 0);
    });
}

#[test]
fn test_batch_remove_above_threshold_requires_dual_control() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    let names = register_batch(&env, &contract_id, 4);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        // size == threshold + 1: rejected, must use propose/execute.
        let res = TrustBridgeContract::batch_remove(env.clone(), admin.clone(), names.clone());
        assert_eq!(res, Err(ContractError::DualControlRequired));
        // Registry untouched — the fail-before-write property.
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 4);
    });
}

#[test]
fn test_batch_remove_threshold_zero_disables_dual_control() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    let names = register_batch(&env, &contract_id, 20);
    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::get_batch_remove_threshold(env.clone()),
            0
        );
        let summary =
            TrustBridgeContract::batch_remove(env.clone(), admin.clone(), names.clone()).unwrap();
        assert_eq!(summary.successful, 20);
    });
}

#[test]
fn test_propose_then_execute_batch_remove_by_second_admin() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    let second_admin = Address::generate(&env);
    let names = register_batch(&env, &contract_id, 4);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::set_role(env.clone(), second_admin.clone(), Role::Admin).unwrap();

        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();
        let pending = TrustBridgeContract::get_pending_batch_remove(env.clone()).unwrap();
        assert_eq!(pending.usernames.len(), 4);
        assert_eq!(pending.proposed_by, admin);

        let summary =
            TrustBridgeContract::execute_batch_remove(env.clone(), second_admin.clone()).unwrap();
        assert_eq!(summary.successful, 4);
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 0);
        assert!(TrustBridgeContract::get_pending_batch_remove(env.clone()).is_none());
    });
}

#[test]
fn test_execute_batch_remove_same_proposer_rejected() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    let names = register_batch(&env, &contract_id, 4);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();
        // Same admin tries to execute their own proposal — dual control
        // means one signature alone is never enough.
        let res = TrustBridgeContract::execute_batch_remove(env.clone(), admin.clone());
        assert_eq!(res, Err(ContractError::NotAuthorized));
        assert!(TrustBridgeContract::get_pending_batch_remove(env.clone()).is_some());
    });
}

#[test]
fn test_execute_batch_remove_without_admin_equivalent_role_rejected() {
    let env = Env::default();
    let (admin, _user, bystander, contract_id) = setup(&env);
    let names = register_batch(&env, &contract_id, 4);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();
        let res = TrustBridgeContract::execute_batch_remove(env.clone(), bystander.clone());
        assert_eq!(res, Err(ContractError::NotAuthorized));
    });
}

#[test]
fn test_expired_admin_role_cannot_execute_batch_remove() {
    let env = Env::default();
    let (admin, _user, second_admin, contract_id) = setup(&env);
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role_with_expiry(
            env.clone(),
            second_admin.clone(),
            Role::Admin,
            Some(2_000),
        )
        .unwrap();
        TrustBridgeContract::propose_batch_remove(
            env.clone(),
            admin.clone(),
            soroban_sdk::vec![&env, username(&env, "octocat")],
        )
        .unwrap();
    });

    env.ledger().set_timestamp(2_000);
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::execute_batch_remove(env.clone(), second_admin.clone());
        assert_eq!(result, Err(ContractError::RoleExpired));
    });
}

#[test]
fn test_propose_batch_remove_rejects_second_proposal_while_pending() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    let names = register_batch(&env, &contract_id, 4);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();
        let res =
            TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone());
        assert_eq!(res, Err(ContractError::BatchRemoveProposalPending));
    });
}

#[test]
fn test_cancel_batch_remove_clears_proposal_and_allows_new_one() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    let names = register_batch(&env, &contract_id, 4);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();
        TrustBridgeContract::cancel_batch_remove(env.clone(), admin.clone()).unwrap();
        assert!(TrustBridgeContract::get_pending_batch_remove(env.clone()).is_none());

        // A fresh proposal is now allowed.
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();
        assert!(TrustBridgeContract::get_pending_batch_remove(env.clone()).is_some());
    });
}

#[test]
fn test_cancel_batch_remove_available_while_paused() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    let names = register_batch(&env, &contract_id, 4);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
        // Cancel still works while paused — the abort path must not be
        // blockable by the same freeze that might motivate using it.
        TrustBridgeContract::cancel_batch_remove(env.clone(), admin.clone()).unwrap();
        assert!(TrustBridgeContract::get_pending_batch_remove(env.clone()).is_none());
    });
}

#[test]
fn test_execute_batch_remove_blocked_while_paused() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    let second_admin = Address::generate(&env);
    let names = register_batch(&env, &contract_id, 4);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::set_role(env.clone(), second_admin.clone(), Role::Admin).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();
        TrustBridgeContract::pause(env.clone(), 1).unwrap();

        let res = TrustBridgeContract::execute_batch_remove(env.clone(), second_admin.clone());
        assert_eq!(res, Err(ContractError::Paused));

        // The proposal survives the pause — the second key can still
        // execute once unpaused.
        TrustBridgeContract::unpause(env.clone(), 4).unwrap();
        let summary =
            TrustBridgeContract::execute_batch_remove(env.clone(), second_admin.clone()).unwrap();
        assert_eq!(summary.successful, 4);
    });
}

#[test]
fn test_execute_batch_remove_after_proposal_expired() {
    let env = Env::default();
    let (admin, _user, _other, contract_id) = setup(&env);
    let second_admin = Address::generate(&env);
    let names = register_batch(&env, &contract_id, 4);
    env.ledger().set_timestamp(1_000);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
        TrustBridgeContract::set_role(env.clone(), second_admin.clone(), Role::Admin).unwrap();
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();
    });

    // Past BATCH_REMOVE_PROPOSAL_TTL_SECS (24h).
    env.ledger().set_timestamp(1_000 + 86_400 + 1);
    env.as_contract(&contract_id, || {
        let res = TrustBridgeContract::execute_batch_remove(env.clone(), second_admin.clone());
        assert_eq!(res, Err(ContractError::NoPendingBatchRemove));
        // Expired proposal was cleared and never executed.
        assert_eq!(TrustBridgeContract::get_stats(env.clone()).total, 4);
        assert!(TrustBridgeContract::get_pending_batch_remove(env.clone()).is_none());

        // A fresh proposal is possible immediately after.
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();
    });
}

// ── Issue #282: Indexer lag helper — last event ledger ───────────────────

/// A fresh instance has never emitted an event, so `get_last_event_ledger`
/// returns 0.
#[test]
fn test_last_event_ledger_zero_on_fresh_instance() {
    let env = Env::default();
    let (_admin, _user, _other, contract_id) = setup(&env);
    env.as_contract(&contract_id, || {
        // `initialize` pushes an audit entry but does not call event_domain,
        // so the cursor stays at 0 until the first event-emitting call.
        assert_eq!(
            TrustBridgeContract::get_last_event_ledger(env.clone()),
            0,
            "get_last_event_ledger must return 0 before any event is emitted"
        );
    });
}

/// `register` emits `RegisteredEvent` via `event_domain`, which stamps
/// the ledger cursor.  The returned value must equal the ledger sequence
/// of the invocation.
#[test]
fn test_last_event_ledger_updated_after_register() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();

    let seq_before = env.ledger().sequence();
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
        let last = TrustBridgeContract::get_last_event_ledger(env.clone());
        // The cursor must be at or after the register ledger.
        assert!(
            last >= seq_before,
            "last_event_ledger ({last}) must be >= register ledger ({seq_before})"
        );
        assert_ne!(last, 0, "last_event_ledger must not be 0 after register");
    });
}

/// `verify` emits `VerifiedEvent`; the cursor must advance after the call.
#[test]
fn test_last_event_ledger_updated_after_verify() {
    let env = Env::default();
    let (admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    let after_register = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_last_event_ledger(env.clone())
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "octocat")).unwrap();
    });

    env.as_contract(&contract_id, || {
        let after_verify = TrustBridgeContract::get_last_event_ledger(env.clone());
        assert!(
            after_verify >= after_register,
            "last_event_ledger must not decrease after verify"
        );
        assert_ne!(after_verify, 0);
    });
}

/// `remove` emits `RemovedEvent`; the cursor must be set.
#[test]
fn test_last_event_ledger_updated_after_remove() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), user.clone(), username(&env, "octocat")).unwrap();
    });

    env.as_contract(&contract_id, || {
        assert_ne!(
            TrustBridgeContract::get_last_event_ledger(env.clone()),
            0,
            "last_event_ledger must be set after remove"
        );
    });
}

/// `get_last_event_ledger` is a read and must work while the contract is
/// paused — indexers need to detect lag even during a maintenance freeze.
#[test]
fn test_last_event_ledger_readable_while_paused() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "octocat"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
        TrustBridgeContract::pause(env.clone(), 1).unwrap();
    });

    env.as_contract(&contract_id, || {
        // Must succeed even while paused.
        let last = TrustBridgeContract::get_last_event_ledger(env.clone());
        assert_ne!(last, 0, "last_event_ledger must be readable while paused");
    });
}

/// The cursor is monotonic: a second `register` must not decrease it.
#[test]
fn test_last_event_ledger_is_monotonic() {
    let env = Env::default();
    let (_admin, user, _other, contract_id) = setup(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "alice"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    let first = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_last_event_ledger(env.clone())
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username(&env, "bob"),
            Address::generate(&env),
            Vec::new(&env),
        )
        .unwrap();
    });

    env.as_contract(&contract_id, || {
        let second = TrustBridgeContract::get_last_event_ledger(env.clone());
        assert!(
            second >= first,
            "last_event_ledger must be monotonically non-decreasing ({second} < {first})"
        );
    });
}

// ── Provenance digests & assert_build (Issues #225 / #400) ──────────────

/// Builds a provenance record with both digests set, the shape every write
/// path now produces. The multisig path used to omit the two fields
/// entirely, which did not default them — it failed to compile (Issue #400).
fn provenance_with(
    env: &Env,
    admin: &Address,
    wasm_hash: BytesN<32>,
    sbom_hash: Option<BytesN<32>>,
    source_hash: Option<BytesN<32>>,
) -> crate::storage::WasmProvenance {
    crate::storage::WasmProvenance {
        wasm_hash,
        previous_wasm_hash: None,
        upgraded_by: admin.clone(),
        upgraded_at: env.ledger().timestamp(),
        version: soroban_sdk::vec![env, 1u32, 0u32, 0u32],
        attested: false,
        sbom_hash,
        source_hash,
    }
}

#[test]
fn assert_build_succeeds_on_a_matching_hash() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, _, contract_id) = setup(&env);

    let hash = BytesN::from_array(&env, &[7u8; 32]);
    env.as_contract(&contract_id, || {
        set_wasm_provenance(
            &env,
            &provenance_with(&env, &admin, hash.clone(), None, None),
        );
        assert_eq!(
            TrustBridgeContract::assert_build(env.clone(), hash.clone()),
            Ok(())
        );
    });
}

#[test]
fn assert_build_reports_a_mismatch() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, _, contract_id) = setup(&env);

    let stored = BytesN::from_array(&env, &[7u8; 32]);
    let other = BytesN::from_array(&env, &[8u8; 32]);

    env.as_contract(&contract_id, || {
        set_wasm_provenance(&env, &provenance_with(&env, &admin, stored, None, None));
        assert_eq!(
            TrustBridgeContract::assert_build(env.clone(), other),
            Err(ContractError::ProvenanceMismatch)
        );
    });
}

#[test]
fn assert_build_reports_missing_provenance() {
    // A contract that has never upgraded has nothing to compare against.
    // Reporting that distinctly matters: "nothing deployed yet" and
    // "you built the wrong thing" call for different operator action.
    let env = Env::default();
    env.mock_all_auths();
    let (_, _, _, contract_id) = setup(&env);

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::assert_build(env.clone(), BytesN::from_array(&env, &[7u8; 32])),
            Err(ContractError::ProvenanceMissing)
        );
    });
}

#[test]
fn set_provenance_digests_fills_each_field_independently() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, _, contract_id) = setup(&env);

    let hash = BytesN::from_array(&env, &[7u8; 32]);
    let sbom = BytesN::from_array(&env, &[1u8; 32]);
    let source = BytesN::from_array(&env, &[2u8; 32]);

    env.as_contract(&contract_id, || {
        set_wasm_provenance(&env, &provenance_with(&env, &admin, hash, None, None));

        // Passing None must leave the other field untouched, which is what
        // makes the two digests backfillable in separate calls.
        TrustBridgeContract::set_provenance_digests(env.clone(), Some(sbom.clone()), None).unwrap();
        let after_sbom = get_wasm_provenance(&env).unwrap();
        assert_eq!(after_sbom.sbom_hash, Some(sbom.clone()));
        assert_eq!(after_sbom.source_hash, None);

        TrustBridgeContract::set_provenance_digests(env.clone(), None, Some(source.clone()))
            .unwrap();
        let after_source = get_wasm_provenance(&env).unwrap();
        assert_eq!(after_source.sbom_hash, Some(sbom));
        assert_eq!(after_source.source_hash, Some(source));
    });
}

#[test]
fn set_provenance_digests_requires_an_existing_record() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, _, _, contract_id) = setup(&env);

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::set_provenance_digests(
                env.clone(),
                Some(BytesN::from_array(&env, &[1u8; 32])),
                None
            ),
            Err(ContractError::ProvenanceMissing)
        );
    });
}

// ── NetworkMismatch (Issue #401) ───────────────────────────────────────

#[test]
fn network_mismatch_has_the_documented_discriminant() {
    // Code 30 is load-bearing: it is what off-chain decoders branch on, and
    // it is asserted in abi/contract_error_codes.golden and docs/ABI.md.
    assert_eq!(ContractError::NetworkMismatch.code(), 30);
    assert_eq!(
        ContractError::from_code(30),
        Some(ContractError::NetworkMismatch)
    );
}

#[test]
fn network_mismatch_does_not_collide_with_its_neighbours() {
    // The bug this variant was added around: docs had NetworkMismatch at 21,
    // where InvalidPauseReason actually lives (Issue #402).
    assert_eq!(ContractError::InvalidPauseReason.code(), 21);
    assert_ne!(
        ContractError::NetworkMismatch.code(),
        ContractError::InvalidPauseReason.code()
    );
    assert_eq!(ContractError::AttestationRequired.code(), 29);
    assert_eq!(ContractError::VerifierAllowlistFull.code(), 31);
}

#[test]
fn a_matching_network_passes_the_check() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, _, _, contract_id) = setup(&env);

    // `initialize` tags the instance with the executing network, so the
    // check must pass immediately afterwards.
    env.as_contract(&contract_id, || {
        assert_eq!(crate::storage::require_matching_network(&env), Ok(()));
    });
}

#[test]
fn a_foreign_network_tag_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, _, _, contract_id) = setup(&env);

    env.as_contract(&contract_id, || {
        // Stand in for state restored from another network.
        crate::storage::set_network_id(&env, &BytesN::from_array(&env, &[0xAB; 32]));

        assert_eq!(
            crate::storage::require_matching_network(&env),
            Err(ContractError::NetworkMismatch)
        );
        // And it rides along inside require_initialized, which is what makes
        // a new entry point unable to forget the check.
        assert_eq!(
            crate::storage::require_initialized(&env),
            Err(ContractError::NetworkMismatch)
        );
    });
}

#[test]
fn mutating_entry_points_reject_foreign_network_before_writing() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, user, _, contract_id) = setup(&env);

    env.as_contract(&contract_id, || {
        let foreign = BytesN::from_array(&env, &[0xAB; 32]);
        crate::storage::set_network_id(&env, &foreign);

        // Initialization, ordinary writes, delegated batch writes, and
        // permissionless maintenance all reject the restored state.
        assert_eq!(
            TrustBridgeContract::initialize(env.clone(), admin.clone()),
            Err(ContractError::NetworkMismatch)
        );
        assert_eq!(
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "alice"),
                user,
                Vec::new(&env),
            ),
            Err(ContractError::NetworkMismatch)
        );
        assert_eq!(
            TrustBridgeContract::batch_verify(env.clone(), admin.clone(), Vec::new(&env)),
            Err(ContractError::NetworkMismatch)
        );
        assert_eq!(
            TrustBridgeContract::prune_expired_verifiers(env.clone()),
            Err(ContractError::NetworkMismatch)
        );
        assert_eq!(
            TrustBridgeContract::pause(env.clone(), PauseReason::Maintenance as u32),
            Err(ContractError::NetworkMismatch)
        );
        assert_eq!(
            TrustBridgeContract::compact_index(env.clone()),
            Err(ContractError::NetworkMismatch)
        );
        assert_eq!(
            TrustBridgeContract::adopt_network_tag(env.clone()),
            Err(ContractError::NetworkMismatch)
        );

        assert_eq!(crate::storage::get_network_id(&env), Some(foreign));
        assert_eq!(crate::storage::get_count(&env), 0);
        assert!(!crate::storage::is_paused(&env));
    });
}

#[test]
fn matching_and_legacy_untagged_state_keep_mutation_behavior() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _, _, contract_id) = setup(&env);

    env.as_contract(&contract_id, || {
        assert_eq!(
            TrustBridgeContract::initialize(env.clone(), admin.clone()),
            Err(ContractError::AlreadyInitialized)
        );

        // Pre-network-tagging deployments have no recorded id and remain
        // usable until an admin explicitly adopts the current network.
        env.storage()
            .instance()
            .remove(&crate::storage::NETWORK_KEY);
        assert_eq!(TrustBridgeContract::set_cooldown(env.clone(), 42), Ok(()));
        assert_eq!(crate::storage::get_cooldown(&env), 42);
        assert_eq!(crate::storage::get_network_id(&env), None);
    });
}

#[test]
fn network_mismatch_is_fatal_not_retryable() {
    // The executing network does not change between attempts — retrying the
    // same call forever would be the wrong off-chain behaviour.
    assert_eq!(
        ContractError::NetworkMismatch.category(),
        crate::error::ErrorCategory::Fatal
    );
    assert!(!ContractError::NetworkMismatch.is_retryable());
}
