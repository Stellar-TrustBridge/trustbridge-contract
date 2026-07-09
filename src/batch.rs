/// Batch operation utilities for efficient contract interactions.
///
/// This module provides helpers for performing multiple operations efficiently,
/// particularly useful for dashboard syncing and bulk verifications.

use soroban_sdk::{String, Address, Env};

/// Result of a single batch operation.
#[derive(Clone, Debug)]
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
    pub fn success(id: String) -> Self {
        BatchOperationResult {
            success: true,
            id,
            error: None,
        }
    }

    /// Create a failed result with error message.
    pub fn failed(id: String, error: String) -> Self {
        BatchOperationResult {
            success: false,
            id,
            error: Some(error),
        }
    }
}

/// Summary statistics for batch operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchSummary {
    pub total: u32,
    pub successful: u32,
    pub failed: u32,
    pub success_rate: u32, // percentage
}

impl BatchSummary {
    /// Calculate summary from results.
    pub fn from_results(results: &[BatchOperationResult]) -> Self {
        let total = results.len() as u32;
        let successful = results.iter().filter(|r| r.success).count() as u32;
        let failed = total - successful;
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
    pub fn all_successful(&self) -> bool {
        self.failed == 0
    }

    /// Check if at least some operations succeeded.
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

impl BatchConfig {
    /// Create default batch configuration (safe limits).
    pub fn default() -> Self {
        BatchConfig {
            max_batch_size: 100,
            max_total_items: 10000,
        }
    }

    /// Validate that a batch size is acceptable.
    pub fn is_valid_batch_size(&self, size: u32) -> bool {
        size > 0 && size <= self.max_batch_size
    }
}

/// Chunk an iterator into fixed-size batches.
pub fn chunk_into_batches<T: Clone>(items: &[T], batch_size: usize) -> Vec<Vec<T>> {
    let mut batches = Vec::new();
    for chunk in items.chunks(batch_size) {
        batches.push(chunk.to_vec());
    }
    batches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_summary() {
        let results = vec![
            BatchOperationResult::success(String::from_str(&env, "user1")),
            BatchOperationResult::success(String::from_str(&env, "user2")),
            BatchOperationResult::failed(String::from_str(&env, "user3"), String::from_str(&env, "error")),
        ];

        let summary = BatchSummary::from_results(&results);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.successful, 2);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.success_rate, 66);
    }

    #[test]
    fn test_chunk_into_batches() {
        let items = vec![1, 2, 3, 4, 5];
        let batches = chunk_into_batches(&items, 2);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0], vec![1, 2]);
        assert_eq!(batches[1], vec![3, 4]);
        assert_eq!(batches[2], vec![5]);
    }

    #[test]
    fn test_batch_config() {
        let config = BatchConfig::default();
        assert!(config.is_valid_batch_size(50));
        assert!(!config.is_valid_batch_size(0));
        assert!(!config.is_valid_batch_size(101));
    }
}
