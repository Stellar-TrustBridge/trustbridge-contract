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