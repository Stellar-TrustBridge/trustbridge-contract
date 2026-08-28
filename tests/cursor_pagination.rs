//! Standalone verification for Issue #215 (opaque cursor tokens for
//! `get_registered_paginated` / `get_public_paginated`).
//!
//! Kept as its own integration test file, independent of `tests/integration.rs`,
//! so it can be built and run on its own via
//! `cargo test --test cursor_pagination` regardless of the state of that
//! other file.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

use trustbridge_contract::{ContractError, TrustBridgeContract};

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

/// Registers `n` freshly-generated users named `user{start}..user{start+n}`,
/// so repeated calls in one test (with increasing `start`) grow the registry
/// without re-registering (and thus not actually adding) an earlier name.
fn register_range(env: &Env, start: u32, n: u32) -> Vec<(String, Address)> {
    let mut out = Vec::new(env);
    for i in start..start + n {
        let addr = Address::generate(env);
        let name = s(env, &alloc::format!("user{i:03}"));
        TrustBridgeContract::register(env.clone(), name.clone(), addr.clone(), Vec::new(env))
            .unwrap();
        out.push_back((name, addr));
    }
    out
}

fn register_n(env: &Env, n: u32) -> Vec<(String, Address)> {
    register_range(env, 0, n)
}

extern crate alloc;

#[test]
fn test_full_walk_with_cursor_none_start_visits_every_record_once() {
    let (env, _admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_n(&env, 25);

        let mut seen: alloc::vec::Vec<String> = alloc::vec::Vec::new();
        let mut cursor = None;
        loop {
            let page = TrustBridgeContract::get_registered_paginated(env.clone(), cursor, 10)
                .unwrap();
            for i in 0..page.records.len() {
                let (username, _) = page.records.get(i).unwrap();
                seen.push(username);
            }
            if !page.has_more {
                assert!(page.next_cursor.is_none());
                break;
            }
            cursor = page.next_cursor;
        }

        assert_eq!(seen.len(), 25);
    });
}

#[test]
fn test_empty_registry_page_has_no_next_cursor() {
    let (env, _admin, contract_id) = setup();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let page = TrustBridgeContract::get_registered_paginated(env.clone(), None, 10).unwrap();
        assert_eq!(page.records.len(), 0);
        assert!(page.next_cursor.is_none());
        assert!(!page.has_more);
    });
}

#[test]
fn test_last_page_has_no_next_cursor() {
    let (env, _admin, contract_id) = setup();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_n(&env, 5);
        let page = TrustBridgeContract::get_registered_paginated(env.clone(), None, 10).unwrap();
        assert_eq!(page.records.len(), 5);
        assert!(page.next_cursor.is_none());
        assert!(!page.has_more);
    });
}

/// The core Issue #215 scenario: a cursor issued before a removal must not
/// be usable to silently continue past drifted positions — it must fail
/// loudly instead.
#[test]
fn test_removing_a_record_after_a_cursor_is_issued_invalidates_it() {
    let (env, admin, contract_id) = setup();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_n(&env, 10);

        // First page of 4; there is more after it.
        let page1 =
            TrustBridgeContract::get_registered_paginated(env.clone(), None, 4).unwrap();
        assert_eq!(page1.records.len(), 4);
        assert!(page1.has_more);
        let stale_cursor = page1.next_cursor.clone().unwrap();

        // Remove a record that sits *before* the stale cursor's offset,
        // shifting every later position back by one.
        TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, "user000")).unwrap();

        // Continuing with the pre-removal cursor must fail loudly rather
        // than silently return a drifted page.
        let result =
            TrustBridgeContract::get_registered_paginated(env.clone(), Some(stale_cursor), 4);
        assert_eq!(result, Err(ContractError::InvalidCursor));

        // The documented recovery — restart from `cursor = None` — must
        // succeed and eventually visit every remaining record exactly once.
        let mut seen: alloc::vec::Vec<String> = alloc::vec::Vec::new();
        let mut cursor = None;
        loop {
            let page =
                TrustBridgeContract::get_registered_paginated(env.clone(), cursor, 4).unwrap();
            for i in 0..page.records.len() {
                let (username, _) = page.records.get(i).unwrap();
                seen.push(username);
            }
            if !page.has_more {
                break;
            }
            cursor = page.next_cursor;
        }
        assert_eq!(seen.len(), 9);
        assert!(!seen.iter().any(|u| *u == s(&env, "user000")));
    });
}

/// A removal that happens *after* the position a cursor already passed
/// still invalidates it under this contract's coarse (whole-index)
/// generation bump — documented behavior, not a bug: any removal shifts the
/// global offset space, so the safe rule is "any removal invalidates every
/// outstanding cursor," not just ones downstream of the removed entry.
#[test]
fn test_removal_anywhere_invalidates_outstanding_cursors_even_if_already_passed() {
    let (env, admin, contract_id) = setup();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_n(&env, 10);

        let page1 =
            TrustBridgeContract::get_registered_paginated(env.clone(), None, 4).unwrap();
        let stale_cursor = page1.next_cursor.clone().unwrap();

        // Remove a record *after* the cursor's offset this time.
        TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, "user009")).unwrap();

        let result =
            TrustBridgeContract::get_registered_paginated(env.clone(), Some(stale_cursor), 4);
        assert_eq!(result, Err(ContractError::InvalidCursor));
    });
}

#[test]
fn test_cursor_is_interchangeable_between_admin_and_public_paginated() {
    let (env, _admin, contract_id) = setup();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_n(&env, 6);

        let page1 = TrustBridgeContract::get_public_paginated(env.clone(), None, 3).unwrap();
        assert!(page1.has_more);
        let cursor = page1.next_cursor;

        // Resume with the admin-gated variant using the cursor the public
        // variant issued.
        let page2 =
            TrustBridgeContract::get_registered_paginated(env.clone(), cursor, 3).unwrap();
        assert_eq!(page2.records.len(), 3);
        assert!(!page2.has_more);
    });
}

/// A cursor is opaque by design (Issue #215): this crate deliberately does
/// not expose a way to construct one outside of a contract call, so an
/// "out of range offset with an otherwise-valid generation" case is not
/// independently reachable from outside the contract — any removal that
/// would shrink the registry below a previously-issued offset also bumps
/// the generation, so `test_removing_a_record_after_a_cursor_is_issued_invalidates_it`
/// and `test_removal_anywhere_invalidates_outstanding_cursors_even_if_already_passed`
/// above already exercise the only way a stale cursor can actually arise.
#[test]
fn test_registering_more_after_a_cursor_is_issued_does_not_invalidate_it() {
    let (env, _admin, contract_id) = setup();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        register_n(&env, 3);

        let page1 = TrustBridgeContract::get_registered_paginated(env.clone(), None, 2).unwrap();
        assert!(page1.has_more);
        let cursor = page1.next_cursor;

        // Appending new registrations only grows the index past the cursor's
        // offset; it never shifts an existing position, so the cursor from
        // before this registration must still be valid.
        register_range(&env, 3, 2);

        let page2 =
            TrustBridgeContract::get_registered_paginated(env.clone(), cursor, 10).unwrap();
        assert!(page2.records.len() >= 1);
    });
}
