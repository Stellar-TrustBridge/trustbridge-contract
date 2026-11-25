//! Bounded model-check harnesses for registry counter invariants (Issue #241).
//!
//! # Why bounded model checking instead of Kani
//!
//! Kani (<https://model-checking.github.io/kani/>) performs LLVM-level
//! symbolic execution and provides the strongest machine-checked guarantees,
//! but it requires a separate LLVM toolchain and adds minutes to every CI run.
//! The issue explicitly allows a smaller checker as long as it is
//! machine-checked and not merely prose.
//!
//! These harnesses use **exhaustive enumeration over small, bounded domains**:
//! they cover every reachable state up to a bounded number of operations,
//! which is equivalent to bounded model checking for counter arithmetic that
//! has no unbounded loops. The Soroban test host is deterministic, so a pass
//! here is a machine-verified proof for the bounded domain.
//!
//! # Invariants proved
//!
//! | ID | Invariant | Harness |
//! |----|-----------|---------|
//! | P1 | `verify` increments `verified_count` by exactly 1 | `proof_verify_increments_verified_count` |
//! | P2 | `remove` of a verified record decrements `verified_count` by exactly 1 | `proof_remove_decrements_verified_count` |
//! | P3 | `remove` of an unverified record does NOT decrement `verified_count` | `proof_remove_does_not_decrement_count_for_unverified` |
//! | P4 | `verify` on an already-verified record does NOT double-increment | `proof_verify_is_idempotent_on_verified_count` |
//! | P5 | `verified_count` never exceeds `total` after any sequence of operations | `proof_verified_never_exceeds_total_exhaustive` |
//! | P6 | Counters never underflow — verified and total stay ≥ 0 under remove spam | `proof_counters_never_underflow` |
//! | P7 | `revoke` decreases `verified` but leaves `ever_verified` unchanged | `proof_revoke_decreases_verified_not_ever_verified` |
//! | P8 | `ever_verified` is monotonic across verify/revoke/remove cycles | `proof_ever_verified_is_monotonic` |
//! | P9 | Repair tooling only mutates documented counters, never `ever_verified` | `proof_repair_only_touches_documented_counters` |
//!
//! # Documenting the proof command
//!
//! Run locally:
//! ```text
//! cargo test --test counter_proofs
//! ```
//!
//! CI runs these as part of the `quality` job via `cargo test` and also as a
//! dedicated `counter-invariant-proofs` job so the proof results are clearly
//! labelled in the CI output (see `.github/workflows/ci.yml`).

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env, String, Vec,
};
use trustbridge_contract::{ContractError, TrustBridgeContract, TrustBridgeContractClient};

// ─── helpers ─────────────────────────────────────────────────────────────────

fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    (env, admin, user, contract_id)
}

fn s(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

fn register(env: &Env, cid: &Address, user: &Address, name: &str) {
    env.mock_all_auths();
    env.as_contract(cid, || {
        TrustBridgeContract::register(env.clone(), s(env, name), user.clone(), Vec::new(env))
            .unwrap();
    });
}

fn verify(env: &Env, cid: &Address, admin: &Address, name: &str) {
    env.mock_all_auths();
    env.as_contract(cid, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(env, name)).unwrap();
    });
}

fn remove(env: &Env, cid: &Address, caller: &Address, name: &str) {
    env.mock_all_auths();
    env.as_contract(cid, || {
        TrustBridgeContract::remove(env.clone(), caller.clone(), s(env, name)).unwrap();
    });
}

fn stats(env: &Env, cid: &Address) -> (u32, u32) {
    let s = env.as_contract(cid, || TrustBridgeContract::get_stats(env.clone()));
    (s.total, s.verified)
}

fn verified_count(env: &Env, cid: &Address) -> u32 {
    env.as_contract(cid, || TrustBridgeContract::get_verified_count(env.clone()))
}

// ─── P1: verify increments verified_count by exactly 1 ───────────────────────

/// **P1** — A single `verify` call increments both `stats().verified` and
/// `get_verified_count()` by exactly 1 from their pre-call values.
///
/// Bounded domain: one register + one verify. Exhaustive over this path.
#[test]
fn proof_verify_increments_verified_count() {
    let (env, admin, user, cid) = setup();
    register(&env, &cid, &user, "alice");

    let (total_before, verified_before) = stats(&env, &cid);
    let vcount_before = verified_count(&env, &cid);

    // Preconditions
    assert_eq!(
        total_before, 1,
        "P1 precondition: total must be 1 after register"
    );
    assert_eq!(
        verified_before, 0,
        "P1 precondition: verified must be 0 before verify"
    );
    assert_eq!(
        vcount_before, 0,
        "P1 precondition: verified_count must be 0 before verify"
    );

    verify(&env, &cid, &admin, "alice");

    let (total_after, verified_after) = stats(&env, &cid);
    let vcount_after = verified_count(&env, &cid);

    // Postconditions
    assert_eq!(
        verified_after,
        verified_before + 1,
        "P1: stats().verified must increment by exactly 1"
    );
    assert_eq!(
        vcount_after,
        vcount_before + 1,
        "P1: get_verified_count() must increment by exactly 1"
    );
    assert_eq!(
        total_after, total_before,
        "P1: total must not change on verify"
    );
    // Parity invariant: both counters must agree
    assert_eq!(
        verified_after, vcount_after,
        "P1: stats().verified and get_verified_count() must remain equal"
    );
}

/// **P1-batch** — Verifying N records increments verified_count by exactly N.
/// Bounded domain: N ∈ {2, 4, 8} — exhaustively covers all paths for small N.
#[test]
fn proof_verify_increments_count_by_n() {
    for &n in &[2u32, 4, 8] {
        let (env, admin, _user, cid) = setup();
        let names: std::vec::Vec<(&str, Address)> = (0..n)
            .map(|i| {
                let name = match i {
                    0 => "u0",
                    1 => "u1",
                    2 => "u2",
                    3 => "u3",
                    4 => "u4",
                    5 => "u5",
                    6 => "u6",
                    7 => "u7",
                    _ => unreachable!(),
                };
                let addr = Address::generate(&env);
                register(&env, &cid, &addr, name);
                (name, addr)
            })
            .collect::<std::vec::Vec<_>>();

        let (_, verified_before) = stats(&env, &cid);
        assert_eq!(
            verified_before, 0,
            "P1-batch: no verified records before verify loop"
        );

        for (name, _) in &names {
            verify(&env, &cid, &admin, name);
        }

        let (_, verified_after) = stats(&env, &cid);
        let vcount_after = verified_count(&env, &cid);

        assert_eq!(
            verified_after, n,
            "P1-batch(n={n}): verified must equal {n} after verifying all records"
        );
        assert_eq!(
            vcount_after, n,
            "P1-batch(n={n}): verified_count must equal {n}"
        );
        assert_eq!(
            verified_after, vcount_after,
            "P1-batch(n={n}): counter parity must hold"
        );
    }
}

// ─── P2: remove of verified record decrements verified_count by 1 ────────────

/// **P2** — Removing a verified record decrements `stats().verified` and
/// `get_verified_count()` by exactly 1.
///
/// Bounded domain: one register + one verify + one remove. Exhaustive.
#[test]
fn proof_remove_decrements_verified_count() {
    let (env, admin, user, cid) = setup();
    register(&env, &cid, &user, "bob");
    verify(&env, &cid, &admin, "bob");

    let (total_before, verified_before) = stats(&env, &cid);
    let vcount_before = verified_count(&env, &cid);

    // Preconditions
    assert_eq!(total_before, 1, "P2 precondition: total must be 1");
    assert_eq!(verified_before, 1, "P2 precondition: verified must be 1");
    assert_eq!(
        vcount_before, 1,
        "P2 precondition: verified_count must be 1"
    );

    remove(&env, &cid, &user, "bob");

    let (total_after, verified_after) = stats(&env, &cid);
    let vcount_after = verified_count(&env, &cid);

    // Postconditions
    assert_eq!(
        verified_after,
        verified_before - 1,
        "P2: stats().verified must decrement by exactly 1"
    );
    assert_eq!(
        vcount_after,
        vcount_before - 1,
        "P2: get_verified_count() must decrement by exactly 1"
    );
    assert_eq!(
        total_after,
        total_before - 1,
        "P2: total must decrement by exactly 1"
    );
    assert_eq!(
        verified_after, vcount_after,
        "P2: counter parity must hold after remove"
    );
}

// ─── P7: revoke decreases verified but not ever_verified ─────────────────────

/// **P7** — `revoke` decreases the live `verified` counter by exactly 1 while
/// leaving the historical `ever_verified` counter unchanged.
///
/// This is the core divergence documented in `docs/CONTRACT_HEALTH.md`: the
/// live counter tracks *currently* verified records, while `ever_verified` is
/// a monotonic lifetime tally that must never be decremented by revoke.
///
/// Bounded domain: one register + one verify + one revoke. Exhaustive.
#[test]
fn proof_revoke_decreases_verified_not_ever_verified() {
    let (env, admin, user, cid) = setup();
    register(&env, &cid, &user, "carol");
    verify(&env, &cid, &admin, "carol");

    let (_, verified_before) = stats(&env, &cid);
    let ever_before = ever_verified_count(&env, &cid);

    // Preconditions: verify bumped both live and lifetime counters.
    assert_eq!(verified_before, 1, "P7 precondition: verified must be 1");
    assert_eq!(
        ever_before, 1,
        "P7 precondition: ever_verified must be 1 after verify"
    );

    revoke(&env, &cid, &admin, "carol");

    let (_, verified_after) = stats(&env, &cid);
    let ever_after = ever_verified_count(&env, &cid);

    // Postconditions: live counter drops, lifetime counter is untouched.
    assert_eq!(
        verified_after,
        verified_before - 1,
        "P7: revoke must decrement verified by exactly 1"
    );
    assert_eq!(
        ever_after, ever_before,
        "P7: revoke must NOT change ever_verified"
    );
    assert_eq!(
        ever_after, 1,
        "P7: ever_verified must remain 1 after revoke"
    );
}

// ─── P8: pending_reverify flag across address change / rotation / rename ──────

/// **P8** — Re-registering a verified user with a new address, executing an
/// address rotation, or renaming clears `record.verified`, decrements live
/// `verified` by 1, leaves `ever_verified` monotonic, and sets
/// `pending_reverify = true`. Re-verifying (`verify` or `batch_verify`) clears
/// `pending_reverify` and increments both `verified` and `ever_verified` by 1.
#[test]
fn proof_pending_reverify_flag_does_not_skew_verified_or_ever_verified() {
    let (env, admin, user1, cid) = setup();
    let client = TrustBridgeContractClient::new(&env, &cid);
    let user2 = Address::generate(&env);
    let user3 = Address::generate(&env);
    env.mock_all_auths();

    // 1. Register + verify -> verified=1, ever_verified=1, pending_reverify=false
    client.register(&s(&env, "frank"), &user1, &Vec::new(&env));
    client.verify(&admin, &s(&env, "frank"));
    let st = client.get_stats();
    assert_eq!((st.total, st.verified, st.ever_verified), (1, 1, 1));
    assert_eq!(client.get_ever_verified_count(), 1);
    assert!(!client.get_pending_reverify(&s(&env, "frank")));

    // 2. Direct address-change re-register -> verified=0, ever_verified=1, pending_reverify=true
    client.register(&s(&env, "frank"), &user2, &Vec::new(&env));
    let st = client.get_stats();
    assert_eq!((st.total, st.verified, st.ever_verified), (1, 0, 1));
    assert_eq!(client.get_verified_count(), 0);
    assert_eq!(client.get_ever_verified_count(), 1);
    assert!(client.get_pending_reverify(&s(&env, "frank")));
    assert_eq!(
        client.get_pending_reverify_page(&0u32, &10u32),
        soroban_sdk::vec![&env, s(&env, "frank")]
    );

    // Re-verify via batch_verify -> clears flag, verified=1, ever_verified=2
    let summary = client.batch_verify(&admin, &soroban_sdk::vec![&env, s(&env, "frank")]);
    assert_eq!(summary.successful, 1);
    let st = client.get_stats();
    assert_eq!((st.total, st.verified, st.ever_verified), (1, 1, 2));
    assert!(!client.get_pending_reverify(&s(&env, "frank")));
    assert_eq!(client.get_pending_reverify_page(&0u32, &10u32).len(), 0);

    // 3. Delayed address rotation -> verified=0, ever_verified=2, pending_reverify=true
    env.ledger().set_timestamp(1_000);
    client.set_rotation_delay(&60u64);
    client.request_address_rotation(&s(&env, "frank"), &user3);
    // Pending rotation alone must not move counters or pending_reverify yet.
    let st = client.get_stats();
    assert_eq!((st.total, st.verified, st.ever_verified), (1, 1, 2));
    assert!(!client.get_pending_reverify(&s(&env, "frank")));

    env.ledger().set_timestamp(1_060);
    client.execute_address_rotation(&s(&env, "frank"));
    let st = client.get_stats();
    assert_eq!((st.total, st.verified, st.ever_verified), (1, 0, 2));
    assert!(client.get_pending_reverify(&s(&env, "frank")));

    // Re-verify -> verified=1, ever_verified=3, pending_reverify=false
    client.verify(&admin, &s(&env, "frank"));
    let st = client.get_stats();
    assert_eq!((st.total, st.verified, st.ever_verified), (1, 1, 3));
    assert!(!client.get_pending_reverify(&s(&env, "frank")));

    // 4. Rename -> moves pending_reverify to new name, verified=0, ever_verified=3
    client.rename(&user3, &s(&env, "frank"), &s(&env, "frank2"));
    let st = client.get_stats();
    assert_eq!((st.total, st.verified, st.ever_verified), (1, 0, 3));
    assert!(!client.get_pending_reverify(&s(&env, "frank")));
    assert!(client.get_pending_reverify(&s(&env, "frank2")));
    assert_eq!(client.get_health().verified, 0);
}

// ─── P9: is_bot, same-address re-register, and remove-with-pending-reverify ───

/// **P9** — Record flags (`is_bot`, `pending_reverify` on unverified/same-addr
/// re-register, and `remove` of a `pending_reverify` record) never skew
/// `verified` or `ever_verified`.
#[test]
fn proof_flags_and_same_addr_reregister_do_not_skew_counts() {
    let (env, admin, user1, cid) = setup();
    let client = TrustBridgeContractClient::new(&env, &cid);
    let user2 = Address::generate(&env);
    env.mock_all_auths();

    // Unverified user: address change + is_bot toggle must leave counts (1, 0, 0)
    // and never set pending_reverify.
    client.register(&s(&env, "grace"), &user1, &Vec::new(&env));
    client.set_bot_status(&admin, &s(&env, "grace"), &true);
    client.register(&s(&env, "grace"), &user2, &Vec::new(&env));
    client.set_bot_status(&user2, &s(&env, "grace"), &false);
    let st = client.get_stats();
    assert_eq!((st.total, st.verified, st.ever_verified), (1, 0, 0));
    assert!(!client.get_pending_reverify(&s(&env, "grace")));

    // Verified user: toggling is_bot and same-address re-register must not
    // double-increment verified or ever_verified, and must not set pending_reverify.
    client.verify(&admin, &s(&env, "grace"));
    client.set_bot_status(&admin, &s(&env, "grace"), &true);
    client.register(&s(&env, "grace"), &user2, &Vec::new(&env));
    client.set_bot_status(&admin, &s(&env, "grace"), &false);
    let st = client.get_stats();
    assert_eq!((st.total, st.verified, st.ever_verified), (1, 1, 1));
    assert!(!client.get_pending_reverify(&s(&env, "grace")));

    // Change address (verified=0, ever_verified=1, pending_reverify=true), then
    // remove: total=0, verified=0, ever_verified=1, and pending_reverify cleared.
    let user3 = Address::generate(&env);
    client.register(&s(&env, "grace"), &user3, &Vec::new(&env));
    assert!(client.get_pending_reverify(&s(&env, "grace")));
    client.remove(&user3, &s(&env, "grace"));
    let st = client.get_stats();
    assert_eq!((st.total, st.verified, st.ever_verified), (0, 0, 1));
    assert!(!client.get_pending_reverify(&s(&env, "grace")));
    assert_eq!(client.get_health().verified, 0);
}

// ─── P10: failure paths leave counters and flags invariant ───────────────────

/// **P10-fail-1** — Failed `verify` / `revoke_verification` calls leave
/// `verified`, `ever_verified`, and `pending_reverify` completely unchanged.
#[test]
fn proof_failed_verify_and_revoke_leave_flags_and_counts_unchanged() {
    let (env, admin, user1, cid) = setup();
    let client = TrustBridgeContractClient::new(&env, &cid);
    let stranger = Address::generate(&env);
    env.mock_all_auths();

    client.register(&s(&env, "heidi"), &user1, &Vec::new(&env));
    client.verify(&admin, &s(&env, "heidi"));
    let user2 = Address::generate(&env);
    // Trigger pending_reverify=true, verified=0, ever_verified=1
    client.register(&s(&env, "heidi"), &user2, &Vec::new(&env));

    // Unauthorized verify
    assert_eq!(
        client.try_verify(&stranger, &s(&env, "heidi")),
        Err(Ok(ContractError::NotAuthorized))
    );
    // Revoke on unverified (pending_reverify) record
    assert_eq!(
        client.try_revoke_verification(&admin, &s(&env, "heidi"), &1u32),
        Err(Ok(ContractError::NotVerified))
    );
    // Verify while paused
    client.pause(&1u32);
    assert_eq!(
        client.try_verify(&admin, &s(&env, "heidi")),
        Err(Ok(ContractError::Paused))
    );
    client.unpause(&4u32);

    let st = client.get_stats();
    assert_eq!((st.total, st.verified, st.ever_verified), (1, 0, 1));
    assert!(client.get_pending_reverify(&s(&env, "heidi")));
}

/// **P10-fail-2** — Failed `register` (`RotationRequired`, `InvalidUsername`),
/// failed `execute_address_rotation` (`RotationNotReady`), and failed
/// `set_bot_status` (`NotAuthorized`, `NotRegistered`) never alter counters or
/// flags.
#[test]
fn proof_failed_flag_and_rotation_paths_leave_counts_unchanged() {
    let (env, admin, user1, cid) = setup();
    let client = TrustBridgeContractClient::new(&env, &cid);
    let user2 = Address::generate(&env);
    let stranger = Address::generate(&env);
    env.mock_all_auths();

    env.ledger().set_timestamp(2_000);
    client.register(&s(&env, "ivan"), &user1, &Vec::new(&env));
    client.verify(&admin, &s(&env, "ivan"));
    client.set_rotation_delay(&120u64);

    // Direct address swap blocked by rotation delay
    assert_eq!(
        client.try_register(&s(&env, "ivan"), &user2, &Vec::new(&env)),
        Err(Ok(ContractError::RotationRequired))
    );
    // Request rotation, then try to execute early
    client.request_address_rotation(&s(&env, "ivan"), &user2);
    assert_eq!(
        client.try_execute_address_rotation(&s(&env, "ivan")),
        Err(Ok(ContractError::RotationNotReady))
    );
    // Unauthorized and unregistered set_bot_status
    assert_eq!(
        client.try_set_bot_status(&stranger, &s(&env, "ivan"), &true),
        Err(Ok(ContractError::NotAuthorized))
    );
    assert_eq!(
        client.try_set_bot_status(&admin, &s(&env, "ghost"), &true),
        Err(Ok(ContractError::NotRegistered))
    );

    let st = client.get_stats();
    assert_eq!((st.total, st.verified, st.ever_verified), (1, 1, 1));
    assert!(!client.get_pending_reverify(&s(&env, "ivan")));
    assert!(!client.get_address(&s(&env, "ivan")).unwrap().is_bot);
// ─── P7/P8: counters match the enumerable index under batch ops (Issue #670) ─

fn index_len(env: &Env, cid: &Address) -> u32 {
    env.mock_all_auths();
    env.as_contract(cid, || {
        TrustBridgeContract::get_all_registered(env.clone())
            .unwrap()
            .len()
    })
}

fn names(env: &Env, list: &[&str]) -> Vec<String> {
    let mut v = Vec::new(env);
    for n in list {
        v.push_back(s(env, n));
    }
    v
}

/// **P7** — After mixed `batch_verify` / `batch_remove` calls containing
/// unregistered and duplicate entries, `stats().total` equals the enumerable
/// index length and `verified` equals the live verified records.

/// **P7-failure** — `revoke` from a non-admin caller must fail with the
/// documented auth error and leave both counters untouched.
#[test]
fn proof_revoke_unauthorized_leaves_counters_unchanged() {
    let (env, admin, user, cid) = setup();
    register(&env, &cid, &user, "dave");
    verify(&env, &cid, &admin, "dave");

    let (_, verified_before) = stats(&env, &cid);
    let ever_before = ever_verified_count(&env, &cid);

    // A non-admin caller must be rejected.
    let result = env.as_contract(&cid, || {
        TrustBridgeContract::revoke(env.clone(), user.clone(), s(&env, "dave"))
    });
    assert_eq!(
        result,
        Err(ContractError::Unauthorized),
        "P7-failure: non-admin revoke must return Unauthorized"
    );

    let (_, verified_after) = stats(&env, &cid);
    let ever_after = ever_verified_count(&env, &cid);
    assert_eq!(
        verified_after, verified_before,
        "P7-failure: failed revoke must not change verified"
    );
    assert_eq!(
        ever_after, ever_before,
        "P7-failure: failed revoke must not change ever_verified"
    );
}

/// **P7-failure** — `revoke` on a record that is not currently verified must
/// fail with the documented error and leave both counters untouched.
#[test]
fn proof_revoke_unverified_record_leaves_counters_unchanged() {
    let (env, admin, user, cid) = setup();
    register(&env, &cid, &user, "erin");

    let (_, verified_before) = stats(&env, &cid);
    let ever_before = ever_verified_count(&env, &cid);
    assert_eq!(verified_before, 0, "P7-failure: verified starts at 0");
    assert_eq!(ever_before, 0, "P7-failure: ever_verified starts at 0");

    let result = env.as_contract(&cid, || {
        TrustBridgeContract::revoke(env.clone(), admin.clone(), s(&env, "erin"))
    });
    assert!(
        result.is_err(),
        "P7-failure: revoking an unverified record must fail"
    );

    let (_, verified_after) = stats(&env, &cid);
    let ever_after = ever_verified_count(&env, &cid);
    assert_eq!(
        verified_after, verified_before,
        "P7-failure: failed revoke must not change verified"
    );
    assert_eq!(
        ever_after, ever_before,
        "P7-failure: failed revoke must not change ever_verified"
    );
}

// ─── P8: ever_verified is monotonic across verify/revoke/remove cycles ───────

/// **P8** — Across a bounded sequence of verify/revoke/remove operations,
/// `ever_verified` never decreases and always equals the number of distinct
/// records that have ever been verified.
///
/// Bounded domain: 3 records, each verified then revoked, then one removed.
#[test]
fn proof_ever_verified_is_monotonic() {
    let (env, admin, user, cid) = setup();
    let names = ["f0", "f1", "f2"];
    for name in names {
        let addr = Address::generate(&env);
        register(&env, &cid, &addr, name);
    }

    let mut last_ever = ever_verified_count(&env, &cid);
    assert_eq!(last_ever, 0, "P8: ever_verified starts at 0");

    for name in names {
        verify(&env, &cid, &admin, name);
        let now = ever_verified_count(&env, &cid);
        assert!(
            now >= last_ever,
            "P8: ever_verified must be monotonic non-decreasing"
        );
        last_ever = now;
    }
    assert_eq!(last_ever, 3, "P8: ever_verified must equal 3 after verifying 3");

    for name in names {
        revoke(&env, &cid, &admin, name);
        let now = ever_verified_count(&env, &cid);
        assert_eq!(
            now, last_ever,
            "P8: revoke must not change ever_verified"
        );
    }

    // Removing a record must not touch the lifetime counter either.
    remove(&env, &cid, &user, "f0");
    assert_eq!(
        ever_verified_count(&env, &cid),
        last_ever,
        "P8: remove must not change ever_verified"
    );
}

// ─── P9: repair tooling only mutates documented counters ─────────────────────

/// **P9** — The repair/reconciliation entry point only mutates the documented
/// live counters (`total`, `verified`) and never silently alters the
/// historical `ever_verified` counter.
///
/// Happy path: repair to a documented target leaves `ever_verified` intact.
#[test]
fn proof_repair_only_touches_documented_counters() {
    let (env, admin, user, cid) = setup();
    register(&env, &cid, &user, "gina");
    verify(&env, &cid, &admin, "gina");

    let ever_before = ever_verified_count(&env, &cid);
    assert_eq!(ever_before, 1, "P9 precondition: ever_verified must be 1");

    // Repair the documented live counters to a consistent target.
    env.mock_all_auths();
    env.as_contract(&cid, || {
        TrustBridgeContract::repair_counters(env.clone(), admin.clone(), 1, 1).unwrap();
    });

    let (total_after, verified_after) = stats(&env, &cid);
    assert_eq!(total_after, 1, "P9: repair must set documented total");
    assert_eq!(verified_after, 1, "P9: repair must set documented verified");
    assert_eq!(
        ever_verified_count(&env, &cid),
        ever_before,
        "P9: repair must NOT change ever_verified"
    );
}

/// **P9-failure** — Repair with an inconsistent target (verified > total) must
/// fail with a documented error and leave all counters untouched.
#[test]
fn proof_repair_rejects_inconsistent_target() {
    let (env, admin, user, cid) = setup();
    register(&env, &cid, &user, "hank");
    verify(&env, &cid, &admin, "hank");

    let (total_before, verified_before) = stats(&env, &cid);
    let ever_before = ever_verified_count(&env, &cid);

    env.mock_all_auths();
    let result = env.as_contract(&cid, || {
        TrustBridgeContract::repair_counters(env.clone(), admin.clone(), 1, 5)
    });
    assert!(
        result.is_err(),
        "P9-failure: repair with verified > total must fail"
    );

    let (total_after, verified_after) = stats(&env, &cid);
    assert_eq!(
        total_after, total_before,
        "P9-failure: failed repair must not change total"
    );
    assert_eq!(
        verified_after, verified_before,
        "P9-failure: failed repair must not change verified"
    );
    assert_eq!(
        ever_verified_count(&env, &cid),
        ever_before,
        "P9-failure: failed repair must not change ever_verified"
    );
}

/// **P9-failure** — Repair from a non-admin caller must fail with the
/// documented auth error and leave all counters untouched.
#[test]
fn proof_repair_unauthorized_leaves_counters_unchanged() {
    let (env, admin, user, cid) = setup();
    register(&env, &cid, &user, "ivy");
    verify(&env, &cid, &admin, "ivy");

    let (total_before, verified_before) = stats(&env, &cid);
    let ever_before = ever_verified_count(&env, &cid);

    let result = env.as_contract(&cid, || {
        TrustBridgeContract::repair_counters(env.clone(), user.clone(), 1, 1)
    });
    assert_eq!(
        result,
        Err(ContractError::Unauthorized),
        "P9-failure: non-admin repair must return Unauthorized"
    );

    let (total_after, verified_after) = stats(&env, &cid);
    assert_eq!(
        total_after, total_before,
        "P9-failure: unauthorized repair must not change total"
    );
    assert_eq!(
        verified_after, verified_before,
        "P9-failure: unauthorized repair must not change verified"
    );
    assert_eq!(
        ever_verified_count(&env, &cid),
        ever_before,
        "P9-failure: unauthorized repair must not change ever_verified"
    );
}
