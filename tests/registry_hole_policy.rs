//! Property test for the flat/chunked username index membership invariant.
//!
//! The existing `compact_index` tests (`tests/integration.rs`) are all
//! scenario-based: they build one specific registry shape and assert one
//! specific outcome. None of them state the general "iff" property that
//! actually defines a correct index: a username belongs to the flat index
//! if and only if it also appears in exactly one chunk and has a live
//! record. This file checks that property directly against raw contract
//! storage — not through `get_all_registered`, which silently drops any
//! index entry whose record is missing and so could never itself catch a
//! membership bug — across register / remove / compact sequences. See
//! `docs/REGISTRY_INVARIANTS.md#hole-policy`.

#![cfg(test)]

use soroban_sdk::{symbol_short, testutils::Address as _, Address, Env, String, Vec};

use trustbridge_contract::TrustBridgeContract;

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });

    (env, admin, contract_id)
}

fn s(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

/// Reads the flat index and every chunk directly out of instance/persistent
/// storage and asserts the membership property in both directions:
/// - every flat-index entry has a live record and appears in the chunks;
/// - every chunked entry appears in the flat index.
///
/// This does not assert anything about *how full* individual chunks are —
/// per the documented hole policy, a chunk shrinking below `CHUNK_SIZE`
/// after a removal is allowed and is not itself a violation. Only
/// membership (not chunk density) is a correctness property here.
fn assert_index_membership_property(env: &Env, contract_id: &Address) {
    env.as_contract(contract_id, || {
        let index: Vec<String> = env
            .storage()
            .instance()
            .get(&symbol_short!("idx"))
            .unwrap_or_else(|| Vec::new(env));

        let chunk_count: u32 = env
            .storage()
            .instance()
            .get(&symbol_short!("chkcnt")) // CHUNK_CNT_KEY — persisted as chkcnt, not chunkcnt
            .unwrap_or(0);

        let mut chunked: Vec<String> = Vec::new(env);
        for c in 0..chunk_count {
            let chunk: Vec<String> = env
                .storage()
                .persistent()
                .get(&(symbol_short!("chunk"), c))
                .unwrap_or_else(|| Vec::new(env));
            for u in chunk.iter() {
                chunked.push_back(u);
            }
        }

        assert_eq!(
            index.len(),
            chunked.len(),
            "flat index and the union of all chunks must have the same length"
        );

        for username in index.iter() {
            assert!(
                TrustBridgeContract::has_record(env.clone(), username.clone()),
                "flat index contains {:?} but it has no backing record",
                username
            );
            assert!(
                chunked.contains(username.clone()),
                "flat index contains {:?} but it is missing from every chunk",
                username
            );
        }

        for username in chunked.iter() {
            assert!(
                index.contains(username.clone()),
                "a chunk contains {:?} but it is missing from the flat index",
                username
            );
            assert!(
                TrustBridgeContract::has_record(env.clone(), username.clone()),
                "a chunk contains {:?} but it has no backing record",
                username
            );
        }
    });
}

#[test]
fn test_index_membership_property_holds_across_register_remove_compact_sequence() {
    let (env, admin, contract_id) = setup();

    let names = ["alice", "bob", "carol", "dave", "eve"];
    let mut addrs = Vec::new(&env);
    for _ in 0..names.len() {
        addrs.push_back(Address::generate(&env));
    }

    // Register all five.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        for (i, name) in names.iter().enumerate() {
            TrustBridgeContract::register(
                env.clone(),
                s(&env, name),
                addrs.get(i as u32).unwrap(),
                Vec::new(&env),
            )
            .unwrap();
        }
    });
    assert_index_membership_property(&env, &contract_id);

    // Remove a middle entry — no compaction yet.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, "carol")).unwrap();
    });
    assert_index_membership_property(&env, &contract_id);

    // Remove the first entry too, then compact.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, "alice")).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::compact_index(env.clone()).unwrap();
    });
    assert_index_membership_property(&env, &contract_id);

    // Remove down to a single survivor, compacting again.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, "bob")).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, "dave")).unwrap();
    });
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::compact_index(env.clone()).unwrap();
    });
    assert_index_membership_property(&env, &contract_id);

    // Remove the last remaining entry — empty-registry edge case.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, "eve")).unwrap();
    });
    assert_index_membership_property(&env, &contract_id);

    // Re-register into the now-empty registry to confirm the property holds
    // after a full empty -> non-empty cycle too.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "frank"),
            Address::generate(&env),
            Vec::new(&env),
        )
        .unwrap();
    });
    assert_index_membership_property(&env, &contract_id);
}

#[test]
fn test_index_membership_property_holds_at_chunk_boundary() {
    let (env, admin, contract_id) = setup();

    // CHUNK_SIZE is 50 (docs/REGISTRY_INVARIANTS.md); register enough users
    // to span two chunks, then remove across the boundary and compact.
    let total = 60u32;
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        for i in 0..total {
            let name = format!("user{i}");
            TrustBridgeContract::register(
                env.clone(),
                s(&env, &name),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        }
    });
    assert_index_membership_property(&env, &contract_id);

    // Remove a run of usernames straddling the chunk boundary (indices
    // 45..=54), then compact.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        for i in 45..55 {
            let name = format!("user{i}");
            TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, &name)).unwrap();
        }
    });
    assert_index_membership_property(&env, &contract_id);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::compact_index(env.clone()).unwrap();
    });
    assert_index_membership_property(&env, &contract_id);
}

// ── batch_remove sparse gaps (Issue #673) ─────────────────────────────────

fn register_names(env: &Env, contract_id: &Address, names: &[&str]) {
    env.mock_all_auths();
    env.as_contract(contract_id, || {
        for name in names {
            TrustBridgeContract::register(
                env.clone(),
                s(env, name),
                Address::generate(env),
                Vec::new(env),
            )
            .unwrap();
        }
    });
}

/// Walks `get_public_paginated` to the end with a small page size.
fn walk_public(env: &Env, contract_id: &Address, limit: u32) -> Vec<String> {
    let mut out = Vec::new(env);
    env.as_contract(contract_id, || {
        let mut cursor = None;
        loop {
            let page = TrustBridgeContract::get_public_paginated(env.clone(), cursor, limit).unwrap();
            for (name, _) in page.records.iter() {
                out.push_back(name);
            }
            if !page.has_more {
                break;
            }
            cursor = page.next_cursor;
        }
    });
    out
}

/// Asserts the paginated walk yields each expected name exactly once.
fn assert_walk_is_exactly(env: &Env, contract_id: &Address, limit: u32, expected: &[&str]) {
    let seen = walk_public(env, contract_id, limit);
    assert_eq!(seen.len() as usize, expected.len(), "limit={limit} skipped or repeated a record");
    for name in expected {
        assert!(seen.contains(s(env, name)), "limit={limit} skipped {name}");
    }
}

/// Sparse, non-contiguous holes from one `batch_remove` (first, middle,
/// last) must keep the membership property and must not cause a paginated
/// walk to skip or repeat any survivor.
#[test]
fn test_batch_remove_sparse_gaps_keep_index_and_pagination_intact() {
    let (env, admin, contract_id) = setup();
    let names = ["u0", "u1", "u2", "u3", "u4", "u5", "u6", "u7"];
    register_names(&env, &contract_id, &names);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let mut batch = Vec::new(&env);
        for name in ["u0", "u3", "u4", "u7"] {
            batch.push_back(s(&env, name));
        }
        let summary = TrustBridgeContract::batch_remove(env.clone(), admin.clone(), batch).unwrap();
        assert_eq!(summary.successful, 4);
        assert_eq!(summary.failed, 0);
    });
    assert_index_membership_property(&env, &contract_id);

    for limit in [1u32, 2, 3] {
        assert_walk_is_exactly(&env, &contract_id, limit, &["u1", "u2", "u5", "u6"]);
    }
}

/// Failure paths: removing absent names leaves the index untouched and
/// counts as `failed`; an empty batch is rejected with a stable error and
/// writes nothing.
#[test]
fn test_batch_remove_failures_leave_no_holes() {
    let (env, admin, contract_id) = setup();
    register_names(&env, &contract_id, &["a0", "a1", "a2"]);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let mut batch = Vec::new(&env);
        batch.push_back(s(&env, "missing"));
        batch.push_back(s(&env, "a1"));
        let summary = TrustBridgeContract::batch_remove(env.clone(), admin.clone(), batch).unwrap();
        assert_eq!(summary.successful, 1);
        assert_eq!(summary.failed, 1);

        assert_eq!(
            TrustBridgeContract::batch_remove(env.clone(), admin.clone(), Vec::new(&env)).err(),
            Some(trustbridge_contract::ContractError::InvalidBatchSize)
        );
    });
    assert_index_membership_property(&env, &contract_id);
    assert_walk_is_exactly(&env, &contract_id, 1, &["a0", "a2"]);
}
