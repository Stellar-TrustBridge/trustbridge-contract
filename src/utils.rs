/// Utility functions for TrustBridge contract operations.
///
/// This module provides helper functions for common contract operations,
/// string manipulation, and validation.

use soroban_sdk::{String, Env};

/// Check if a string is empty or contains only whitespace.
pub fn is_empty_or_whitespace(s: &String) -> bool {
    s.as_bytes().iter().all(|b| b.is_ascii_whitespace())
}

/// Convert a string to lowercase for case-insensitive comparisons.
pub fn to_lowercase_bytes(s: &String) -> Vec<u8> {
    s.as_bytes()
        .iter()
        .map(|b| {
            if b.is_ascii_uppercase() {
                b.to_ascii_lowercase()
            } else {
                *b
            }
        })
        .collect()
}

/// Validate that a GitHub username follows basic rules.
/// GitHub usernames must be 1-39 characters, alphanumeric with hyphens/underscores.
pub fn is_valid_github_username(s: &String) -> bool {
    let bytes = s.as_bytes();

    // Length check: 1-39 characters
    if bytes.len() < 1 || bytes.len() > 39 {
        return false;
    }

    // First character must be alphanumeric
    if !bytes[0].is_ascii_alphanumeric() {
        return false;
    }

    // Last character must be alphanumeric
    if !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
        return false;
    }

    // All characters must be alphanumeric, hyphen, or underscore
    bytes.iter().all(|b| {
        b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_'
    })
}

/// Calculate the percentage of verified contributors out of total.
pub fn calculate_verification_percentage(verified: u32, total: u32) -> u32 {
    if total == 0 {
        return 0;
    }
    ((verified as u64 * 100) / (total as u64)) as u32
}

/// Generate a timestamped event ID for audit trails.
pub fn generate_event_id(env: &Env, nonce: u32) -> u64 {
    let timestamp = env.ledger().timestamp();
    ((timestamp << 32) | (nonce as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_empty_or_whitespace() {
        let empty = String::from_str(&env, "");
        let whitespace = String::from_str(&env, "   ");
        let valid = String::from_str(&env, "hello");

        assert!(is_empty_or_whitespace(&empty));
        assert!(is_empty_or_whitespace(&whitespace));
        assert!(!is_empty_or_whitespace(&valid));
    }

    #[test]
    fn test_is_valid_github_username() {
        let valid1 = String::from_str(&env, "alice");
        let valid2 = String::from_str(&env, "bob-smith");
        let valid3 = String::from_str(&env, "user_123");
        let invalid1 = String::from_str(&env, "-invalid");
        let invalid2 = String::from_str(&env, "invalid-");
        let invalid3 = String::from_str(&env, "a@invalid");

        assert!(is_valid_github_username(&valid1));
        assert!(is_valid_github_username(&valid2));
        assert!(is_valid_github_username(&valid3));
        assert!(!is_valid_github_username(&invalid1));
        assert!(!is_valid_github_username(&invalid2));
        assert!(!is_valid_github_username(&invalid3));
    }

    #[test]
    fn test_calculate_verification_percentage() {
        assert_eq!(calculate_verification_percentage(0, 100), 0);
        assert_eq!(calculate_verification_percentage(50, 100), 50);
        assert_eq!(calculate_verification_percentage(100, 100), 100);
        assert_eq!(calculate_verification_percentage(1, 3), 33);
        assert_eq!(calculate_verification_percentage(10, 0), 0);
    }
}
