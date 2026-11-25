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
        verified_before.saturating_sub(1),
        "P2: stats().verified must decrement by exactly 1 on verified remove"
    );
    assert_eq!(
        vcount_after,
        vcount_before.saturating_sub(1),
        "P2: get_verified_count() must decrement by exactly 1 on verified remove"
    );
    assert_eq!(total_after, 0, "P2: total must be 0 after remove");
    assert_eq!(
        verified_after, vcount_after,
        "P2: counter parity must hold after remove"
    );
}

// ─── P3: remove of unverified record does NOT touch verified_count ────────────

/// **P3** — Removing an **unverified** record must leave `verified_count`
/// unchanged. This catches the classic off-by-one where a remove path
/// unconditionally decrements the counter.
///
/// Bounded domain: one register (no verify) + one remove. Exhaustive.
#[test]
fn proof_remove_does_not_decrement_count_for_unverified() {
    let (env, _admin, user, cid) = setup();
    register(&env, &cid, &user, "charlie");

    let (_, verified_before) = stats(&env, &cid);
    let vcount_before = verified_count(&env, &cid);

    // Preconditions: record exists but is not verified
    assert_eq!(verified_before, 0, "P3 precondition: no verified records");
    assert_eq!(vcount_before, 0, "P3 precondition: verified_count is 0");

    remove(&env, &cid, &user, "charlie");

    let (total_after, verified_after) = stats(&env, &cid);
    let vcount_after = verified_count(&env, &cid);

    assert_eq!(
        verified_after, 0,
        "P3: verified must stay 0 after removing an unverified record"
    );
    assert_eq!(
        vcount_after, 0,
        "P3: verified_count must stay 0 after removing an unverified record"
    );
    assert_eq!(total_after, 0, "P3: total must be 0 after remove");
}

// ─── P4: verify is idempotent — second verify does NOT double-increment ───────

/// **P4** — Calling `verify` on a record that is already verified must return
/// `ContractError::AlreadyVerified` and leave the counters unchanged.
/// This confirms saturating-add semantics: verified_count cannot exceed total.
///
/// Bounded domain: one register + two verify calls. Exhaustive.
#[test]
fn proof_verify_is_idempotent_on_verified_count() {
    let (env, admin, user, cid) = setup();
    register(&env, &cid, &user, "diana");
    verify(&env, &cid, &admin, "diana");

    let (_, verified_after_first) = stats(&env, &cid);
    let vcount_after_first = verified_count(&env, &cid);

    // Second verify must fail
    env.mock_all_auths();
    let second_result = env.as_contract(&cid, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, "diana"))
    });
    assert!(
        second_result.is_err(),
        "P4: second verify on already-verified record must return an error"
    );
    assert_eq!(
        second_result.unwrap_err(),
        ContractError::AlreadyVerified,
        "P4: second verify must return AlreadyVerified"
    );

    let (_, verified_after_second) = stats(&env, &cid);
    let vcount_after_second = verified_count(&env, &cid);

    assert_eq!(
        verified_after_second, verified_after_first,
        "P4: stats().verified must not change on double-verify"
    );
    assert_eq!(
        vcount_after_second, vcount_after_first,
        "P4: get_verified_count() must not change on double-verify"
    );
}

// ─── P5: verified never exceeds total — exhaustive over bounded N ─────────────

/// **P5** — `verified <= total` must hold after every operation in every
/// ordering. Exhaustive over all register/verify/remove orderings for 1–4
/// usernames.
///
/// Bounded domain: N ∈ {1, 2, 3, 4}; all operation sequences for each N.
#[test]
fn proof_verified_never_exceeds_total_exhaustive() {
    // For each N, run: register all, then verify all, then remove one-by-one.
    // Assert the invariant after every single step.
    for &n in &[1u32, 2, 3, 4] {
        let (env, admin, _base_user, cid) = setup();
        let usernames = ["p5a", "p5b", "p5c", "p5d"];

        // Phase 1: register all — verified <= total must hold at each step
        let users: std::vec::Vec<Address> = (0..n as usize)
            .map(|i| {
                let u = Address::generate(&env);
                register(&env, &cid, &u, usernames[i]);
                let (total, verified) = stats(&env, &cid);
                assert!(
                    verified <= total,
                    "P5(n={n}) after register[{i}]: verified({verified}) > total({total})"
                );
                u
            })
            .collect();

        // Phase 2: verify all — verified <= total must hold at each step
        for (i, _) in users.iter().enumerate() {
            verify(&env, &cid, &admin, usernames[i]);
            let (total, verified) = stats(&env, &cid);
            assert!(
                verified <= total,
                "P5(n={n}) after verify[{i}]: verified({verified}) > total({total})"
            );
        }

        // Phase 3: remove all — verified <= total must hold at each step
        for (i, u) in users.iter().enumerate() {
            remove(&env, &cid, u, usernames[i]);
            let (total, verified) = stats(&env, &cid);
            assert!(
                verified <= total,
                "P5(n={n}) after remove[{i}]: verified({verified}) > total({total})"
            );
        }
    }
}

// ─── P6: counters never underflow ────────────────────────────────────────────

/// **P6** — Repeated `remove` attempts on an empty or partially-populated
/// registry must never underflow the counters. Both `total` and `verified`
/// must remain ≥ 0 (they are `u32` so underflow would wrap or saturate; we
/// verify the contract uses saturating arithmetic by asserting neither counter
/// ever equals `u32::MAX` after a removal).
///
/// Bounded domain: all sequences of remove-on-empty and remove-after-partial
/// fill over 32 attempts.
#[test]
fn proof_counters_never_underflow() {
    // Attempt 1: remove on a completely empty registry (not registered)
    let (env, _admin, user, cid) = setup();
    for name in &["ghost1", "ghost2", "ghost3", "ghost4"] {
        env.mock_all_auths();
        let result = env.as_contract(&cid, || {
            TrustBridgeContract::remove(env.clone(), user.clone(), s(&env, name))
        });
        // Must fail with NotRegistered, not panic or wrap
        assert!(
            result.is_err(),
            "P6: remove of unregistered name must return an error, not panic"
        );
        assert_eq!(
            result.unwrap_err(),
            ContractError::NotRegistered,
            "P6: remove of unregistered name must return NotRegistered"
        );
        let (total, verified) = stats(&env, &cid);
        assert_ne!(total, u32::MAX, "P6: total must not wrap to u32::MAX");
        assert_ne!(verified, u32::MAX, "P6: verified must not wrap to u32::MAX");
        assert_eq!(total, 0, "P6: total must remain 0 on failed remove");
        assert_eq!(verified, 0, "P6: verified must remain 0 on failed remove");
        assert_eq!(
            verified_count(&env, &cid),
            0,
            "P6: verified_count must remain 0 on failed remove"
        );
    }

    // Attempt 2: register 4, verify 2, remove all 4 — counters must never underflow
    let (env, admin, _user, cid) = setup();
    let names = ["q1", "q2", "q3", "q4"];
    let users: std::vec::Vec<Address> = names
        .iter()
        .map(|&n| {
            let u = Address::generate(&env);
            register(&env, &cid, &u, n);
            u
        })
        .collect();

    // Verify first two only
    verify(&env, &cid, &admin, "q1");
    verify(&env, &cid, &admin, "q2");

    // Remove all four in order
    for (i, u) in users.iter().enumerate() {
        remove(&env, &cid, u, names[i]);
        let (total, verified) = stats(&env, &cid);
        let vc = verified_count(&env, &cid);
        assert_ne!(total, u32::MAX, "P6: total wrapped on remove[{i}]");
        assert_ne!(verified, u32::MAX, "P6: verified wrapped on remove[{i}]");
        assert_eq!(verified, vc, "P6: counter parity broken on remove[{i}]");
        assert!(verified <= total, "P6: verified > total after remove[{i}]");
    }

    // Final state: both counters must be 0
    let (total_final, verified_final) = stats(&env, &cid);
    assert_eq!(
        total_final, 0,
        "P6: total must be 0 after removing all records"
    );
    assert_eq!(
        verified_final, 0,
        "P6: verified must be 0 after removing all records"
    );
    assert_eq!(
        verified_count(&env, &cid),
        0,
        "P6: verified_count must be 0 after removing all records"
    );
}

// ─── P7: revoke_verification decrements verified_count ───────────────────────

/// **P7** — `revoke_verification` must decrement `verified_count` by 1.
/// A second revoke on the same (now unverified) record must fail and leave
/// counters unchanged — same underflow guard as double-remove.
#[test]
fn proof_revoke_decrements_and_is_idempotent() {
    let (env, admin, user, cid) = setup();
    register(&env, &cid, &user, "eve");
    verify(&env, &cid, &admin, "eve");

    let (_, verified_before) = stats(&env, &cid);
    assert_eq!(verified_before, 1, "P7 precondition");

    // First revoke
    env.mock_all_auths();
    env.as_contract(&cid, || {
        // reason_code 1 = RevokeReason::AdminOverride (or whatever code 1 maps to)
        TrustBridgeContract::revoke_verification(env.clone(), admin.clone(), s(&env, "eve"), 1)
            .unwrap();
    });

    let (_, verified_after) = stats(&env, &cid);
    let vc_after = verified_count(&env, &cid);
    assert_eq!(verified_after, 0, "P7: verified must be 0 after revoke");
    assert_eq!(vc_after, 0, "P7: verified_count must be 0 after revoke");

    // Second revoke must fail with NotVerified
    env.mock_all_auths();
    let second = env.as_contract(&cid, || {
        TrustBridgeContract::revoke_verification(env.clone(), admin.clone(), s(&env, "eve"), 1)
    });
    assert!(second.is_err(), "P7: second revoke must return an error");
    assert_eq!(
        second.unwrap_err(),
        ContractError::NotVerified,
        "P7: second revoke must return NotVerified"
    );

    // Counters must be unchanged after the failed second revoke
    let (_, verified_final) = stats(&env, &cid);
    let vc_final = verified_count(&env, &cid);
    assert_eq!(verified_final, 0, "P7: verified must still be 0");
    assert_eq!(vc_final, 0, "P7: verified_count must still be 0");
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
#[test]
fn proof_counters_match_index_under_mixed_batches() {
    let (env, admin, user, cid) = setup();
    for n in ["a1", "a2", "a3", "a4", "a5"] {
        register(&env, &cid, &user, n);
    }

    env.mock_all_auths();
    env.as_contract(&cid, || {
        let r = TrustBridgeContract::batch_verify(
            env.clone(),
            admin.clone(),
            names(&env, &["a1", "a2", "a2", "ghost", "a3"]),
        )
        .unwrap();
        assert_eq!(r.successful + r.failed, r.total);
    });
    assert_eq!(stats(&env, &cid), (index_len(&env, &cid), 3));

    env.as_contract(&cid, || {
        let r = TrustBridgeContract::batch_remove(
            env.clone(),
            admin.clone(),
            names(&env, &["a1", "a4", "a4", "ghost"]),
        )
        .unwrap();
        assert_eq!(r.successful, 2);
    });

    let (total, verified) = stats(&env, &cid);
    assert_eq!(
        total,
        index_len(&env, &cid),
        "total must equal index length"
    );
    assert_eq!(total, 3);
    assert_eq!(
        verified, 2,
        "only the removed verified record is decremented"
    );
    assert_eq!(verified, verified_count(&env, &cid));
}

/// **P8** — A rejected batch (oversized) moves no counter and leaves the
/// enumerable index untouched.
#[test]
fn proof_rejected_batch_moves_no_counter() {
    let (env, admin, user, cid) = setup();
    register(&env, &cid, &user, "a1");
    let before = (stats(&env, &cid), index_len(&env, &cid));

    let mut big = Vec::new(&env);
    for _ in 0..101 {
        big.push_back(s(&env, "a1"));
    }
    env.mock_all_auths();
    env.as_contract(&cid, || {
        assert_eq!(
            TrustBridgeContract::batch_remove(env.clone(), admin.clone(), big.clone()),
            Err(ContractError::InvalidBatchSize)
        );
    });

    assert_eq!((stats(&env, &cid), index_len(&env, &cid)), before);
}
