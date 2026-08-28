//! Merkle root over an export page, for off-chain inclusion proofs (Issue #216).
//!
//! Treasuries and dashboards need to prove a specific `(username, record)`
//! pair was present in a given `get_registered_paginated` /
//! `get_public_paginated` page without republishing or re-fetching the whole
//! registry. This module defines the leaf encoding and the binary tree
//! construction behind `ExportPage::merkle_root`, so off-chain tooling can
//! independently rebuild the same tree — and a Merkle proof against it —
//! from the page's own `records`, in any language, without depending on this
//! crate.
//!
//! ## Leaf encoding
//!
//! For a page entry `(github_username, record)`:
//!
//! ```text
//! leaf = SHA256(LEAF_DOMAIN || username_bytes || 0x00 || address_strkey_bytes || verified_byte)
//! ```
//!
//! - `username_bytes` — the raw bytes of `github_username` exactly as it
//!   appears in the page (its canonical, lowercased storage-key form — see
//!   Issue #194 — since that is what a page actually contains).
//! - `0x00` — a fixed separator byte between the username and address
//!   fields, so `("ab", "c...")` cannot be confused with `("a", "bc...")`
//!   when the two byte strings are concatenated.
//! - `address_strkey_bytes` — the raw bytes of `record.stellar_address`'s
//!   Stellar strkey (`Address::to_string()`), not its internal binary
//!   encoding, since the strkey is what's trivial for off-chain tooling in
//!   any language to reproduce byte-for-byte from an exported record.
//! - `verified_byte` — `0x01` if `record.verified`, else `0x00`. Carrying the
//!   verified flag in the leaf means a proof attests to verification status
//!   at export time, not just membership.
//!
//! `merkle_leaf_hash` is exposed as a read-only contract entry point so
//! off-chain tooling can check its own reimplementation against the
//! on-chain one for a single entry before trusting proofs built from it.
//!
//! ## Tree construction
//!
//! A standard bottom-up binary tree over the page's leaves, in page order:
//!
//! ```text
//! node = SHA256(NODE_DOMAIN || left || right)
//! ```
//!
//! When a level has an odd number of nodes, the last (rightmost) node is
//! carried up to the next level **unchanged** — it is not duplicated or
//! re-hashed with itself. This avoids the well-known second-preimage
//! weakness of "duplicate the odd node" trees and keeps proof reconstruction
//! simple: a promoted node's proof step is "this hash, no sibling, same
//! side."
//!
//! An empty page's root is [`empty_root`], the all-zero `BytesN<32>` — a
//! sentinel no real leaf or node hash can produce (finding a SHA-256 preimage
//! of all zeros is computationally infeasible), and distinguishable at a
//! glance from every real root. A single-leaf page's root **is** that leaf's
//! hash: with one node at every level, each level "combines" by promotion
//! until only the leaf remains.
//!
//! ## Domain separation
//!
//! Leaf and node hashes are prefixed with distinct, versioned ASCII domain
//! strings (`LEAF_DOMAIN`, `NODE_DOMAIN`) so a leaf hash can never be
//! mistaken for a node hash — or for a hash from an unrelated protocol —
//! even though both are 32-byte SHA-256 outputs. Changing either string
//! changes the tree shape and must ship as a new version suffix (`v2`, ...),
//! never an in-place edit to `v1`.
//!
//! ## Scope
//!
//! This computes a root over a single page, not a historic accumulator over
//! the whole registry's lifetime, and there is no zero-knowledge proof of
//! membership — a verifier needs the leaf's plaintext fields (username,
//! address, verified flag) and a sibling-hash path, exactly like any
//! standard Merkle proof.

use soroban_sdk::{Bytes, BytesN, Env, String, Vec};

use crate::storage::ContributorRecord;

const LEAF_DOMAIN: &[u8] = b"trustbridge/export-leaf/v1:";
const NODE_DOMAIN: &[u8] = b"trustbridge/export-node/v1:";

/// Separator byte between the username and address fields of a leaf, so
/// `(a, bc)` and `(ab, c)` cannot hash to the same leaf.
const FIELD_SEPARATOR: u8 = 0x00;

/// The Merkle root of an empty page: the all-zero sentinel no real hash can
/// produce.
#[must_use]
pub fn empty_root(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[0u8; 32])
}

/// Computes the leaf hash for one export entry. See the module docs for the
/// exact byte layout.
#[must_use]
pub fn leaf_hash(
    env: &Env,
    username: &String,
    stellar_address: &soroban_sdk::Address,
    verified: bool,
) -> BytesN<32> {
    let mut buf = Bytes::from_slice(env, LEAF_DOMAIN);
    buf.append(&username.to_bytes());
    buf.push_back(FIELD_SEPARATOR);
    buf.append(&stellar_address.to_string().to_bytes());
    buf.push_back(u8::from(verified));
    env.crypto().sha256(&buf).to_bytes()
}

fn node_hash(env: &Env, left: &BytesN<32>, right: &BytesN<32>) -> BytesN<32> {
    let mut buf = Bytes::from_slice(env, NODE_DOMAIN);
    buf.append(&Bytes::from(left.clone()));
    buf.append(&Bytes::from(right.clone()));
    env.crypto().sha256(&buf).to_bytes()
}

/// Computes the Merkle root over `leaves`, in order.
///
/// `leaves.len() == 0` returns [`empty_root`]. `leaves.len() == 1` returns
/// that single leaf unchanged. See the module docs for the odd-node
/// promotion rule applied at every level above the leaves.
#[must_use]
pub fn root_of(env: &Env, leaves: &Vec<BytesN<32>>) -> BytesN<32> {
    if leaves.is_empty() {
        return empty_root(env);
    }

    let mut level: Vec<BytesN<32>> = leaves.clone();
    while level.len() > 1 {
        let mut next: Vec<BytesN<32>> = Vec::new(env);
        let mut i: u32 = 0;
        while i + 1 < level.len() {
            let left = level.get(i).unwrap();
            let right = level.get(i + 1).unwrap();
            next.push_back(node_hash(env, &left, &right));
            i += 2;
        }
        if i < level.len() {
            // Odd node out: promote unchanged rather than duplicate-hash it.
            next.push_back(level.get(i).unwrap());
        }
        level = next;
    }

    level.get(0).unwrap()
}

/// Computes the Merkle root over one export page's records, in page order.
#[must_use]
pub fn root_of_records(env: &Env, records: &Vec<(String, ContributorRecord)>) -> BytesN<32> {
    let mut leaves: Vec<BytesN<32>> = Vec::new(env);
    for i in 0..records.len() {
        let (username, record) = records.get(i).unwrap();
        leaves.push_back(leaf_hash(env, &username, &record.stellar_address, record.verified));
    }
    root_of(env, &leaves)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Address;

    fn rec(env: &Env, addr: &Address, verified: bool) -> ContributorRecord {
        ContributorRecord {
            stellar_address: addr.clone(),
            payout_address: addr.clone(),
            registered_at: 0,
            verified,
            is_bot: false,
        }
    }

    #[test]
    fn test_empty_page_root_is_all_zero() {
        let env = Env::default();
        let leaves: Vec<BytesN<32>> = Vec::new(&env);
        assert_eq!(root_of(&env, &leaves), empty_root(&env));
    }

    #[test]
    fn test_single_leaf_page_root_equals_the_leaf() {
        let env = Env::default();
        let addr = Address::generate(&env);
        let username = String::from_str(&env, "octocat");
        let leaf = leaf_hash(&env, &username, &addr, false);

        let mut leaves: Vec<BytesN<32>> = Vec::new(&env);
        leaves.push_back(leaf.clone());

        assert_eq!(root_of(&env, &leaves), leaf);
    }

    #[test]
    fn test_root_changes_when_verified_flag_changes() {
        let env = Env::default();
        let addr = Address::generate(&env);
        let username = String::from_str(&env, "octocat");

        let leaf_unverified = leaf_hash(&env, &username, &addr, false);
        let leaf_verified = leaf_hash(&env, &username, &addr, true);

        assert_ne!(leaf_unverified, leaf_verified);
    }

    #[test]
    fn test_root_of_records_matches_manual_leaf_hashes() {
        let env = Env::default();
        let a1 = Address::generate(&env);
        let a2 = Address::generate(&env);
        let u1 = String::from_str(&env, "alice");
        let u2 = String::from_str(&env, "bob");

        let mut records: Vec<(String, ContributorRecord)> = Vec::new(&env);
        records.push_back((u1.clone(), rec(&env, &a1, true)));
        records.push_back((u2.clone(), rec(&env, &a2, false)));

        let mut leaves: Vec<BytesN<32>> = Vec::new(&env);
        leaves.push_back(leaf_hash(&env, &u1, &a1, true));
        leaves.push_back(leaf_hash(&env, &u2, &a2, false));

        assert_eq!(root_of_records(&env, &records), root_of(&env, &leaves));
    }

    #[test]
    fn test_odd_leaf_count_promotes_the_last_leaf_unchanged() {
        let env = Env::default();
        let addr = Address::generate(&env);
        let l0 = leaf_hash(&env, &String::from_str(&env, "a"), &addr, false);
        let l1 = leaf_hash(&env, &String::from_str(&env, "b"), &addr, false);
        let l2 = leaf_hash(&env, &String::from_str(&env, "c"), &addr, false);

        let mut leaves: Vec<BytesN<32>> = Vec::new(&env);
        leaves.push_back(l0.clone());
        leaves.push_back(l1.clone());
        leaves.push_back(l2.clone());

        // Level 1: [node_hash(l0, l1), l2 promoted unchanged]
        // Root: node_hash(node_hash(l0, l1), l2)
        let expected = node_hash(&env, &node_hash(&env, &l0, &l1), &l2);
        assert_eq!(root_of(&env, &leaves), expected);
    }

    #[test]
    fn test_leaf_order_matters() {
        let env = Env::default();
        let addr = Address::generate(&env);
        let l0 = leaf_hash(&env, &String::from_str(&env, "a"), &addr, false);
        let l1 = leaf_hash(&env, &String::from_str(&env, "b"), &addr, false);

        let mut forward: Vec<BytesN<32>> = Vec::new(&env);
        forward.push_back(l0.clone());
        forward.push_back(l1.clone());

        let mut reversed: Vec<BytesN<32>> = Vec::new(&env);
        reversed.push_back(l1);
        reversed.push_back(l0);

        assert_ne!(root_of(&env, &forward), root_of(&env, &reversed));
    }
}
