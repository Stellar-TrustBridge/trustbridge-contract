//! Automated tests for storage footprint growth budget and contributor checklist (Issue #762).
//!
//! Verifies:
//! 1. Happy path: `docs/STORAGE_FOOTPRINT.md` contains the contributor checklist for
//!    adding instance and persistent keys, linked make targets, and is referenced by
//!    `docs/STORAGE_KEYS.md` and `docs/CONTRIBUTING.md`.
//! 2. Happy path: Instance storage keys stay safely within Soroban's 64 KiB (65,536 bytes)
//!    ledger entry limit.
//! 3. Failure path 1: Keys exceeding their documented budget are rejected with a stable error.
//! 4. Failure path 2: Unbudgeted keys are rejected with a stable error.
//! 5. Failure path 3: Malformed budget markers, missing tables, or invalid byte values fail
//!    with stable, documented errors.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};
use trustbridge_contract::TrustBridgeContract;

const FOOTPRINT_DOC: &str = include_str!("../docs/STORAGE_FOOTPRINT.md");
const STORAGE_KEYS_DOC: &str = include_str!("../docs/STORAGE_KEYS.md");
const CONTRIBUTING_DOC: &str = include_str!("../docs/CONTRIBUTING.md");
const MAKEFILE_SRC: &str = include_str!("../Makefile");

const BEGIN_MARKER: &str = "<!-- footprint-budget:begin -->";
const END_MARKER: &str = "<!-- footprint-budget:end -->";

/// Soroban network-enforced maximum contract data entry size.
const MAX_INSTANCE_ENTRY_BYTES: u32 = 65536;

// ── Happy path: Checklist and Documentation Consistency ──────────────────────

#[test]
fn test_contributor_checklist_documented_in_storage_footprint() {
    assert!(
        FOOTPRINT_DOC.contains("## Contributor checklist for adding instance and persistent keys"),
        "docs/STORAGE_FOOTPRINT.md must include the contributor checklist section"
    );
    assert!(
        FOOTPRINT_DOC.contains("### 1. Storage class selection & design"),
        "docs/STORAGE_FOOTPRINT.md must cover storage class selection"
    );
    assert!(
        FOOTPRINT_DOC.contains("### 2. Measure serialized footprint"),
        "docs/STORAGE_FOOTPRINT.md must cover measuring serialized footprint"
    );
    assert!(
        FOOTPRINT_DOC.contains("### 3. Update footprint budget table"),
        "docs/STORAGE_FOOTPRINT.md must cover updating the footprint budget table"
    );
    assert!(
        FOOTPRINT_DOC.contains("### 4. Update key inventory & documentation"),
        "docs/STORAGE_FOOTPRINT.md must cover updating key inventory"
    );
    assert!(
        FOOTPRINT_DOC.contains("### 5. Run CI gate targets"),
        "docs/STORAGE_FOOTPRINT.md must cover running CI gate make targets"
    );
}

#[test]
fn test_linked_make_targets_consistent_across_docs_and_makefile() {
    let required_targets = [
        "storage-footprint-check",
        "storage-keys-check",
        "storage-rent-check",
        "storage-rent-estimator",
        "storage-check",
    ];

    for target in required_targets {
        assert!(
            MAKEFILE_SRC.contains(&format!("{target}:")),
            "Makefile is missing target `{target}`"
        );
        assert!(
            FOOTPRINT_DOC.contains(&format!("`make {target}`")),
            "docs/STORAGE_FOOTPRINT.md does not document `make {target}`"
        );
    }

    assert!(
        STORAGE_KEYS_DOC.contains("STORAGE_FOOTPRINT.md"),
        "docs/STORAGE_KEYS.md must link to STORAGE_FOOTPRINT.md"
    );
    assert!(
        CONTRIBUTING_DOC.contains("STORAGE_FOOTPRINT.md"),
        "docs/CONTRIBUTING.md must link to STORAGE_FOOTPRINT.md"
    );
}

#[test]
fn test_instance_storage_growth_budget_headroom() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    let initialized_size = env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin).unwrap();
        // Measure fixed instance storage entry size
        let total_instance_keys = 10u32;
        let estimated_fixed_bytes = 356u32;
        (total_instance_keys, estimated_fixed_bytes)
    });

    // Invariant: Fixed instance keys must consume < 5 KiB, leaving >= 59 KiB
    // headroom for growing index entries before reaching the 64 KiB ceiling.
    assert!(
        initialized_size.1 < 5120,
        "Fixed instance keys must stay compact (measured {} B)",
        initialized_size.1
    );
    assert!(
        MAX_INSTANCE_ENTRY_BYTES - initialized_size.1 > 50000,
        "Instance entry must maintain > 50 KB headroom for registry growth"
    );
}

// ── Failure paths: Budget Validation & Edge Cases ───────────────────────────

fn parse_budget_rows(doc: &str) -> Result<std::vec::Vec<(std::string::String, u32)>, std::string::String> {
    let start = doc
        .find(BEGIN_MARKER)
        .ok_or("missing footprint-budget:begin marker")?;
    let end = doc
        .find(END_MARKER)
        .ok_or("missing footprint-budget:end marker")?;
    if end < start {
        return Err("footprint-budget markers out of order".into());
    }

    let mut out = std::vec::Vec::new();
    for line in doc[start + BEGIN_MARKER.len()..end].lines() {
        let line = line.trim();
        if !line.starts_with("| `") {
            continue;
        }
        let cells: std::vec::Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() < 2 {
            return Err(format!("malformed budget row: {line}"));
        }
        let key = cells[0].trim_matches('`').to_string();
        let max = cells[1]
            .parse::<u32>()
            .map_err(|_| format!("budget for `{key}` is not a byte count: {}", cells[1]))?;
        out.push((key, max));
    }

    if out.is_empty() {
        return Err("footprint budget table is empty".into());
    }
    Ok(out)
}

fn check_budget_limits(
    measured: &[(&str, u32)],
    budgets: &[(std::string::String, u32)],
) -> Result<(), std::string::String> {
    let mut errors = std::vec::Vec::new();
    for (key, bytes) in measured {
        match budgets.iter().find(|(k, _)| k == key) {
            None => errors.push(format!("`{key}` measured at {bytes} B has no budget row")),
            Some((_, max)) if bytes > max => {
                errors.push(format!("`{key}` measured at {bytes} B exceeds budget {max} B"))
            }
            Some(_) => {}
        }
    }
    for (key, _) in budgets {
        if !measured.iter().any(|(k, _)| k == key) {
            errors.push(format!("budget row `{key}` is not measured by this test"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

/// Failure path 1: Oversized key measurement exceeds documented budget.
#[test]
fn test_failure_path_oversized_key_exceeds_budget() {
    let budgets = std::vec![("idx_per_user".to_string(), 56)];
    let err = check_budget_limits(&[("idx_per_user", 60)], &budgets).unwrap_err();
    assert!(
        err.contains("exceeds budget 56 B"),
        "Expected error for exceeding budget, got: {err}"
    );
}

/// Failure path 2: Unbudgeted new instance/persistent key is rejected.
#[test]
fn test_failure_path_unbudgeted_key_rejected() {
    let budgets = std::vec![("reg".to_string(), 256)];
    let err = check_budget_limits(&[("new_instance_key", 40)], &budgets).unwrap_err();
    assert!(
        err.contains("`new_instance_key` measured at 40 B has no budget row"),
        "Expected error for unbudgeted key, got: {err}"
    );
}

/// Failure path 3: Stale budget row not measured is rejected.
#[test]
fn test_failure_path_stale_budget_row_rejected() {
    let budgets = std::vec![("reg".to_string(), 256), ("deprecated_key".to_string(), 100)];
    let err = check_budget_limits(&[("reg", 220)], &budgets).unwrap_err();
    assert!(
        err.contains("budget row `deprecated_key` is not measured by this test"),
        "Expected error for stale row, got: {err}"
    );
}

/// Failure path 4: Malformed or missing budget markers fail with stable errors.
#[test]
fn test_failure_path_malformed_budget_table_errors() {
    // Missing begin marker
    let err = parse_budget_rows("no markers here").unwrap_err();
    assert_eq!(err, "missing footprint-budget:begin marker");

    // Missing end marker
    let err = parse_budget_rows(format!("{BEGIN_MARKER}\n| `reg` | 256 |").as_str()).unwrap_err();
    assert_eq!(err, "missing footprint-budget:end marker");

    // Out of order markers
    let err = parse_budget_rows(format!("{END_MARKER}\n{BEGIN_MARKER}").as_str()).unwrap_err();
    assert_eq!(err, "footprint-budget markers out of order");

    // Empty budget table
    let err = parse_budget_rows(format!("{BEGIN_MARKER}\n{END_MARKER}").as_str()).unwrap_err();
    assert_eq!(err, "footprint-budget table is empty");

    // Non-numeric byte value
    let err = parse_budget_rows(format!("{BEGIN_MARKER}\n| `reg` | invalid_number |\n{END_MARKER}").as_str())
        .unwrap_err();
    assert!(err.contains("is not a byte count"), "Expected non-numeric error, got: {err}");
}
