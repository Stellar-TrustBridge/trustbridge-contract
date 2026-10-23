//! Standalone verification for Issue #216 (Merkle root over an export page).
//!
//! Kept as its own integration test file, independent of `tests/integration.rs`,
//! so it can be built and run on its own via `cargo test --test merkle_export`
//! regardless of the state of that other file.
//!
//! The proof builder/verifier below is a deliberately independent
//! reimplementation of the tree rules documented in `src/merkle.rs` and
//! `docs/ABI.md` — it does not call into the contract's internal tree code —
//! because the point of this test is to prove that off-chain tooling, given
//! only the exported page and the documented spec, can build proofs that
//! verify against the on-chain root.

#![cfg(test)]

use soroban_sdk::Vec as SVec;
use soroban_sdk::{testutils::Address as _, Address, Bytes, BytesN, Env, String};

use trustbridge_contract::{ContractError, ContributorRecord, TrustBridgeContract};

const GOLDEN_ADMIN: &str = "GAAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQDZ7H";
const GOLDEN_ALICE: &str = "GABAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEJXA";
const GOLDEN_BOB: &str = "GA7QYNF7SOWQ3GLR2BGMZEHXAVIRZA4KVWLTJJFC7MGXUA74P7UJVSGZ";
const GOLDEN_CAROL: &str = "GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFSHONUCEOASW7QC7OX2H";

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

fn hash_from_hex(env: &Env, hex: &str) -> BytesN<32> {
    let bytes = std::array::from_fn(|index| {
        u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).unwrap()
    });
    BytesN::from_array(env, &bytes)
}

// ── Independent off-chain-style Merkle implementation, per docs/ABI.md ─────
//
// This does not call `trustbridge_contract`'s internal tree code — it is a
// from-scratch reimplementation of the documented spec, exactly like an
// off-chain treasury or dashboard would write.

const LEAF_DOMAIN: &[u8] = b"trustbridge/export-leaf/v1:";
const NODE_DOMAIN: &[u8] = b"trustbridge/export-node/v1:";

fn leaf_hash(env: &Env, username: &String, addr: &Address, verified: bool) -> BytesN<32> {
    let mut buf = Bytes::from_slice(env, LEAF_DOMAIN);
    buf.append(&username.to_bytes());
    buf.push_back(0u8);
    buf.append(&addr.to_string().to_bytes());
    buf.push_back(u8::from(verified));
    env.crypto().sha256(&buf).to_bytes()
}

fn node_hash(env: &Env, left: &BytesN<32>, right: &BytesN<32>) -> BytesN<32> {
    let mut buf = Bytes::from_slice(env, NODE_DOMAIN);
    buf.append(&Bytes::from(left.clone()));
    buf.append(&Bytes::from(right.clone()));
    env.crypto().sha256(&buf).to_bytes()
}

/// Independent tree root reconstruction over leaves, per docs/ABI.md.
fn independent_root_of(env: &Env, leaves: &SVec<BytesN<32>>) -> BytesN<32> {
    if leaves.is_empty() {
        return BytesN::from_array(env, &[0u8; 32]);
    }

    let mut level: SVec<BytesN<32>> = leaves.clone();
    while level.len() > 1 {
        let mut next: SVec<BytesN<32>> = SVec::new(env);
        let mut i: u32 = 0;
        while i + 1 < level.len() {
            let left = level.get(i).unwrap();
            let right = level.get(i + 1).unwrap();
            next.push_back(node_hash(env, &left, &right));
            i += 2;
        }
        if i < level.len() {
            next.push_back(level.get(i).unwrap());
        }
        level = next;
    }

    level.get(0).unwrap()
}

/// Computes the independent Merkle root over a sequence of exported records.
fn independent_root_of_records(
    env: &Env,
    records: &SVec<(String, ContributorRecord)>,
) -> BytesN<32> {
    let mut leaves: SVec<BytesN<32>> = SVec::new(env);
    for i in 0..records.len() {
        let (username, record) = records.get(i).unwrap();
        leaves.push_back(leaf_hash(
            env,
            &username,
            &record.stellar_address,
            record.verified,
        ));
    }
    independent_root_of(env, &leaves)
}

/// One step of an inclusion proof: the sibling hash to combine with and
/// which side it sits on, or `Promoted` for a level where the current node
/// had no sibling and was carried up unchanged.
#[derive(Clone)]
enum Step {
    Pair {
        sibling: BytesN<32>,
        sibling_is_left: bool,
    },
    Promoted,
}

/// Builds an inclusion proof for `leaves[index]` by replaying the same
/// level-by-level tree construction `src/merkle.rs` documents, recording the
/// sibling at each level instead of discarding it.
fn build_proof(env: &Env, leaves: &SVec<BytesN<32>>, index: u32) -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();
    let mut level: SVec<BytesN<32>> = leaves.clone();
    let mut idx = index;

    while level.len() > 1 {
        let mut next: SVec<BytesN<32>> = SVec::new(env);
        let mut i: u32 = 0;
        while i + 1 < level.len() {
            let left = level.get(i).unwrap();
            let right = level.get(i + 1).unwrap();
            if idx == i {
                steps.push(Step::Pair {
                    sibling: right.clone(),
                    sibling_is_left: false,
                });
                idx = next.len();
            } else if idx == i + 1 {
                steps.push(Step::Pair {
                    sibling: left.clone(),
                    sibling_is_left: true,
                });
                idx = next.len();
            }
            next.push_back(node_hash(env, &left, &right));
            i += 2;
        }
        if i < level.len() {
            if idx == i {
                steps.push(Step::Promoted);
                idx = next.len();
            }
            next.push_back(level.get(i).unwrap());
        }
        level = next;
    }

    steps
}

/// Recomputes the root from `leaf` and `steps`, exactly as a verifier with no
/// access to the rest of the tree would.
fn verify_proof(env: &Env, leaf: &BytesN<32>, steps: &[Step], root: &BytesN<32>) -> bool {
    let mut acc = leaf.clone();
    for step in steps {
        acc = match step {
            Step::Pair {
                sibling,
                sibling_is_left,
            } => {
                if *sibling_is_left {
                    node_hash(env, sibling, &acc)
                } else {
                    node_hash(env, &acc, sibling)
                }
            }
            Step::Promoted => acc,
        };
    }
    acc == *root
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn test_merkle_root_present_on_export_page() {
    let (env, admin, contract_id) = setup();
    let a1 = Address::generate(&env);
    let a2 = Address::generate(&env);
    let a3 = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        for (name, addr) in [("alice", &a1), ("bob", &a2), ("carol", &a3)] {
            TrustBridgeContract::register(
                env.clone(),
                s(&env, name),
                addr.clone(),
                SVec::new(&env),
            )
            .unwrap();
        }
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, "alice")).unwrap();

        let page = TrustBridgeContract::get_registered_paginated(env.clone(), None, 10).unwrap();
        assert_eq!(page.records.len(), 3);
        assert_ne!(page.merkle_root, BytesN::from_array(&env, &[0u8; 32]));
    });
}

#[test]
fn test_empty_registry_export_has_all_zero_root() {
    let (env, _admin, contract_id) = setup();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let page = TrustBridgeContract::get_registered_paginated(env.clone(), None, 10).unwrap();
        assert_eq!(page.records.len(), 0);
        assert_eq!(page.merkle_root, BytesN::from_array(&env, &[0u8; 32]));
    });
}

#[test]
fn test_leaf_hash_matches_contract_helper() {
    let (env, _admin, contract_id) = setup();
    let addr = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "octocat"),
            addr.clone(),
            SVec::new(&env),
        )
        .unwrap();

        let via_contract = TrustBridgeContract::merkle_leaf_hash(
            env.clone(),
            s(&env, "octocat"),
            addr.clone(),
            false,
        );
        let via_reimpl = leaf_hash(&env, &s(&env, "octocat"), &addr, false);
        assert_eq!(via_contract, via_reimpl);
    });
}

#[test]
fn test_included_member_proves_inclusion_and_non_member_fails() {
    let (env, admin, contract_id) = setup();
    let names = ["alice", "bob", "carol", "dave", "erin"];
    let addrs: Vec<Address> = names.iter().map(|_| Address::generate(&env)).collect();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        for (name, addr) in names.iter().zip(addrs.iter()) {
            TrustBridgeContract::register(
                env.clone(),
                s(&env, name),
                addr.clone(),
                SVec::new(&env),
            )
            .unwrap();
        }
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, "carol")).unwrap();

        let page = TrustBridgeContract::get_registered_paginated(env.clone(), None, 10).unwrap();
        assert_eq!(page.records.len(), 5);

        // Rebuild the leaf set exactly as documented, independent of the
        // contract's internal tree code.
        let mut leaves: SVec<BytesN<32>> = SVec::new(&env);
        for i in 0..page.records.len() {
            let (username, record) = page.records.get(i).unwrap();
            leaves.push_back(leaf_hash(
                &env,
                &username,
                &record.stellar_address,
                record.verified,
            ));
        }

        // "carol" (index 2) is verified; her leaf must reflect that and her
        // proof must verify against the page's published root.
        let carol_index = 2u32;
        let (carol_username, carol_record) = page.records.get(carol_index).unwrap();
        assert_eq!(carol_username, s(&env, "carol"));
        assert!(carol_record.verified);

        let carol_leaf = leaves.get(carol_index).unwrap();
        let proof = build_proof(&env, &leaves, carol_index);
        assert!(
            verify_proof(&env, &carol_leaf, &proof, &page.merkle_root),
            "carol's proof must verify against the page's merkle_root"
        );

        // A non-member (never registered) must not verify against this root
        // using carol's proof path — there is no leaf for them in this tree.
        let outsider = Address::generate(&env);
        let outsider_leaf = leaf_hash(&env, &s(&env, "mallory"), &outsider, false);
        assert!(
            !verify_proof(&env, &outsider_leaf, &proof, &page.merkle_root),
            "a non-member must not verify against the page's merkle_root"
        );

        // Tampering with carol's verified flag must also invalidate the leaf
        // against the same proof path, since the flag is part of the leaf.
        let tampered_leaf = leaf_hash(&env, &carol_username, &carol_record.stellar_address, false);
        assert!(
            !verify_proof(&env, &tampered_leaf, &proof, &page.merkle_root),
            "a tampered verified flag must change the leaf and break the proof"
        );
    });
}

#[test]
fn test_single_record_page_root_equals_its_own_leaf() {
    let (env, _admin, contract_id) = setup();
    let addr = Address::generate(&env);

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "octocat"),
            addr.clone(),
            SVec::new(&env),
        )
        .unwrap();

        let page = TrustBridgeContract::get_registered_paginated(env.clone(), None, 10).unwrap();
        assert_eq!(page.records.len(), 1);
        let expected = leaf_hash(&env, &s(&env, "octocat"), &addr, false);
        assert_eq!(page.merkle_root, expected);
    });
}

#[test]
fn test_v1_export_merkle_golden_vectors() {
    let env = Env::default();
    let admin = Address::from_str(&env, GOLDEN_ADMIN);
    let alice = Address::from_str(&env, GOLDEN_ALICE);
    let bob = Address::from_str(&env, GOLDEN_BOB);
    let carol = Address::from_str(&env, GOLDEN_CAROL);
    let contract_id = env.register(TrustBridgeContract, ());

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
        for (name, address) in [("Alice", &alice), ("bob", &bob), ("carol", &carol)] {
            TrustBridgeContract::register(
                env.clone(),
                s(&env, name),
                address.clone(),
                SVec::new(&env),
            )
            .unwrap();
        }
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, "alice")).unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, "carol")).unwrap();
    });

    let expected_alice = hash_from_hex(
        &env,
        "03f5777f71952d65b384a30ee2a389996eec0c5411882fab4b825dbd45ba34f1",
    );
    let expected_bob = hash_from_hex(
        &env,
        "965304356c6c946637f68203856da9f67804028a3bc009e5fb82d6d66ff3554c",
    );
    let expected_carol = hash_from_hex(
        &env,
        "f36b3fabc5f6ded2df39a6ff6def7e7e751e62bc2754467ccadd214bdc2b3a31",
    );

    assert_eq!(
        TrustBridgeContract::merkle_leaf_hash(
            env.clone(),
            s(&env, "alice"),
            alice.clone(),
            true,
        ),
        expected_alice
    );
    assert_eq!(
        TrustBridgeContract::merkle_leaf_hash(env.clone(), s(&env, "bob"), bob.clone(), false,),
        expected_bob
    );
    assert_eq!(
        TrustBridgeContract::merkle_leaf_hash(
            env.clone(),
            s(&env, "carol"),
            carol.clone(),
            true,
        ),
        expected_carol
    );

    let single = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_registered_paginated(env.clone(), None, 1).unwrap()
    });
    assert_eq!(single.records.len(), 1);
    assert_eq!(single.merkle_root, expected_alice);

    let page = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_registered_paginated(env.clone(), None, 10).unwrap()
    });
    assert_eq!(page.records.len(), 3);
    assert_eq!(
        page.merkle_root,
        hash_from_hex(
            &env,
            "81074a96376e707644a2042a2fb47bceb61f9f572b42fa9f92ce2817e6f873b0",
        )
    );
}

// ── Property Tests: Cross-page Merkle root stability (Issue #619) ───────────

/// Issue #619: Merkle roots across export pages must be stable for a fixed
/// registry snapshot regardless of page offset and limit parameters.
///
/// Property-style verification that walks an entire registry snapshot using
/// multiple distinct page limits (e.g., 1, 2, 3, 4, 5, 7, 8, 10, 11, 15, 20)
/// and asserts:
/// 1. Every pagination walk reconstructs the exact same underlying record set.
/// 2. The independently computed Merkle root of the full export is identical
///    across all page limits and paging splits.
/// 3. For every page in every walk, the contract's published `page.merkle_root`
///    matches the independent reimplementation for that page's records.
/// 4. Proofs generated for any record verify against the page's `merkle_root`.
#[test]
fn test_merkle_root_stability_across_shuffled_page_limits_and_offsets() {
    let (env, admin, contract_id) = setup();
    let usernames = [
        "alice", "bob", "carol", "david", "eve", "frank", "grace", "heidi", "ivan", "judy",
        "mallory",
    ];
    let addresses: Vec<Address> = usernames.iter().map(|_| Address::generate(&env)).collect();

    env.mock_all_auths();
    for (name, addr) in usernames.iter().zip(addresses.iter()) {
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                s(&env, name),
                addr.clone(),
                SVec::new(&env),
            )
            .unwrap();
        });
    }

    // Verify some users; leave others unverified (their addresses get redacted to zero address).
    for verified_name in ["alice", "carol", "eve", "heidi", "judy"] {
        env.as_contract(&contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, verified_name)).unwrap();
        });
    }

    // Baseline reference: export in one large page.
    let baseline_page = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), None, 50).unwrap()
    });
    assert_eq!(baseline_page.records.len(), 11);
    assert_eq!(baseline_page.total, 11);
    assert!(!baseline_page.has_more);
    let canonical_snapshot_root = baseline_page.merkle_root.clone();
    assert_ne!(canonical_snapshot_root, BytesN::from_array(&env, &[0u8; 32]));

    // Check independent root of baseline records matches contract merkle_root
    assert_eq!(
        independent_root_of_records(&env, &baseline_page.records),
        canonical_snapshot_root
    );

    // Shuffle various page limits (including prime, even, odd, small, large).
    let limits_to_test = [1u32, 2, 3, 4, 5, 7, 8, 10, 11, 15, 20];

    for &limit in &limits_to_test {
        let mut cursor = None;
        let mut walk_records: SVec<(String, ContributorRecord)> = SVec::new(&env);
        let mut page_count = 0;

        loop {
            let page = env.as_contract(&contract_id, || {
                TrustBridgeContract::get_public_paginated(env.clone(), cursor.clone(), limit).unwrap()
            });
            page_count += 1;

            // Invariant: every page's merkle_root must match independent recomputation
            let expected_page_root = independent_root_of_records(&env, &page.records);
            assert_eq!(
                page.merkle_root, expected_page_root,
                "page merkle_root must match independent leaf/tree construction for limit {limit}"
            );

            // Invariant: inclusion proofs for all records on this page verify against page.merkle_root
            let mut page_leaves: SVec<BytesN<32>> = SVec::new(&env);
            for i in 0..page.records.len() {
                let (u, r) = page.records.get(i).unwrap();
                page_leaves.push_back(leaf_hash(&env, &u, &r.stellar_address, r.verified));
            }
            for i in 0..page.records.len() {
                let proof = build_proof(&env, &page_leaves, i);
                let leaf = page_leaves.get(i).unwrap();
                assert!(
                    verify_proof(&env, &leaf, &proof, &page.merkle_root),
                    "proof for leaf {i} must verify against page.merkle_root"
                );
            }

            for i in 0..page.records.len() {
                walk_records.push_back(page.records.get(i).unwrap());
            }

            if let Some(next) = page.next_cursor {
                assert!(page.has_more);
                cursor = Some(next);
            } else {
                assert!(!page.has_more);
                break;
            }
        }

        assert!(page_count >= 1, "must have fetched at least one page");

        // Entire record set must match baseline
        assert_eq!(
            walk_records.len(),
            11,
            "walk with limit {limit} must visit all 11 records"
        );
        for i in 0..11 {
            let (walk_user, walk_rec) = walk_records.get(i).unwrap();
            let (base_user, base_rec) = baseline_page.records.get(i).unwrap();
            assert_eq!(walk_user, base_user);
            assert_eq!(walk_rec.stellar_address, base_rec.stellar_address);
            assert_eq!(walk_rec.verified, base_rec.verified);
            assert_eq!(walk_rec.is_bot, base_rec.is_bot);
        }

        // Full export Merkle root must be IDENTICAL across all page limits
        let walk_root = independent_root_of_records(&env, &walk_records);
        assert_eq!(
            walk_root, canonical_snapshot_root,
            "Merkle root of full export must be identical regardless of page limit {limit}"
        );
    }
}

/// Issue #619: Merkle roots must be identical for the same underlying record
/// set, even when queried with different page limits or across page boundaries.
#[test]
fn test_page_merkle_root_identical_for_same_underlying_record_set() {
    let (env, admin, contract_id) = setup();
    let names = ["alice", "bob", "carol", "dave", "erin"];
    let addrs: Vec<Address> = names.iter().map(|_| Address::generate(&env)).collect();

    env.mock_all_auths();
    for (name, addr) in names.iter().zip(addrs.iter()) {
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                s(&env, name),
                addr.clone(),
                SVec::new(&env),
            )
            .unwrap();
        });
    }

    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, "alice")).unwrap();
    });

    // Query offset 0 with limit 5, limit 10, limit 25 -> all cover records 0..5 (all 5 records).
    let page_5 = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), None, 5).unwrap()
    });
    let page_10 = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), None, 10).unwrap()
    });
    let page_25 = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), None, 25).unwrap()
    });

    assert_eq!(page_5.records.len(), 5);
    assert_eq!(page_10.records.len(), 5);
    assert_eq!(page_25.records.len(), 5);

    // Assert merkle_root equality across calls with varying limit for the same record set
    assert_eq!(page_5.merkle_root, page_10.merkle_root);
    assert_eq!(page_10.merkle_root, page_25.merkle_root);

    // Query tail page: cursor after first 3 records (at offset 3, 2 records remain: dave, erin).
    let first_3 = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), None, 3).unwrap()
    });
    let tail_cursor = first_3.next_cursor.expect("should have next cursor");

    // Both limit 2 and limit 10 from tail_cursor return records [dave, erin]
    let tail_lim2 = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), Some(tail_cursor.clone()), 2).unwrap()
    });
    let tail_lim10 = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), Some(tail_cursor), 10).unwrap()
    });

    assert_eq!(tail_lim2.records.len(), 2);
    assert_eq!(tail_lim10.records.len(), 2);
    assert_eq!(tail_lim2.merkle_root, tail_lim10.merkle_root);
    assert_eq!(
        tail_lim2.merkle_root,
        independent_root_of_records(&env, &tail_lim2.records)
    );
}

/// Issue #619: Edge cases — empty tail page and single-record registry.
#[test]
fn test_empty_tail_page_and_single_record_edge_cases() {
    let (env, _admin, contract_id) = setup();

    // 1. Empty registry: get_public_paginated must return empty records and all-zero root.
    let empty_page = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), None, 10).unwrap()
    });
    assert_eq!(empty_page.records.len(), 0);
    assert_eq!(empty_page.total, 0);
    assert!(!empty_page.has_more);
    assert_eq!(empty_page.next_cursor, None);
    assert_eq!(
        empty_page.merkle_root,
        BytesN::from_array(&env, &[0u8; 32])
    );
    assert_eq!(
        empty_page.merkle_root,
        independent_root_of_records(&env, &empty_page.records)
    );

    // 2. Single-record registry: test across various page limits.
    let addr = Address::generate(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "solo"),
            addr.clone(),
            SVec::new(&env),
        )
        .unwrap();
    });

    for limit in [1u32, 2, 5, 10] {
        let page = env.as_contract(&contract_id, || {
            TrustBridgeContract::get_public_paginated(env.clone(), None, limit).unwrap()
        });
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.total, 1);
        assert!(!page.has_more);
        assert_eq!(page.next_cursor, None);

        // For unverified user in get_public_paginated, address is redacted to zero address
        let zero_addr =
            Address::from_str(&env, "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF");
        let expected_leaf = leaf_hash(&env, &s(&env, "solo"), &zero_addr, false);
        assert_eq!(
            page.merkle_root, expected_leaf,
            "single record merkle_root must equal leaf hash across limits"
        );
        assert_eq!(
            page.merkle_root,
            independent_root_of_records(&env, &page.records)
        );

        let mut page_leaves = SVec::new(&env);
        page_leaves.push_back(expected_leaf.clone());
        let proof = build_proof(&env, &page_leaves, 0);
        assert!(verify_proof(&env, &expected_leaf, &proof, &page.merkle_root));
    }

    // 3. Multi-page pagination reaching empty tail page:
    // Register second record so 2 records exist.
    let addr2 = Address::generate(&env);
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "second"),
            addr2.clone(),
            SVec::new(&env),
        )
        .unwrap();
    });

    // Page 1 with limit 1 returns first record and next_cursor pointing to offset 1
    let p1 = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), None, 1).unwrap()
    });
    assert_eq!(p1.records.len(), 1);
    assert!(p1.has_more);
    let c1 = p1.next_cursor.expect("cursor must exist");

    // Page 2 with limit 1 returns second record and no next_cursor
    let p2 = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), Some(c1.clone()), 1).unwrap()
    });
    assert_eq!(p2.records.len(), 1);
    assert!(!p2.has_more);
    assert_eq!(p2.next_cursor, None);

    // A cursor at offset == total under the current generation returns an empty tail page.
    let mut cursor_bytes = [0u8; 8];
    cursor_bytes[0..4].copy_from_slice(&2u32.to_be_bytes()); // offset = 2 == total
    cursor_bytes[4..8].copy_from_slice(&0u32.to_be_bytes()); // initial generation = 0
    let tail_cursor = BytesN::from_array(&env, &cursor_bytes);
    let tail_page = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), Some(tail_cursor), 10).unwrap()
    });
    assert_eq!(tail_page.records.len(), 0);
    assert!(!tail_page.has_more);
    assert_eq!(tail_page.next_cursor, None);
    assert_eq!(tail_page.merkle_root, BytesN::from_array(&env, &[0u8; 32]));
}

/// Issue #619: Edge cases — record removal invalidates prior Merkle roots.
#[test]
fn test_record_removal_invalidates_prior_merkle_roots() {
    let (env, _admin, contract_id) = setup();
    let names = ["alice", "bob", "carol", "dave", "erin"];
    let addrs: Vec<Address> = names.iter().map(|_| Address::generate(&env)).collect();

    env.mock_all_auths();
    for (name, addr) in names.iter().zip(addrs.iter()) {
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                s(&env, name),
                addr.clone(),
                SVec::new(&env),
            )
            .unwrap();
        });
    }

    let before_page = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), None, 10).unwrap()
    });
    assert_eq!(before_page.records.len(), 5);
    let before_root = before_page.merkle_root;

    // Remove middle user "carol"
    env.as_contract(&contract_id, || {
        TrustBridgeContract::remove(env.clone(), addrs[2].clone(), s(&env, "carol")).unwrap();
    });

    let after_page = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), None, 10).unwrap()
    });
    assert_eq!(after_page.records.len(), 4);
    assert_ne!(
        after_page.merkle_root, before_root,
        "Record removal must invalidate prior Merkle root"
    );
    assert_eq!(
        after_page.merkle_root,
        independent_root_of_records(&env, &after_page.records)
    );

    // Paginating with smaller limit also produces the new valid root across the walk
    let p1 = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), None, 2).unwrap()
    });
    let p2 = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), p1.next_cursor, 10).unwrap()
    });
    let mut walk_recs = SVec::new(&env);
    for i in 0..p1.records.len() {
        walk_recs.push_back(p1.records.get(i).unwrap());
    }
    for i in 0..p2.records.len() {
        walk_recs.push_back(p2.records.get(i).unwrap());
    }
    assert_eq!(
        independent_root_of_records(&env, &walk_recs),
        after_page.merkle_root
    );
}

/// Issue #619: Edge cases — bot flag changes update records and maintain consistent leaves.
#[test]
fn test_bot_flag_changes_maintain_leaf_and_root_consistency() {
    let (env, admin, contract_id) = setup();
    let addr = Address::generate(&env);
    let username = s(&env, "botuser");

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username.clone(),
            addr.clone(),
            SVec::new(&env),
        )
        .unwrap();
    });

    // Mark as bot
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_bot_status(env.clone(), admin.clone(), username.clone(), true)
            .unwrap();
    });

    let page = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), None, 10).unwrap()
    });
    assert_eq!(page.records.len(), 1);
    let (name, record) = page.records.get(0).unwrap();
    assert_eq!(name, username);
    assert!(record.is_bot, "is_bot flag must be true on export page");

    // Leaf hash matches contract helper
    let via_contract = TrustBridgeContract::merkle_leaf_hash(
        env.clone(),
        name.clone(),
        record.stellar_address.clone(),
        record.verified,
    );
    let via_independent = leaf_hash(&env, &name, &record.stellar_address, record.verified);
    assert_eq!(via_contract, via_independent);
    assert_eq!(page.merkle_root, via_independent);
}

/// Issue #619: Edge cases — uninitialized contract returns error, not empty root.
#[test]
fn test_uninitialized_contract_returns_error_not_empty_root() {
    let env = Env::default();
    let contract_id = env.register(TrustBridgeContract, ());

    // Calling get_public_paginated before initialize must return NotInitialized
    let res = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), None, 10)
    });
    assert_eq!(
        res,
        Err(ContractError::NotInitialized),
        "uninitialized contract must return NotInitialized error, not an empty root"
    );
}

// ── Proof verification negative tests (Issue #667) ───────────────────────────

/// Registers `names` and returns the exported page's leaves and merkle root.
fn page_leaves(env: &Env, contract_id: &Address, names: &[&str]) -> (SVec<BytesN<32>>, BytesN<32>) {
    env.mock_all_auths();
    env.as_contract(contract_id, || {
        for name in names {
            TrustBridgeContract::register(
                env.clone(),
                s(env, name),
                Address::generate(env),
                SVec::new(env),
            )
            .unwrap();
        }
        let page = TrustBridgeContract::get_registered_paginated(env.clone(), None, 10).unwrap();
        let mut leaves: SVec<BytesN<32>> = SVec::new(env);
        for (username, record) in page.records.iter() {
            leaves.push_back(leaf_hash(
                env,
                &username,
                &record.stellar_address,
                record.verified,
            ));
        }
        (leaves, page.merkle_root)
    })
}

#[test]
fn test_truncated_proof_fails_verification() {
    let (env, _admin, contract_id) = setup();
    let (leaves, root) = page_leaves(
        &env,
        &contract_id,
        &["alice", "bob", "carol", "dave", "erin"],
    );

    let leaf = leaves.get(1).unwrap();
    let proof = build_proof(&env, &leaves, 1);
    assert!(
        verify_proof(&env, &leaf, &proof, &root),
        "full proof must verify"
    );

    for keep in 0..proof.len() {
        assert!(
            !verify_proof(&env, &leaf, &proof[..keep], &root),
            "proof truncated to {keep} step(s) must not verify"
        );
    }
}

#[test]
fn test_reordered_leaves_fail_verification() {
    let (env, _admin, contract_id) = setup();
    let (leaves, root) = page_leaves(&env, &contract_id, &["alice", "bob", "carol", "dave"]);

    let mut swapped = leaves.clone();
    swapped.set(0, leaves.get(1).unwrap());
    swapped.set(1, leaves.get(0).unwrap());

    // A proof built from the reordered leaf set must not verify against the
    // published root, even though it contains exactly the same leaves.
    let proof = build_proof(&env, &swapped, 0);
    assert!(!verify_proof(&env, &swapped.get(0).unwrap(), &proof, &root));

    // Flipping a step's side is the in-proof equivalent of reordering.
    let mut flipped = build_proof(&env, &leaves, 0);
    if let Step::Pair {
        sibling_is_left, ..
    } = &mut flipped[0]
    {
        *sibling_is_left = !*sibling_is_left;
    }
    assert!(!verify_proof(
        &env,
        &leaves.get(0).unwrap(),
        &flipped,
        &root
    ));
}

#[test]
fn test_proof_for_wrong_index_fails_verification() {
    let (env, _admin, contract_id) = setup();
    let (leaves, root) = page_leaves(&env, &contract_id, &["alice", "bob", "carol"]);

    let proof_for_bob = build_proof(&env, &leaves, 1);
    assert!(!verify_proof(
        &env,
        &leaves.get(0).unwrap(),
        &proof_for_bob,
        &root
    ));
}
