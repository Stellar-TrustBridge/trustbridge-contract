// Helper module staged ahead of its call sites: the items below are part of the
// contract's internal toolkit and are covered by this module's own tests, but
// are not yet wired into `lib.rs`.
#![allow(dead_code)]
/// Batch operation utilities for efficient contract interactions.
///
/// This module provides helpers for performing multiple operations efficiently,
/// particularly useful for dashboard syncing and bulk verifications.
use soroban_sdk::{contracttype, String};

/// Result of a single batch operation.
///
/// `#[contracttype]` so it can cross the contract boundary — these types
/// existed but were plain Rust structs, which meant nothing in this module
/// could ever be returned from a contract function.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchOperationResult {
    /// Whether the operation succeeded
    pub success: bool,
    /// Operation identifier (e.g., username or address)
    pub id: String,
    /// Optional error message
    pub error: Option<String>,
}

impl BatchOperationResult {
    /// Create a successful result.
    #[must_use]
    pub fn success(id: String) -> Self {
        BatchOperationResult {
            success: true,
            id,
            error: None,
        }
    }

    /// Create a failed result with error message.
    #[must_use]
    pub fn failed(id: String, error: String) -> Self {
        BatchOperationResult {
            success: false,
            id,
            error: Some(error),
        }
    }
}

/// Summary statistics for batch operations.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchSummary {
    pub total: u32,
    pub successful: u32,
    pub failed: u32,
    pub success_rate: u32, // percentage
}

impl BatchSummary {
    /// Calculate summary from count.
    #[must_use]
    pub fn new(total: u32, successful: u32) -> Self {
        let failed = total.saturating_sub(successful);
        let success_rate = if total > 0 {
            ((successful as u64 * 100) / (total as u64)) as u32
        } else {
            0
        };

        BatchSummary {
            total,
            successful,
            failed,
            success_rate,
        }
    }

    /// Check if all operations succeeded.
    #[must_use]
    pub fn all_successful(&self) -> bool {
        self.failed == 0
    }

    /// Check if at least some operations succeeded.
    #[must_use]
    pub fn any_successful(&self) -> bool {
        self.successful > 0
    }
}

/// Configuration for batch operation limits.
#[derive(Clone, Copy, Debug)]
pub struct BatchConfig {
    /// Maximum items per batch
    pub max_batch_size: u32,
    /// Maximum total items to process
    pub max_total_items: u32,
}

impl Default for BatchConfig {
    fn default() -> Self {
        BatchConfig {
            max_batch_size: 100,
            max_total_items: 10000,
        }
    }
}

impl BatchConfig {
    /// Validate that a batch size is acceptable.
    #[must_use]
    pub fn is_valid_batch_size(&self, size: u32) -> bool {
        size > 0 && size <= self.max_batch_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_summary() {
        let summary = BatchSummary::new(3, 2);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.successful, 2);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.success_rate, 66);
    }

    #[test]
    fn test_batch_config() {
        let config = BatchConfig::default();
        assert!(config.is_valid_batch_size(50));
        assert!(!config.is_valid_batch_size(0));
        assert!(!config.is_valid_batch_size(101));
    }
}
