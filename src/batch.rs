//! Batch operation utilities for efficient contract interactions.
//!
//! This module provides helpers for performing multiple operations efficiently,
//! particularly useful for dashboard syncing and bulk verifications.
use super::{
    bump_ever_verified_count, clear_pending_reverify, event_domain, get_record, is_admin_caller,
    push_audit_entry, require_initialized, require_not_paused, require_role_not_expired,
    set_record, set_verified_count, storage_get_role, storage_get_verified_count,
    storage_is_active_verifier, verifier_allowlist_active,
};
use crate::storage::{charge_verify_rate, is_verification_expired, set_verified_at};
use crate::{AuditEventType, AuditLogEntry, ContractError, Role, VerifiedEvent};
use soroban_sdk::{contracttype, Address, Env, String, Vec};

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
        let success_rate = crate::utils::calculate_verification_percentage(successful, total);

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

pub(super) fn batch_verify(
    env: Env,
    caller: Address,
    usernames: Vec<String>,
) -> Result<BatchSummary, ContractError> {
    require_initialized(&env)?;
    require_not_paused(&env)?;

    // Budget cap, not just a shape check — see `MAX_WRITE_BATCH` (Issue #227).
    let config = BatchConfig::for_writes();
    if !config.is_valid_batch_size(usernames.len()) {
        return Err(ContractError::InvalidBatchSize);
    }

    caller.require_auth();

    let is_admin = is_admin_caller(&env, &caller);
    if !is_admin {
        require_role_not_expired(&env, &caller)?;
    }
    // Verifier authorization (Issue #293): once the campaign allowlist has
    // been populated, a non-admin caller must be an active (non-expired)
    // allowlist member. Until then, pure role-based mode preserves existing
    // deployments.
    let is_verifier = if is_admin {
        false
    } else if verifier_allowlist_active(&env) {
        storage_is_active_verifier(&env, &caller, env.ledger().timestamp())
    } else {
        storage_get_role(&env, &caller) == Some(Role::Verifier)
    };
    if !is_admin && !is_verifier {
        return Err(ContractError::NotAuthorized);
    }

    // Charge one rate-limit unit per requested username before deduplication;
    // reject atomically if a non-admin batch would exceed the per-ledger cap.
    if !is_admin {
        charge_verify_rate(&env, &caller, usernames.len())?;
    }

    let total = usernames.len();
    let timestamp = env.ledger().timestamp();

    // Resolve and deduplicate the full batch before writing any records.
    let mut pending: Vec<String> = Vec::new(&env);
    for username in usernames.iter() {
        let Some(record) = get_record(&env, &username) else {
            continue;
        };
        let active = record.verified && !is_verification_expired(&env, &username);
        if active || pending.iter().any(|existing| existing == username) {
            continue;
        }
        pending.push_back(username);
    }

    let mut successful: u32 = 0;
    let mut newly_verified: u32 = 0;
    for username in pending.iter() {
        let Some(mut record) = get_record(&env, &username) else {
            continue;
        };

        let was_verified = record.verified;
        record.verified = true;
        set_record(&env, &username, &record);
        set_verified_at(&env, &username, timestamp);
        if !was_verified {
            newly_verified = newly_verified.saturating_add(1);
            bump_ever_verified_count(&env);
        }
        clear_pending_reverify(&env, &username);

        VerifiedEvent {
            github_username: username.clone(),
            stellar_address: record.stellar_address.clone(),
            timestamp,
            domain: event_domain(&env),
        }
        .publish(&env);

        push_audit_entry(
            &env,
            AuditLogEntry::new(
                AuditEventType::UserVerified,
                timestamp,
                Some(caller.clone()),
            )
            .with_username(username.clone())
            .with_address(record.stellar_address),
        );

        successful = successful.saturating_add(1);
    }

    // Update the aggregate count once, and only for false-to-true transitions.
    if newly_verified > 0 {
        set_verified_count(
            &env,
            storage_get_verified_count(&env).saturating_add(newly_verified),
        );
    }

    Ok(BatchSummary::new(total, successful))
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
    use crate::TrustBridgeContract;
    use soroban_sdk::testutils::Address as _;

    fn setup(env: &Env) -> (Address, Address, Address, Address) {
        let admin = Address::generate(env);
        let user = Address::generate(env);
        let other = Address::generate(env);
        let contract_id = env.register(TrustBridgeContract, ());
        env.as_contract(&contract_id, || {
            TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
        });
        (admin, user, other, contract_id)
    }

    fn username(env: &Env, name: &str) -> String {
        String::from_str(env, name)
    }

    #[test]
    fn test_batch_summary() {
        let summary = BatchSummary::new(3, 2);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.successful, 2);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.success_rate, 67);
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

    #[test]
    fn test_batch_verify_happy_path() {
        let env = Env::default();
        let (admin, user1, user2, contract_id) = setup(&env);
        let user3 = Address::generate(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            for (name, user) in [("user1", user1), ("user2", user2), ("user3", user3)] {
                TrustBridgeContract::register(
                    env.clone(),
                    username(&env, name),
                    user,
                    Vec::new(&env),
                )
                .unwrap();
            }
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames = soroban_sdk::vec![
                &env,
                username(&env, "user1"),
                username(&env, "user2"),
                username(&env, "user3"),
            ];
            let summary =
                TrustBridgeContract::batch_verify(env.clone(), admin.clone(), usernames).unwrap();
            assert_eq!(summary.total, 3);
            assert_eq!(summary.successful, 3);
            assert_eq!(summary.failed, 0);
            assert_eq!(summary.success_rate, 100);
            assert_eq!(TrustBridgeContract::get_verified_count(env.clone()), 3);
        });
    }

    #[test]
    fn test_batch_verify_partial_and_mixed() {
        let env = Env::default();
        let (admin, user1, _user2, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "user1"),
                user1,
                Vec::new(&env),
            )
            .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username(&env, "user1"))
                .unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames =
                soroban_sdk::vec![&env, username(&env, "user1"), username(&env, "user2"),];
            let summary =
                TrustBridgeContract::batch_verify(env.clone(), admin.clone(), usernames).unwrap();
            assert_eq!(summary.total, 2);
            assert_eq!(summary.successful, 0);
            assert_eq!(summary.failed, 2);
            assert_eq!(summary.success_rate, 0);
        });
    }

    #[test]
    fn test_batch_verify_verifier_role() {
        let env = Env::default();
        let (_admin, user1, verifier, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), verifier.clone(), Role::Verifier).unwrap();
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "user1"),
                user1,
                Vec::new(&env),
            )
            .unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames = soroban_sdk::vec![&env, username(&env, "user1")];
            let summary =
                TrustBridgeContract::batch_verify(env.clone(), verifier.clone(), usernames)
                    .unwrap();
            assert_eq!(summary.successful, 1);
        });
    }

    #[test]
    fn test_batch_verify_upgrader_rejected() {
        let env = Env::default();
        let (_admin, user1, upgrader, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::set_role(env.clone(), upgrader.clone(), Role::Upgrader).unwrap();
            TrustBridgeContract::register(
                env.clone(),
                username(&env, "user1"),
                user1,
                Vec::new(&env),
            )
            .unwrap();
        });

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let usernames = soroban_sdk::vec![&env, username(&env, "user1")];
            assert_eq!(
                TrustBridgeContract::batch_verify(env.clone(), upgrader.clone(), usernames),
                Err(ContractError::NotAuthorized)
            );
        });
    }

    #[test]
    fn test_batch_verify_empty_or_oversize() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            let empty = soroban_sdk::vec![&env];
            assert_eq!(
                TrustBridgeContract::batch_verify(env.clone(), admin.clone(), empty),
                Err(ContractError::InvalidBatchSize)
            );
        });
    }

    #[test]
    fn test_batch_verify_paused() {
        let env = Env::default();
        let (admin, _user, _other, contract_id) = setup(&env);

        env.mock_all_auths();
        env.as_contract(&contract_id, || {
            TrustBridgeContract::pause(env.clone(), 1).unwrap();
            let usernames = soroban_sdk::vec![&env, username(&env, "user1")];
            assert_eq!(
                TrustBridgeContract::batch_verify(env.clone(), admin.clone(), usernames),
                Err(ContractError::Paused)
            );
        });
    }
}
