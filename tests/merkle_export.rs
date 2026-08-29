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

use soroban_sdk::{testutils::Address as _, Address, Bytes, BytesN, Env, String};
use soroban_sdk::Vec as SVec;

use trustbridge_contract::TrustBridgeContract;

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

/// One step of an inclusion proof: the sibling hash to combine with and
/// which side it sits on, or `Promoted` for a level where the current node
/// had no sibling and was carried up unchanged.
#[derive(Clone)]
enum Step {
    Pair { sibling: BytesN<32>, sibling_is_left: bool },
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
            Step::Pair { sibling, sibling_is_left } => {
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

        let page = TrustBridgeContract::get_registered_paginated(env.clone(), 0, 10).unwrap();
        assert_eq!(page.records.len(), 3);
        assert_ne!(page.merkle_root, BytesN::from_array(&env, &[0u8; 32]));
    });
}

#[test]
fn test_empty_registry_export_has_all_zero_root() {
    let (env, _admin, contract_id) = setup();
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        let page = TrustBridgeContract::get_registered_paginated(env.clone(), 0, 10).unwrap();
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
            TrustBridgeContract::register(env.clone(), s(&env, name), addr.clone(), SVec::new(&env))
                .unwrap();
        }
        TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, "carol")).unwrap();

        let page = TrustBridgeContract::get_registered_paginated(env.clone(), 0, 10).unwrap();
        assert_eq!(page.records.len(), 5);

        // Rebuild the leaf set exactly as documented, independent of the
        // contract's internal tree code.
        let mut leaves: SVec<BytesN<32>> = SVec::new(&env);
        for i in 0..page.records.len() {
            let (username, record) = page.records.get(i).unwrap();
            leaves.push_back(leaf_hash(&env, &username, &record.stellar_address, record.verified));
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

        let page = TrustBridgeContract::get_registered_paginated(env.clone(), 0, 10).unwrap();
        assert_eq!(page.records.len(), 1);
        let expected = leaf_hash(&env, &s(&env, "octocat"), &addr, false);
        assert_eq!(page.merkle_root, expected);
    });
}
