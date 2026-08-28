//! Contract versioning and compatibility tracking.
//!
//! This module provides version management utilities for tracking contract
//! upgrades, migrations, and maintaining backward compatibility. The deployed
//! version is written to instance storage by `initialize` and exposed through
//! the `version` and `is_compatible` contract functions, which the generated
//! TypeScript bindings package uses to guard against ABI drift.

// Some items here are staged ahead of their call sites: they are covered by
// this module's own tests but are not yet wired into `lib.rs`.
#![allow(dead_code)]

/// Contract version information.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

/// First contract version exposing the `batch_verify` entry point.
///
/// `batch_verify` is additive — it introduces a new function without changing
/// any existing signature — so it lands as a minor bump. Off-chain callers
/// (dashboard, indexer, the generated TypeScript bindings) should gate on
/// `Version::supports_batch_verify` rather than assuming the function exists,
/// since a contract deployed at 1.0.0 will reject the invocation outright.
pub const BATCH_VERIFY_MIN_VERSION: Version = Version::new(1, 1, 0);

/// First contract version whose public read functions (`get_address`,
/// `has_record`, `get_public_paginated`, `get_stats`, `get_verified_count`,
/// `get_role`, `version`/`is_compatible`, …) are safe for a sibling contract
/// to call cross-contract (Wave issue #149).
///
/// The read surface those functions expose has been stable since the
/// contract's initial release, so this is 1.0.0 rather than a forward-looking
/// gate like [`BATCH_VERIFY_MIN_VERSION`] — it exists so a consuming contract
/// can assert compatibility the same way it does for `batch_verify`, instead
/// of special-casing "assume reads always work."
///
/// Admin-gated exports (`get_all_registered`, `get_registered_page`,
/// `get_registered_paginated`) are deliberately excluded: they call
/// `admin.require_auth()`, and a cross-contract invocation cannot supply the
/// registry admin's signature, so they are not part of this surface at any
/// version. See `docs/ABI.md` § Cross-Contract Read Interface.
pub const CROSS_CONTRACT_READ_MIN_VERSION: Version = Version::new(1, 0, 0);

impl Version {
    /// Create a new version.
    #[must_use]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Version {
            major,
            minor,
            patch,
        }
    }

    /// Whether this version exposes the `batch_verify` entry point.
    ///
    /// Lets a caller branch between one batched invocation and N individual
    /// `verify` calls without probing the contract and interpreting a failure.
    #[must_use]
    pub fn supports_batch_verify(&self) -> bool {
        self.is_compatible_with(BATCH_VERIFY_MIN_VERSION)
    }

    /// Whether this version's public read functions are safe to call
    /// cross-contract. See `CROSS_CONTRACT_READ_MIN_VERSION`.
    #[must_use]
    pub fn supports_cross_contract_reads(&self) -> bool {
        self.is_compatible_with(CROSS_CONTRACT_READ_MIN_VERSION)
    }

    /// Parse a version from a tuple (used for storage).
    #[must_use]
    pub fn from_tuple(tuple: (u32, u32, u32)) -> Self {
        Version {
            major: tuple.0,
            minor: tuple.1,
            patch: tuple.2,
        }
    }

    /// Convert version to a tuple for storage.
    #[must_use]
    pub fn to_tuple(&self) -> (u32, u32, u32) {
        (self.major, self.minor, self.patch)
    }

    /// Create version 1.0.0 (initial release).
    #[must_use]
    pub fn v1_0_0() -> Self {
        Version::new(1, 0, 0)
    }

    /// Check if this version is compatible with a minimum required version.
    #[must_use]
    pub fn is_compatible_with(&self, minimum: Version) -> bool {
        // Same major version, minor and patch must be >= minimum
        if self.major != minimum.major {
            return self.major > minimum.major;
        }
        if self.minor != minimum.minor {
            return self.minor > minimum.minor;
        }
        self.patch >= minimum.patch
    }

    /// Check if a migration is needed from old version to new version.
    #[must_use]
    pub fn needs_migration(&self, target: Version) -> bool {
        self != &target && self.major <= target.major
    }

    /// Get the next patch version (for hot fixes).
    #[must_use]
    pub fn bump_patch(&self) -> Version {
        Version::new(self.major, self.minor, self.patch + 1)
    }

    /// Get the next minor version (for features).
    #[must_use]
    pub fn bump_minor(&self) -> Version {
        Version::new(self.major, self.minor + 1, 0)
    }

    /// Get the next major version (for breaking changes).
    #[must_use]
    pub fn bump_major(&self) -> Version {
        Version::new(self.major + 1, 0, 0)
    }
}

impl core::fmt::Display for Version {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Contract migration state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MigrationState {
    /// No migration needed
    NotRequired,
    /// Migration is pending
    Pending,
    /// Migration is in progress
    InProgress,
    /// Migration completed successfully
    Completed,
    /// Migration failed
    Failed,
}

/// Compatibility information between versions.
#[derive(Clone, Copy, Debug)]
pub struct CompatibilityInfo {
    pub current_version: Version,
    pub target_version: Version,
    pub migration_required: bool,
    pub breaking_changes: bool,
    pub data_migration_required: bool,
}

impl CompatibilityInfo {
    /// Create compatibility info for an upgrade.
    pub fn for_upgrade(current: Version, target: Version) -> Self {
        let breaking_changes = current.major != target.major;
        let data_migration_required = breaking_changes || current.minor != target.minor;

        CompatibilityInfo {
            current_version: current,
            target_version: target,
            // Direction matters: the deployed version is what may need to move
            // to the target, not the other way round.
            migration_required: current.needs_migration(target),
            breaking_changes,
            data_migration_required,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate alloc;

    #[test]
    fn test_version_comparison() {
        let v1_0_0 = Version::v1_0_0();
        let v1_0_1 = Version::new(1, 0, 1);
        let v1_1_0 = Version::new(1, 1, 0);
        let v2_0_0 = Version::new(2, 0, 0);

        assert!(v1_0_1 > v1_0_0);
        assert!(v1_1_0 > v1_0_1);
        assert!(v2_0_0 > v1_1_0);
    }

    #[test]
    fn test_version_compatibility() {
        let v1_2_0 = Version::new(1, 2, 0);
        let v1_1_5 = Version::new(1, 1, 5);

        assert!(v1_2_0.is_compatible_with(v1_1_5));
        assert!(!v1_1_5.is_compatible_with(v1_2_0));
    }

    #[test]
    fn test_version_bumps() {
        let v1_2_3 = Version::new(1, 2, 3);

        assert_eq!(v1_2_3.bump_patch(), Version::new(1, 2, 4));
        assert_eq!(v1_2_3.bump_minor(), Version::new(1, 3, 0));
        assert_eq!(v1_2_3.bump_major(), Version::new(2, 0, 0));
    }

    #[test]
    fn test_version_display() {
        let v = Version::new(1, 2, 3);
        let s = alloc::format!("{}", v);
        assert_eq!(s, "1.2.3");
    }

    #[test]
    fn test_version_tuple_roundtrip() {
        let v = Version::new(2, 7, 13);
        assert_eq!(Version::from_tuple(v.to_tuple()), v);
    }

    #[test]
    fn test_needs_migration() {
        let current = Version::new(1, 0, 0);

        assert!(current.needs_migration(Version::new(1, 1, 0)));
        assert!(current.needs_migration(Version::new(2, 0, 0)));
        // Same version is already migrated.
        assert!(!current.needs_migration(current));
        // Downgrades to an older major are not a migration path.
        assert!(!Version::new(2, 0, 0).needs_migration(Version::new(1, 0, 0)));
    }

    #[test]
    fn test_compatibility_info_for_minor_upgrade() {
        let info = CompatibilityInfo::for_upgrade(Version::new(1, 0, 0), Version::new(1, 1, 0));

        assert!(info.migration_required);
        assert!(!info.breaking_changes);
        assert!(info.data_migration_required);
        assert_eq!(info.current_version, Version::new(1, 0, 0));
        assert_eq!(info.target_version, Version::new(1, 1, 0));
    }

    #[test]
    fn test_compatibility_info_for_major_upgrade() {
        let info = CompatibilityInfo::for_upgrade(Version::new(1, 4, 2), Version::new(2, 0, 0));

        assert!(info.migration_required);
        assert!(info.breaking_changes);
        assert!(info.data_migration_required);
    }

    #[test]
    fn test_compatibility_info_for_patch_upgrade() {
        let info = CompatibilityInfo::for_upgrade(Version::new(1, 0, 0), Version::new(1, 0, 1));

        assert!(info.migration_required);
        assert!(!info.breaking_changes);
        // A patch release keeps the storage layout, so no data migration.
        assert!(!info.data_migration_required);
    }

    #[test]
    fn test_migration_state_variants_are_distinct() {
        assert_ne!(MigrationState::NotRequired, MigrationState::Pending);
        assert_ne!(MigrationState::InProgress, MigrationState::Completed);
        assert_ne!(MigrationState::Completed, MigrationState::Failed);
    }

    // ABI breaking-change CI gate (Contract Wave #38).
    //
    // These pin the rules a CI gate relies on to decide whether a proposed
    // version bump is allowed to accompany an ABI-breaking change, and
    // whether a migration must run before the new version is trusted.

    #[test]
    fn gate_major_bump_is_flagged_as_breaking() {
        let current = Version::new(1, 4, 2);
        let target = current.bump_major();

        let info = CompatibilityInfo::for_upgrade(current, target);
        assert!(info.breaking_changes);
        assert!(info.data_migration_required);
    }

    #[test]
    fn gate_minor_and_patch_bumps_are_not_breaking() {
        let current = Version::new(1, 4, 2);

        let minor = CompatibilityInfo::for_upgrade(current, current.bump_minor());
        assert!(!minor.breaking_changes);
        assert!(minor.data_migration_required);

        let patch = CompatibilityInfo::for_upgrade(current, current.bump_patch());
        assert!(!patch.breaking_changes);
        assert!(!patch.data_migration_required);
    }

    #[test]
    fn gate_downgrade_is_not_treated_as_a_forward_migration() {
        // A CI gate must not let a "downgrade" masquerade as a safe
        // no-migration bump: needs_migration() only trips for forward
        // moves within the same major line or ahead.
        let current = Version::new(2, 0, 0);
        let older = Version::new(1, 9, 9);

        assert!(!current.needs_migration(older));
    }

    #[test]
    fn gate_same_version_requires_no_migration_and_is_not_breaking() {
        let v = Version::new(3, 1, 4);
        let info = CompatibilityInfo::for_upgrade(v, v);

        assert!(!info.migration_required);
        assert!(!info.breaking_changes);
        assert!(!info.data_migration_required);
    }

    #[test]
    fn gate_compatibility_check_rejects_lower_major() {
        // is_compatible_with() is the check a caller uses to assert a
        // deployed contract is new enough; a lower major must never read
        // as compatible even if minor/patch look newer.
        let deployed = Version::new(1, 9, 9);
        let minimum_required = Version::new(2, 0, 0);

        assert!(!deployed.is_compatible_with(minimum_required));
    }

    #[test]
    fn test_export_paginated_requires_admin_auth() {
        use soroban_sdk::{testutils::Address as _, Address, Env};
        use crate::TrustBridgeContract;

        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register(TrustBridgeContract, ());

        env.as_contract(&contract_id, || {
            TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
        });

        // Mock all auths to let the call succeed
        env.mock_all_auths();

        env.as_contract(&contract_id, || {
            let _page = TrustBridgeContract::get_registered_paginated(env.clone(), 0, 10).unwrap();
        });

        // Verify that get_registered_paginated indeed checked the admin's authorization
        let auths = env.auths();
        assert_eq!(auths.len(), 1);
        let (auth_addr, invocation) = auths.get(0).unwrap();
        assert_eq!(auth_addr, admin);
        assert_eq!(invocation.function.name, soroban_sdk::Symbol::new(&env, "get_registered_paginated"));
    }
}
