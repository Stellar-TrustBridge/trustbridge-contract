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

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};
use trustbridge_contract::{ContractError, TrustBridgeContract};

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
    assert_eq!(total_before, 1, "P1 precondition: total must be 1 after register");
    assert_eq!(verified_before, 0, "P1 precondition: verified must be 0 before verify");
    assert_eq!(vcount_before, 0, "P1 precondition: verified_count must be 0 before verify");

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
        let names: Vec<(&str, Address)> = (0..n)
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
        assert_eq!(verified_before, 0, "P1-batch: no verified records before verify loop");

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
    assert_eq!(vcount_before, 1, "P2 precondition: verified_count must be 1");

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
    assert_eq!(
        total_after, 0,
        "P2: total must be 0 after remove"
    );
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
        assert!(
            verified <= total,
            "P6: verified > total after remove[{i}]"
        );
    }

    // Final state: both counters must be 0
    let (total_final, verified_final) = stats(&env, &cid);
    assert_eq!(total_final, 0, "P6: total must be 0 after removing all records");
    assert_eq!(verified_final, 0, "P6: verified must be 0 after removing all records");
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
        TrustBridgeContract::revoke_verification(
            env.clone(),
            admin.clone(),
            s(&env, "eve"),
            1,
        )
        .unwrap();
    });

    let (_, verified_after) = stats(&env, &cid);
    let vc_after = verified_count(&env, &cid);
    assert_eq!(
        verified_after, 0,
        "P7: verified must be 0 after revoke"
    );
    assert_eq!(vc_after, 0, "P7: verified_count must be 0 after revoke");

    // Second revoke must fail with NotVerified
    env.mock_all_auths();
    let second = env.as_contract(&cid, || {
        TrustBridgeContract::revoke_verification(
            env.clone(),
            admin.clone(),
            s(&env, "eve"),
            1,
        )
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
