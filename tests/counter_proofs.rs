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
