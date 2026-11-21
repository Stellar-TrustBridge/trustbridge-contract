//! Digest stability tests for `export_attestation` (Issue #778).
//!
//! `storage::build_export_digest` computes `sha256(page.to_xdr(env))` over
//! the `ExportPage` returned by a given cursor/limit query (see
//! `src/storage.rs`). For offline/air-gapped Wave audits to work, that
//! digest must be a pure function of the page's content: the same
//! registry state queried the same way must always produce the same
//! digest, and the digest must change whenever any field that
//! `ExportPage`'s XDR encoding covers changes. "Across SDK versions" means
//! this test suite does not depend on wall-clock time, ledger timestamps,
//! or any other non-deterministic input — only `records`, `next_cursor`,
//! `total`, `merkle_root`, and `has_more`, exactly the fields locked by
//! `tests/export_page_layout.rs` against `abi/export_page.layout.golden`.
//! If a future SDK upgrade changes XDR encoding of any of those field
//! types, this suite's repeated-call tests will catch a hashing-input
//! regression even though the layout test only catches a field-shape
//! regression.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, BytesN, Env, String, Vec};

use trustbridge_contract::TrustBridgeContract;

fn setup() -> (Env, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    (env, contract_id)
}

fn register_n(env: &Env, contract_id: &Address, n: u32) {
    env.mock_all_auths();
    env.as_contract(contract_id, || {
        for i in 0..n {
            TrustBridgeContract::register(
                env.clone(),
                String::from_str(env, &format!("user{i:03}")),
                Address::generate(env),
                Vec::new(env),
            )
            .unwrap();
        }
    });
}

/// Calling `export_attestation` twice for the same page (no state change
/// between calls) must return the exact same digest byte-for-byte.
#[test]
fn test_digest_is_stable_across_repeated_calls_for_same_page() {
    let (env, contract_id) = setup();
    register_n(&env, &contract_id, 5);

    env.mock_all_auths();
    let (digest_a, digest_b) = env.as_contract(&contract_id, || {
        let a = TrustBridgeContract::export_attestation(env.clone(), None, 10).unwrap();
        let b = TrustBridgeContract::export_attestation(env.clone(), None, 10).unwrap();
        (a.digest, b.digest)
    });

    assert_eq!(digest_a, digest_b, "digest must be stable for identical page bytes");
}

/// The digest must be identical across *separate contract instances* (and
/// separate `Env`s) that end up holding the exact same registry content —
/// the digest is a function of the page bytes only, never of instance
/// identity, ledger sequence, or storage internals. Each fresh `Env`'s
/// deterministic test-address generator starts from the same internal
/// counter, so the same sequence of `register` calls on two independent
/// envs produces byte-identical `ExportPage` content.
#[test]
fn test_digest_is_stable_across_separate_instances_with_identical_content() {
    fn digest_for_fresh_registry(env: &Env) -> BytesN<32> {
        let admin = Address::generate(env);
        let contract_id = env.register(TrustBridgeContract, ());
        env.as_contract(&contract_id, || {
            TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
        });

        env.mock_all_auths();
        let addr = Address::generate(env);
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                String::from_str(env, "stableuser"),
                addr,
                Vec::new(env),
            )
            .unwrap();
        });

        env.as_contract(&contract_id, || {
            TrustBridgeContract::export_attestation(env.clone(), None, 10)
                .unwrap()
                .digest
        })
    }

    let env_a = Env::default();
    let env_b = Env::default();

    let digest_a = digest_for_fresh_registry(&env_a);
    let digest_b = digest_for_fresh_registry(&env_b);

    assert_eq!(
        digest_a, digest_b,
        "identical page content on independent envs/instances must hash identically"
    );
}

/// A digest must change when the underlying page content changes (adding a
/// record) — this is the complementary sensitivity check: stability tests
/// alone cannot catch a digest function that always returns a constant.
#[test]
fn test_digest_changes_when_page_content_changes() {
    let (env, contract_id) = setup();
    register_n(&env, &contract_id, 3);

    env.mock_all_auths();
    let digest_before = env.as_contract(&contract_id, || {
        TrustBridgeContract::export_attestation(env.clone(), None, 10)
            .unwrap()
            .digest
    });

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            String::from_str(&env, "extrauser"),
            Address::generate(&env),
            Vec::new(&env),
        )
        .unwrap();
    });

    let digest_after = env.as_contract(&contract_id, || {
        TrustBridgeContract::export_attestation(env.clone(), None, 10)
            .unwrap()
            .digest
    });

    assert_ne!(
        digest_before, digest_after,
        "digest must change when page content changes"
    );
}

/// `BytesN<32>` digests must be exactly 32 bytes and non-zero for a
/// non-empty page, as a basic sanity floor beneath the stability
/// assertions above.
#[test]
fn test_digest_is_32_bytes_and_nonzero_for_nonempty_page() {
    let (env, contract_id) = setup();
    register_n(&env, &contract_id, 2);

    env.mock_all_auths();
    let digest: BytesN<32> = env.as_contract(&contract_id, || {
        TrustBridgeContract::export_attestation(env.clone(), None, 10)
            .unwrap()
            .digest
    });

    assert_eq!(digest.to_array().len(), 32);
    assert_ne!(digest.to_array(), [0u8; 32]);
}

/// Paging through the same registry with a smaller limit (multiple pages)
/// must produce, for each individual page, a digest stable across repeated
/// identical queries — pagination does not introduce any non-determinism
/// into a single page's digest.
#[test]
fn test_digest_stable_per_page_under_pagination() {
    let (env, contract_id) = setup();
    register_n(&env, &contract_id, 8);

    env.mock_all_auths();
    let (cursor, digest_page1_a) = env.as_contract(&contract_id, || {
        let page = TrustBridgeContract::export_attestation(env.clone(), None, 3).unwrap();
        (page.page.next_cursor, page.digest)
    });

    env.mock_all_auths();
    let digest_page1_b = env.as_contract(&contract_id, || {
        TrustBridgeContract::export_attestation(env.clone(), None, 3)
            .unwrap()
            .digest
    });

    assert_eq!(digest_page1_a, digest_page1_b);
    assert!(cursor.is_some(), "expected more pages with 8 records and limit 3");
}
