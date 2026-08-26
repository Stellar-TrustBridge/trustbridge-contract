// Helper module staged ahead of its call sites: the items below are part of the
// contract's internal toolkit and are covered by this module's own tests, but
// are not yet wired into `lib.rs`.
#![allow(dead_code)]
/// Enhanced error context and handling for TrustBridge contract.
///
/// This module provides more detailed error information and recovery hints
/// that can be used for better diagnostics and user feedback.
use crate::ContractError;

/// Detailed error information with context and recovery suggestions.
#[derive(Clone, Debug)]
pub struct ErrorContext {
    pub error: ContractError,
    pub context: &'static str,
    pub recovery_hint: &'static str,
}

impl ErrorContext {
    /// Create an error context for NotInitialized.
    pub fn not_initialized() -> Self {
        ErrorContext {
            error: ContractError::NotInitialized,
            context: "Contract has not been initialized",
            recovery_hint: "Call initialize() with an admin address first",
        }
    }

    /// Create an error context for AlreadyInitialized.
    pub fn already_initialized() -> Self {
        ErrorContext {
            error: ContractError::AlreadyInitialized,
            context: "Contract was already initialized",
            recovery_hint: "The initialize() function can only be called once",
        }
    }

    /// Create an error context for NotAuthorized.
    pub fn not_authorized(action: &'static str) -> Self {
        ErrorContext {
            error: ContractError::NotAuthorized,
            context: action,
            recovery_hint: "Ensure you're calling with the correct signing address",
        }
    }

    /// Create an error context for NotRegistered.
    pub fn not_registered(username: &'static str) -> Self {
        ErrorContext {
            error: ContractError::NotRegistered,
            context: username,
            recovery_hint: "Register the GitHub username with a Stellar address first",
        }
    }

    /// Create an error context for AlreadyVerified.
    pub fn already_verified(username: &'static str) -> Self {
        ErrorContext {
            error: ContractError::AlreadyVerified,
            context: username,
            recovery_hint: "This account is already verified; no need to verify again",
        }
    }

    /// Get a full error message with context and hint.
    pub fn full_message(&self) -> &'static str {
        self.context
    }
}

/// Result type alias for contract operations.
pub type ContractResult<T> = Result<T, ContractError>;

/// Error classification for retry logic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCategory {
    /// Error is transient and operation can be retried
    Transient,
    /// Error is permanent and operation cannot be retried
    Permanent,
    /// Error is due to invalid input
    Validation,
}

/// Classify an error for retry logic.
pub fn classify_error(error: ContractError) -> ErrorCategory {
    match error {
        ContractError::AlreadyInitialized => ErrorCategory::Permanent,
        ContractError::NotInitialized => ErrorCategory::Transient,
        ContractError::NotAuthorized => ErrorCategory::Permanent,
        ContractError::NotRegistered => ErrorCategory::Validation,
        ContractError::AlreadyVerified => ErrorCategory::Validation,
        ContractError::NotVerified => ErrorCategory::Validation,
        ContractError::Paused => ErrorCategory::Transient,
        ContractError::CooldownActive => ErrorCategory::Transient,
        ContractError::InvalidVersion => ErrorCategory::Validation,
        ContractError::InvalidRole => ErrorCategory::Validation,
        ContractError::InvalidUsername => ErrorCategory::Validation,
        ContractError::AttestationExpired => ErrorCategory::Transient,
        ContractError::UnattestedWasm => ErrorCategory::Permanent,
        ContractError::InvalidBatchSize => ErrorCategory::Validation,
        ContractError::InvalidReasonCode => ErrorCategory::Validation,
        ContractError::ZeroAddress => ErrorCategory::Validation,
        ContractError::ChallengeAlreadyActive => ErrorCategory::Validation,
        ContractError::NoChallengeActive => ErrorCategory::Validation,
        ContractError::ChallengeNotResolvable => ErrorCategory::Transient,
        ContractError::ChallengeActive => ErrorCategory::Transient,
    }
}

/// Check if an error allows retry.
pub fn is_retryable(error: ContractError) -> bool {
    matches!(classify_error(error), ErrorCategory::Transient)
}
