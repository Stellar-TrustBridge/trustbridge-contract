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

/// Per-ledger cap on batch entry points that **write** state (Issue #227).
///
/// # Why this is lower than `max_batch_size`
///
/// The default 100 was a shape check, not a resource budget — it was never
/// derived from what a batch actually costs. A write batch pays, per accepted
/// entry, a persistent read, a persistent write, a TTL extension, an event
/// publish, and an audit-log append. The worst case is a full batch of
/// maximum-length (39-character) usernames that all need writing, and the
/// contract has no way to check its remaining instruction budget mid-loop:
/// Soroban exposes no such host function, so a batch that overruns simply
/// traps. There is no partial success to fall back on.
///
/// 25 is the cap this contract measures against. `test_bench_batch_verify_max`
/// runs a full batch of maximum-length usernames and asserts the measured cost
/// stays within a fraction of the per-transaction limit, so the headroom is a
/// number this repo checks rather than one someone remembered.
///
/// Raising this requires re-running that benchmark, not just editing the
/// constant.
pub const MAX_WRITE_BATCH: u32 = 25;

impl BatchConfig {
    /// Config for batch entry points that write state.
    ///
    /// See [`MAX_WRITE_BATCH`] for how the cap is derived.
    #[must_use]
    pub fn for_writes() -> Self {
        BatchConfig {
            max_batch_size: MAX_WRITE_BATCH,
            max_total_items: 10_000,
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
    fn test_write_batch_config_is_tighter_than_default() {
        let writes = BatchConfig::for_writes();
        assert_eq!(writes.max_batch_size, MAX_WRITE_BATCH);
        assert!(
            writes.max_batch_size < BatchConfig::default().max_batch_size,
            "write batches must be capped below the generic shape limit"
        );
        assert!(writes.is_valid_batch_size(MAX_WRITE_BATCH));
        assert!(!writes.is_valid_batch_size(MAX_WRITE_BATCH + 1));
        assert!(!writes.is_valid_batch_size(0));
    }

    #[test]
    fn test_batch_config() {
        let config = BatchConfig::default();
        assert!(config.is_valid_batch_size(50));
        assert!(!config.is_valid_batch_size(0));
        assert!(!config.is_valid_batch_size(101));
    }
}
