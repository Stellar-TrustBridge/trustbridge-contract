//! Migration step registry documentation template (Issue #858).
//!
//! Kept as a standalone integration test so it runs independently of the
//! pre-existing lib test compilation state. Run with:
//! `cargo test --test migration_registry`.

const ARCHITECTURE: &str = include_str!("../docs/ARCHITECTURE.md");
const CONTRIBUTING: &str = include_str!("../docs/CONTRIBUTING.md");
const VERSION_SRC: &str = include_str!("../src/version.rs");
const STORAGE_SRC: &str = include_str!("../src/storage.rs");

use trustbridge_contract::Version;

/// Happy path: the registry template is present everywhere a Wave operator or
/// contributor will look, and forward semver is recognized as a migration.
#[test]
fn migration_registry_template_is_documented_and_forward_step_is_valid() {
    assert!(ARCHITECTURE.contains("Migration Step Registry (Issue #858)"));
    assert!(ARCHITECTURE
        .contains("| Step | From → To | Idempotency key | Operator action | Automated test |"));
    assert!(CONTRIBUTING.contains("Registering a migration step (Issue #858)"));
    assert!(VERSION_SRC.contains("Migration step registry (Issue #858)"));
    assert!(STORAGE_SRC.contains("migration step registry"));

    let from = Version::new(1, 0, 0);
    let to = Version::new(1, 1, 0);
    assert!(from.needs_migration(to), "forward step must be registered");
}

/// Failure path 1 — equal target is not a migration, so a registry row cannot
/// double-apply a step to the already-deployed version.
#[test]
fn equal_target_is_not_a_registered_migration() {
    let v = Version::new(2, 3, 4);
    assert!(!v.needs_migration(v));
    assert!(ARCHITECTURE.contains("Equal target"));
}

/// Failure path 2 — a lower major target is rejected; the registry documents
/// that there is no downgrade step.
#[test]
fn lower_target_is_not_a_registered_migration() {
    assert!(!Version::new(2, 0, 0).needs_migration(Version::new(1, 9, 9)));
    assert!(!Version::new(3, 1, 2).needs_migration(Version::new(2, 9, 9)));
    assert!(!Version::new(1, 0, 1).needs_migration(Version::new(0, 9, 9)));
    assert!(ARCHITECTURE.contains("Lower target"));
}
