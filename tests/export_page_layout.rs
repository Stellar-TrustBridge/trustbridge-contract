//! ABI layout regression checks for dashboard-facing export types.

fn declared_export_page_fields(source: &str) -> Vec<String> {
    let struct_start = source
        .find("pub struct ExportPage {")
        .expect("ExportPage declaration must exist");
    let struct_body = &source[struct_start..]
        .split_once('}')
        .expect("ExportPage declaration must be closed")
        .0;

    struct_body
        .lines()
        .filter_map(|line| {
            let field = line.trim().strip_prefix("pub ")?;
            let (name, field_type) = field.split_once(':')?;
            Some(format!(
                "{}: {}",
                name.trim(),
                field_type.trim_end_matches(',').trim()
            ))
        })
        .collect()
}

fn golden_export_page_fields(golden: &str) -> Vec<&str> {
    golden
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

#[test]
fn export_page_layout_matches_golden() {
    let source = include_str!("../src/storage.rs");
    let golden = include_str!("../abi/export_page.layout.golden");
    let actual = declared_export_page_fields(source);
    let expected = golden_export_page_fields(golden);

    assert_eq!(
        actual, expected,
        "ExportPage layout drifted from abi/export_page.layout.golden; update the golden and document the ABI break"
    );
}

#[test]
fn export_page_layout_preserves_golden_prefix() {
    let source = include_str!("../src/storage.rs");
    let golden = include_str!("../abi/export_page.layout.golden");
    let actual = declared_export_page_fields(source);
    let expected = golden_export_page_fields(golden);

    assert!(
        actual.len() >= expected.len(),
        "ExportPage fields were removed; update the golden only for an intentional ABI break"
    );
    for (index, expected_field) in expected.iter().enumerate() {
        assert_eq!(
            actual[index], *expected_field,
            "ExportPage field {} changed name, type, or order; update the golden and document the ABI break",
            index
        );
    }
}

// ── Deterministic byte layout (Issue #692) ──────────────────────────────────
//
// The field-name golden above guards the declared struct. The checks below
// guard the bytes: the XDR an auditor hashes for an `ExportPage` must depend
// only on registry contents and order, never on the host, the `Env`
// instance, or how the page was reached.

mod byte_layout {
    use soroban_sdk::{
        testutils::Address as _, xdr::ToXdr, Address, Bytes, BytesN, Env, String, Vec,
    };
    use trustbridge_contract::{ExportPage, TrustBridgeContract};

    /// Builds a fresh registry, registering `names` in order with the
    /// deterministic test-address generator, and returns the first page.
    fn export(names: &[&str], limit: u32) -> (Env, ExportPage) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register(TrustBridgeContract, ());
        let page = env.as_contract(&contract_id, || {
            TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
            for name in names {
                TrustBridgeContract::register(
                    env.clone(),
                    String::from_str(&env, name),
                    Address::generate(&env),
                    Vec::new(&env),
                )
                .unwrap();
            }
            TrustBridgeContract::get_registered_paginated(env.clone(), None, limit).unwrap()
        });
        (env, page)
    }

    fn xdr_bytes(env: &Env, page: &ExportPage) -> std::vec::Vec<u8> {
        let bytes: Bytes = page.clone().to_xdr(env);
        bytes.iter().collect()
    }

    #[test]
    fn identical_registries_export_identical_page_bytes() {
        let (env_a, page_a) = export(&["alice", "bob", "carol"], 2);
        let (env_b, page_b) = export(&["alice", "bob", "carol"], 2);

        let bytes_a = xdr_bytes(&env_a, &page_a);
        assert!(!bytes_a.is_empty());
        assert_eq!(
            bytes_a,
            xdr_bytes(&env_b, &page_b),
            "ExportPage XDR differs between two identical registries"
        );
        // Serializing the same page twice is also stable.
        assert_eq!(bytes_a, xdr_bytes(&env_a, &page_a));
    }

    #[test]
    fn merkle_root_and_cursor_bytes_are_deterministic() {
        let (_, page_a) = export(&["alice", "bob", "carol"], 2);
        let (_, page_b) = export(&["alice", "bob", "carol"], 2);

        assert_eq!(page_a.merkle_root, page_b.merkle_root);
        assert_ne!(page_a.merkle_root.to_array(), [0u8; 32]);

        // Cursor layout: offset (u32 BE) || index generation (u32 BE).
        let cursor: BytesN<8> = page_a.next_cursor.clone().unwrap();
        assert_eq!(cursor.to_array(), [0, 0, 0, 2, 0, 0, 0, 0]);
        assert_eq!(page_a.next_cursor, page_b.next_cursor);
        assert_eq!((page_a.total, page_a.has_more), (3, true));
    }

    #[test]
    fn empty_page_has_fixed_byte_layout() {
        let (env_a, page_a) = export(&[], 10);
        let (env_b, page_b) = export(&[], 10);

        assert_eq!(page_a.merkle_root.to_array(), [0u8; 32]);
        assert_eq!(page_a.next_cursor, None);
        assert_eq!(xdr_bytes(&env_a, &page_a), xdr_bytes(&env_b, &page_b));
    }

    /// Failure path: the layout is order-sensitive, so a reordered registry
    /// must not produce bytes or a root that verify against the original.
    #[test]
    fn reordered_registry_changes_page_bytes_and_root() {
        let (env_a, page_a) = export(&["alice", "bob"], 10);
        let (env_b, page_b) = export(&["bob", "alice"], 10);

        assert_ne!(xdr_bytes(&env_a, &page_a), xdr_bytes(&env_b, &page_b));
        assert_ne!(page_a.merkle_root, page_b.merkle_root);
    }

    /// Failure path: any change in record contents changes the bytes.
    #[test]
    fn different_contents_change_page_bytes() {
        let (env_a, page_a) = export(&["alice", "bob"], 10);
        let (env_b, page_b) = export(&["alice", "carol"], 10);

        assert_ne!(xdr_bytes(&env_a, &page_a), xdr_bytes(&env_b, &page_b));
        assert_ne!(page_a.merkle_root, page_b.merkle_root);
    }
}

// ── Pending reverify flags do not gate export attestation (Issue #864) ──────
//
// A Wave rotation can leave some usernames flagged for re-verification while
// auditors still need signed exports. `PENDING_REVERIFY_KEY` is an advisory
// per-username flag: it is not an `ExportPage` field and must not block
// `export_attestation` or change its digest except through normal record
// changes. The failure paths below pin the pre-existing error semantics.

mod pending_reverify_attestation {
    use soroban_sdk::{
        testutils::Address as _, xdr::ToXdr, Address, Bytes, BytesN, Env, String, Vec,
    };
    use trustbridge_contract::{ContractError, ExportPage, TrustBridgeContract};

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
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

    fn register(env: &Env, contract_id: &Address, name: &str) {
        env.as_contract(contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                s(env, name),
                Address::generate(env),
                Vec::new(env),
            )
            .unwrap();
        });
    }

    /// Register → verify → re-register at a new address sets `pend_rev`.
    fn flag_reverify(env: &Env, contract_id: &Address, admin: &Address, name: &str) {
        register(env, contract_id, name);
        env.as_contract(contract_id, || {
            TrustBridgeContract::verify(env.clone(), admin.clone(), s(env, name)).unwrap();
        });
        register(env, contract_id, name);
    }

    fn sha256_page(env: &Env, page: &ExportPage) -> BytesN<32> {
        let xdr: Bytes = page.clone().to_xdr(env);
        env.crypto().sha256(&xdr).into()
    }

    /// Happy path: two of three records are flagged, yet the attestation is
    /// identical to a plain export and its digest stays stable.
    #[test]
    fn attestation_succeeds_with_partial_pending_reverify_flags() {
        let (env, admin, contract_id) = setup();
        for name in ["alice", "bob", "carol"] {
            register(&env, &contract_id, name);
        }
        flag_reverify(&env, &contract_id, &admin, "alice");
        flag_reverify(&env, &contract_id, &admin, "carol");

        let pending = env.as_contract(&contract_id, || {
            TrustBridgeContract::get_pending_reverify(env.clone(), 0, 100).unwrap()
        });
        assert_eq!(
            pending,
            Vec::from_array(&env, [s(&env, "alice"), s(&env, "carol")])
        );

        // Separate frames: each admin-authenticated read gets its own auth
        // frame (two `require_auth`s in one frame trip `Auth(ExistingValue)`).
        let att = env.as_contract(&contract_id, || {
            TrustBridgeContract::export_attestation(env.clone(), None, 10).unwrap()
        });
        let page = env.as_contract(&contract_id, || {
            TrustBridgeContract::get_registered_paginated(env.clone(), None, 10).unwrap()
        });
        let att_again = env.as_contract(&contract_id, || {
            TrustBridgeContract::export_attestation(env.clone(), None, 10).unwrap()
        });

        assert_eq!(att.page, page);
        assert_eq!(att.page.records.len(), 3);
        assert_eq!(att.digest, sha256_page(&env, &att.page));
        assert_eq!(
            att_again.digest, att.digest,
            "repeated attestation must stay byte-identical with flags present"
        );
    }

    /// Failure path 1: flags present do not excuse a stale cursor.
    #[test]
    fn attestation_still_rejects_stale_cursor_with_flags_set() {
        let (env, admin, contract_id) = setup();
        for name in ["alice", "bob", "carol", "dave"] {
            register(&env, &contract_id, name);
        }
        flag_reverify(&env, &contract_id, &admin, "alice");
        flag_reverify(&env, &contract_id, &admin, "carol");

        let cursor = env.as_contract(&contract_id, || {
            TrustBridgeContract::export_attestation(env.clone(), None, 2)
                .unwrap()
                .page
                .next_cursor
                .unwrap()
        });
        env.as_contract(&contract_id, || {
            TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, "bob")).unwrap();
        });

        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::export_attestation(env.clone(), Some(cursor), 2),
                Err(ContractError::InvalidCursor)
            );
            // The flags survived; only the invalid read failed.
            assert_eq!(
                TrustBridgeContract::get_pending_reverify(env.clone(), 0, 100).unwrap(),
                Vec::from_array(&env, [s(&env, "alice"), s(&env, "carol")])
            );
        });
    }

    /// Failure path 2: initialization/auth ordering is unchanged — flags do
    /// not create an implicit admin or an implicit initialized registry.
    #[test]
    fn uninitialized_attestation_still_errors() {
        let env = Env::default();
        let contract_id = env.register(TrustBridgeContract, ());
        env.as_contract(&contract_id, || {
            assert_eq!(
                TrustBridgeContract::export_attestation(env.clone(), None, 10),
                Err(ContractError::NotInitialized)
            );
            assert_eq!(
                TrustBridgeContract::get_pending_reverify(env.clone(), 0, 10),
                Err(ContractError::NotInitialized)
            );
        });
    }
}
