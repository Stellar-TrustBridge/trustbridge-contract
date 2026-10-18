//! Golden snapshot of public contract entrypoints (Issue #631).
//!
//! Compares the sorted `(name, arity)` list extracted from `docs/abi.json`
//! against `tests/testdata/abi/public_entrypoints.golden.json`. Error-enum
//! changes stay in `tests/contract_error_codes.rs` / `abi/contract_error_codes.golden`.
//!
//! Refresh after an intentional public interface change:
//!   python3 scripts/generate_abi_json.py --update-entrypoints-golden

const ABI_JSON: &str = include_str!("../docs/abi.json");
const GOLDEN: &str = include_str!("testdata/abi/public_entrypoints.golden.json");

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Entrypoint {
    name: String,
    arity: u32,
}

fn arity_from_signature(signature: &str) -> u32 {
    let start = signature.find('(').expect("signature has '('");
    let mut depth = 0i32;
    let mut end = None;
    for (i, ch) in signature[start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(start + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end.expect("matching ')'");
    let inner = signature[start + 1..end].trim();
    if inner.is_empty() {
        return 0;
    }
    let mut depth = 0i32;
    let mut parts = 1u32;
    for ch in inner.chars() {
        match ch {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => parts += 1,
            _ => {}
        }
    }
    parts
}

fn is_public_entrypoint(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    if name.starts_with('_') {
        return false;
    }
    if lowered.starts_with("test") || lowered.ends_with("_test") || lowered.contains("_test_") {
        return false;
    }
    true
}

fn extract_functions(abi_json: &str) -> Vec<(String, String)> {
    let marker = "\"functions\":";
    let start = abi_json.find(marker).expect("functions array") + marker.len();
    let rest = abi_json[start..].trim_start();
    let arr_start = rest.find('[').expect("[");
    let mut depth = 0i32;
    let mut end = arr_start;
    for (i, ch) in rest[arr_start..].char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    end = arr_start + i;
                    break;
                }
            }
            _ => {}
        }
    }
    let array = &rest[arr_start + 1..end];
    let mut out = Vec::new();
    for chunk in array.split('{') {
        let name = json_string_field(chunk, "name");
        let signature = json_string_field(chunk, "signature");
        if let (Some(name), Some(signature)) = (name, signature) {
            out.push((name, signature));
        }
    }
    out
}

fn json_string_field(object: &str, field: &str) -> Option<String> {
    let key = format!("\"{field}\"");
    let idx = object.find(&key)?;
    let after = object[idx + key.len()..].trim_start();
    let after = after.strip_prefix(':')?.trim_start();
    let after = after.strip_prefix('"')?;
    let mut escaped = false;
    let mut value = String::new();
    for ch in after.chars() {
        if escaped {
            value.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            return Some(value);
        }
        value.push(ch);
    }
    None
}

fn public_entrypoints_from_abi(abi_json: &str) -> Vec<Entrypoint> {
    let mut items: Vec<Entrypoint> = extract_functions(abi_json)
        .into_iter()
        .filter(|(name, _)| is_public_entrypoint(name))
        .map(|(name, signature)| Entrypoint {
            arity: arity_from_signature(&signature),
            name,
        })
        .collect();
    items.sort();
    items
}

fn parse_golden(golden: &str) -> Vec<Entrypoint> {
    let marker = "\"entrypoints\":";
    let start = golden.find(marker).expect("entrypoints array") + marker.len();
    let rest = golden[start..].trim_start();
    let arr_start = rest.find('[').expect("[");
    let mut depth = 0i32;
    let mut end = arr_start;
    for (i, ch) in rest[arr_start..].char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    end = arr_start + i;
                    break;
                }
            }
            _ => {}
        }
    }
    let array = &rest[arr_start + 1..end];
    let mut items = Vec::new();
    for chunk in array.split('{') {
        let name = json_string_field(chunk, "name");
        let arity = chunk
            .find("\"arity\"")
            .and_then(|i| {
                chunk[i..]
                    .split(':')
                    .nth(1)
                    .and_then(|s| s.trim().split(|c: char| !c.is_ascii_digit()).next())
                    .and_then(|n| n.parse().ok())
            });
        if let (Some(name), Some(arity)) = (name, arity) {
            items.push(Entrypoint { name, arity });
        }
    }
    items.sort();
    items
}

#[test]
fn current_abi_matches_golden_entrypoint_snapshot() {
    let from_abi = public_entrypoints_from_abi(ABI_JSON);
    let from_golden = parse_golden(GOLDEN);
    assert_eq!(
        from_abi, from_golden,
        "public entrypoint list drifted. If this is intentional, run \
         python3 scripts/generate_abi_json.py --update-entrypoints-golden"
    );
}

#[test]
fn removed_entrypoint_is_detected() {
    let mut items = public_entrypoints_from_abi(ABI_JSON);
    assert!(items.len() > 1, "fixture ABI must list more than one function");
    items.pop();
    assert_ne!(items, parse_golden(GOLDEN));
}

#[test]
fn renamed_entrypoint_is_detected() {
    let mut items = public_entrypoints_from_abi(ABI_JSON);
    items[0].name = format!("{}__renamed", items[0].name);
    items.sort();
    assert_ne!(items, parse_golden(GOLDEN));
}

#[test]
fn arity_mismatch_is_detected() {
    let mut items = public_entrypoints_from_abi(ABI_JSON);
    items[0].arity = items[0].arity.saturating_add(1);
    items.sort();
    assert_ne!(items, parse_golden(GOLDEN));
}

#[test]
fn comparison_is_order_independent() {
    let mut shuffled = public_entrypoints_from_abi(ABI_JSON);
    shuffled.reverse();
    shuffled.sort();
    assert_eq!(shuffled, parse_golden(GOLDEN));
}

#[test]
fn internal_and_test_only_names_are_excluded() {
    let sample = r#"{
      "functions": [
        {"name": "initialize", "signature": "initialize(admin: Address) -> Result<(), ContractError>"},
        {"name": "_internal", "signature": "_internal() -> ()"},
        {"name": "test_helper", "signature": "test_helper() -> ()"}
      ]
    }"#;
    let names: Vec<_> = public_entrypoints_from_abi(sample)
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, vec!["initialize".to_string()]);
}

#[test]
fn error_codes_remain_a_separate_snapshot() {
    let error_golden = include_str!("../abi/contract_error_codes.golden");
    assert!(
        error_golden.contains("AlreadyInitialized"),
        "error-code golden must remain the source of truth for ContractError"
    );
    assert!(
        !GOLDEN.contains("AlreadyInitialized"),
        "entrypoint snapshot must not duplicate the error-enum table"
    );
}
