/// Audit logging for tracking contract operations and admin actions.
///
/// This module provides structured audit events for compliance and debugging,
/// including admin actions, registrations, and verification events.
use soroban_sdk::{contracttype, Address, String};

/// Types of audit events that can be recorded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[contracttype]
#[repr(u32)]
pub enum AuditEventType {
    // Codes are append-only for the same reason `ContractError`'s are: an
    // indexer stores the number, so renumbering silently re-labels history it
    // has already written. Add new variants at the end.
    /// Contract initialization
    ContractInitialized = 1,
    /// User registration
    UserRegistered = 2,
    /// User removal (self or admin)
    UserRemoved = 3,
    /// User verification
    UserVerified = 4,
    /// Admin action
    AdminAction = 5,
    /// Unauthorized access attempt
    UnauthorizedAttempt = 6,
    /// Data export (for dashboard sync)
    DataExported = 7,
    /// A verification was revoked (Issue #397).
    VerificationRevoked = 8,
    /// A registration was moved to a new username (Issue #397).
    UserRenamed = 9,
    /// The contract was paused, by `pause`, `set_paused` or `emergency_pause`.
    ContractPaused = 10,
    /// The contract was unpaused.
    ContractUnpaused = 11,
    /// A role was granted, activated, or changed.
    RoleChanged = 12,
    /// The contract WASM was replaced.
    ContractUpgraded = 13,
    /// The audit configuration was mutated (Issue #412).
    AuditConfigChanged = 14,
}

impl AuditEventType {
    /// Get a string representation of the event type.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AuditEventType::ContractInitialized => "CONTRACT_INITIALIZED",
            AuditEventType::UserRegistered => "USER_REGISTERED",
            AuditEventType::UserRemoved => "USER_REMOVED",
            AuditEventType::UserVerified => "USER_VERIFIED",
            AuditEventType::AdminAction => "ADMIN_ACTION",
            AuditEventType::UnauthorizedAttempt => "UNAUTHORIZED_ATTEMPT",
            AuditEventType::DataExported => "DATA_EXPORTED",
            AuditEventType::VerificationRevoked => "VERIFICATION_REVOKED",
            AuditEventType::UserRenamed => "USER_RENAMED",
            AuditEventType::ContractPaused => "CONTRACT_PAUSED",
            AuditEventType::ContractUnpaused => "CONTRACT_UNPAUSED",
            AuditEventType::RoleChanged => "ROLE_CHANGED",
            AuditEventType::ContractUpgraded => "CONTRACT_UPGRADED",
            AuditEventType::AuditConfigChanged => "AUDIT_CONFIG_CHANGED",
        }
    }
}

/// Structured audit log entry.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct AuditLogEntry {
    pub event_type: AuditEventType,
    pub timestamp: u64,
    pub actor: Option<Address>,
    pub target_username: Option<String>,
    pub target_address: Option<Address>,
    pub details: Option<String>,
}

impl AuditLogEntry {
    /// Create a new audit log entry.
    #[must_use]
    pub fn new(event_type: AuditEventType, timestamp: u64, actor: Option<Address>) -> Self {
        AuditLogEntry {
            event_type,
            timestamp,
            actor,
            target_username: None,
            target_address: None,
            details: None,
        }
    }

    /// Add target username to the entry.
    #[must_use]
    pub fn with_username(mut self, username: String) -> Self {
        self.target_username = Some(username);
        self
    }

    /// Add target address to the entry.
    #[must_use]
    pub fn with_address(mut self, address: Address) -> Self {
        self.target_address = Some(address);
        self
    }

    /// Add details to the entry.
    #[must_use]
    pub fn with_details(mut self, details: String) -> Self {
        self.details = Some(details);
        self
    }
}

/// Configuration for audit logging.
///
/// # Fields and defaults
///
/// The contract stores a single `AuditConfig` instance. When no explicit
/// configuration has been written, [`AuditConfig::default`] is used:
///
/// | Field              | Type   | Default | Meaning                                                        |
/// |--------------------|--------|---------|----------------------------------------------------------------|
/// | `enabled`          | `bool` | `true`  | Whether audit logging is enabled.                              |
/// | `max_events`       | `u32`  | `1000`  | Maximum number of events retained in memory.                   |
/// | `log_unauthorized` | `bool` | `true`  | Whether unauthorized access attempts are recorded.             |
///
/// # Mutation authorization
///
/// `AuditConfig` may only be mutated by an authorized admin. Callers must
/// authenticate as the contract admin (or a role holding the audit-admin
/// permission) before any field is changed; see [`AuditConfig::assert_can_mutate`].
/// Unauthorized attempts are rejected and, when `log_unauthorized` is set,
/// recorded as an [`AuditEventType::UnauthorizedAttempt`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[contracttype]
pub struct AuditConfig {
    /// Whether audit logging is enabled. Default: `true`.
    pub enabled: bool,
    /// Maximum number of events to retain in memory. Default: `1000`.
    pub max_events: u32,
    /// Whether to log unauthorized attempts. Default: `true`.
    pub log_unauthorized: bool,
}

impl Default for AuditConfig {
    fn default() -> Self {
        AuditConfig {
            enabled: true,
            max_events: 1000,
            log_unauthorized: true,
        }
    }
}

impl AuditConfig {
    /// Create audit configuration with custom settings.
    #[must_use]
    pub fn custom(enabled: bool, max_events: u32, log_unauthorized: bool) -> Self {
        AuditConfig {
            enabled,
            max_events,
            log_unauthorized,
        }
    }

    /// Enforce that `caller` is authorized to mutate the audit configuration.
    ///
    /// Only the contract admin may mutate `AuditConfig`. This must be called
    /// before any field is changed so that unauthorized callers are rejected
    /// and cannot weaken the audit trail.
    ///
    /// # Panics
    ///
    /// Panics if `caller` is not the authorized admin.
    pub fn assert_can_mutate(caller: &Address, admin: &Address) {
        if caller != admin {
            panic!("unauthorized: only admin may mutate AuditConfig");
        }
    }
}

/// Audit event counter for statistics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[contracttype]
pub struct AuditStats {
    pub total_events: u32,
    pub registrations: u32,
    pub removals: u32,
    pub verifications: u32,
    pub unauthorized_attempts: u32,
}

impl Default for AuditStats {
    fn default() -> Self {
        Self::new()
    }
}

impl AuditStats {
    /// Create new empty statistics.
    #[must_use]
    pub fn new() -> Self {
        AuditStats {
            total_events: 0,
            registrations: 0,
            removals: 0,
            verifications: 0,
            unauthorized_attempts: 0,
        }
    }

    /// Record an event in statistics.
    pub fn record_event(&mut self, event_type: AuditEventType) {
        self.total_events += 1;
        match event_type {
            AuditEventType::UserRegistered => self.registrations += 1,
            AuditEventType::UserRemoved => self.removals += 1,
            AuditEventType::UserVerified => self.verifications += 1,
            AuditEventType::UnauthorizedAttempt => self.unauthorized_attempts += 1,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    #[test]
    fn default_config_documents_expected_values() {
        let config = AuditConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_events, 1000);
        assert!(config.log_unauthorized);
    }

    #[test]
    fn admin_can_mutate_config() {
        let env = Env::default();
        let admin = Address::generate(&env);
        // Should not panic for the authorized admin.
        AuditConfig::assert_can_mutate(&admin, &admin);
    }

    #[test]
    #[should_panic(expected = "unauthorized")]
    fn unauthorized_caller_cannot_mutate_config() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let attacker = Address::generate(&env);
        AuditConfig::assert_can_mutate(&attacker, &admin);
    }
}
