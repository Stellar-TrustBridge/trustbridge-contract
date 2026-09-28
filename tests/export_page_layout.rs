//! ABI layout regression checks for dashboard-facing export types.

fn declared_export_page_fields(source: &str) -> Vec<String> {
    let struct_start = source
        .find("pub struct ExportPage {")
        .expect("ExportPage declaration must exist");
    let struct_body = &source[struct_start..]
        .split_once('}')
        .expect("ExportPage declaration must be closed").0;

    struct_body
        .lines()
        .filter_map(|line| {
            let field = line.trim().strip_prefix("pub ")?;
            let (name, field_type) = field.split_once(':')?;
            Some(format!("{}: {}", name.trim(), field_type.trim_end_matches(',').trim()))
        })
        .collect()
}

#[test]
fn export_page_layout_preserves_golden_prefix() {
    let source = include_str!("../src/storage.rs");
    let golden = include_str!("../abi/export_page.layout.golden");
    let actual = declared_export_page_fields(source);
    let expected: Vec<&str> = golden
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();

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

#[test]
fn export_page_layout_version_is_exported_and_matches() {
    assert_eq!(
        trustbridge_contract::EXPORT_PAGE_LAYOUT_VERSION,
        2,
        "EXPORT_PAGE_LAYOUT_VERSION must match documented version"
    );
}

#[test]
fn typed_export_record_consumer_parsing() {
    use soroban_sdk::{Address, BytesN, Env, String, Vec as SVec};
    use trustbridge_contract::{ContributorRecord, ExportPage, ExportRecord};

    let env = Env::default();
    let stellar_addr = Address::generate(&env);
    let payout_addr = Address::generate(&env);

    let record = ContributorRecord {
        stellar_address: stellar_addr.clone(),
        payout_address: payout_addr.clone(),
        registered_at: 1_700_000_000,
        verified: true,
        is_bot: false,
    };

    let username = String::from_str(&env, "octocat");
    let export_entry: ExportRecord = (username.clone(), record.clone());

    let mut records = SVec::new(&env);
    records.push_back(export_entry);

    let page = ExportPage {
        records,
        next_cursor: Some(BytesN::from_array(&env, &[1u8; 8])),
        total: 1,
        merkle_root: BytesN::from_array(&env, &[2u8; 32]),
        has_more: false,
    };

    // Verify consumer can access all fields unambiguously
    assert_eq!(page.total, 1);
    assert!(!page.has_more);
    assert!(page.next_cursor.is_some());
    assert_eq!(page.records.len(), 1);

    let (parsed_user, parsed_record) = page.records.get(0).unwrap();
    assert_eq!(parsed_user, username);
    assert_eq!(parsed_record.stellar_address, stellar_addr);
    assert_eq!(parsed_record.payout_address, payout_addr);
    assert_eq!(parsed_record.registered_at, 1_700_000_000);
    assert!(parsed_record.verified);
    assert!(!parsed_record.is_bot);
}