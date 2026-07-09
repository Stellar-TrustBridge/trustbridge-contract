/// Audit logging for tracking contract operations and admin actions.
///
/// This module provides structured audit events for compliance and debugging,
/// including admin actions, registrations, and verification events.

use soroban_sdk::{String, Address, Env};

/// Types of audit events that can be recorded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditEventType {
    /// Contract initialization
    ContractInitialized,
    /// User registration
    UserRegistered,
    /// User removal (self or admin)
    UserRemoved,
    /// User verification
    UserVerified,
    /// Admin action
    AdminAction,
    /// Unauthorized access attempt
    UnauthorizedAttempt,
    /// Data export (for dashboard sync)
    DataExported,
}

impl AuditEventType {
    /// Get a string representation of the event type.
    pub fn as_str(self) -> &'static str {
        match self {
            AuditEventType::ContractInitialized => "CONTRACT_INITIALIZED",
            AuditEventType::UserRegistered => "USER_REGISTERED",
            AuditEventType::UserRemoved => "USER_REMOVED",
            AuditEventType::UserVerified => "USER_VERIFIED",
            AuditEventType::AdminAction => "ADMIN_ACTION",
            AuditEventType::UnauthorizedAttempt => "UNAUTHORIZED_ATTEMPT",
            AuditEventType::DataExported => "DATA_EXPORTED",
        }
    }
}

/// Structured audit log entry.
#[derive(Clone, Debug)]
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
    pub fn new(
        event_type: AuditEventType,
        timestamp: u64,
        actor: Option<Address>,
    ) -> Self {
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
    pub fn with_username(mut self, username: String) -> Self {
        self.target_username = Some(username);
        self
    }

    /// Add target address to the entry.
    pub fn with_address(mut self, address: Address) -> Self {
        self.target_address = Some(address);
        self
    }

    /// Add details to the entry.
    pub fn with_details(mut self, details: String) -> Self {
        self.details = Some(details);
        self
    }
}

/// Configuration for audit logging.
#[derive(Clone, Copy, Debug)]
pub struct AuditConfig {
    /// Whether audit logging is enabled
    pub enabled: bool,
    /// Maximum number of events to retain in memory
    pub max_events: u32,
    /// Whether to log unauthorized attempts
    pub log_unauthorized: bool,
}

impl AuditConfig {
    /// Create default audit configuration.
    pub fn default() -> Self {
        AuditConfig {
            enabled: true,
            max_events: 1000,
            log_unauthorized: true,
        }
    }

    /// Create audit configuration with custom settings.
    pub fn custom(enabled: bool, max_events: u32, log_unauthorized: bool) -> Self {
        AuditConfig {
            enabled,
            max_events,
            log_unauthorized,
        }
    }
}

/// Audit event counter for statistics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuditStats {
    pub total_events: u32,
    pub registrations: u32,
    pub removals: u32,
    pub verifications: u32,
    pub unauthorized_attempts: u32,
}

impl AuditStats {
    /// Create new empty statistics.
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
            _ => {},
        }
    }
}
