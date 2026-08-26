//! Read-only lookup stub for migration-window dashboard sync.
//!
//! This module defines the shape of a cross-registry or legacy-registry
//! lookup without adding any contract entry points. It is intended for
//! off-chain consumers that need a deterministic fallback during a migration
//! window while the on-chain registry remains the source of truth.

extern crate alloc;

use alloc::string::String;

/// Read-only username lookup result.
#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryLookup {
    pub github_username: String,
    pub stellar_address: Option<String>,
    pub source_registry_id: String,
}

/// Read-only registry lookup interface.
#[allow(dead_code)]
pub trait RegistryReadStub {
    fn lookup(&self, github_username: &str) -> RegistryLookup;
}

/// Deterministic fixture used by tests and docs.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FixtureRegistryReadStub;

impl RegistryReadStub for FixtureRegistryReadStub {
    fn lookup(&self, github_username: &str) -> RegistryLookup {
        match github_username {
            "legacy-alice" => RegistryLookup {
                github_username: String::from("legacy-alice"),
                stellar_address: Some(String::from(
                    "GCFIXTUREALICE0000000000000000000000000000000000000000",
                )),
                source_registry_id: String::from("legacy-registry"),
            },
            "legacy-bob" => RegistryLookup {
                github_username: String::from("legacy-bob"),
                stellar_address: Some(String::from(
                    "GCFIXTUREBOB000000000000000000000000000000000000000000",
                )),
                source_registry_id: String::from("legacy-registry"),
            },
            _ => RegistryLookup {
                github_username: String::from(github_username),
                stellar_address: None,
                source_registry_id: String::from("legacy-registry"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_returns_deterministic_lookup_rows() {
        let stub = FixtureRegistryReadStub;

        assert_eq!(
            stub.lookup("legacy-alice"),
            RegistryLookup {
                github_username: String::from("legacy-alice"),
                stellar_address: Some(String::from(
                    "GCFIXTUREALICE0000000000000000000000000000000000000000"
                )),
                source_registry_id: String::from("legacy-registry"),
            }
        );

        assert_eq!(
            stub.lookup("unknown-user"),
            RegistryLookup {
                github_username: String::from("unknown-user"),
                stellar_address: None,
                source_registry_id: String::from("legacy-registry"),
            }
        );
    }
}
