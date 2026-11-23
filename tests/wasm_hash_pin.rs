//! Automated tests for wasm-hash.pin format, verification logic, and strict mode (Issue #759).
//!
//! Validates:
//! 1. Happy path: strict mode (`WASM_HASH_STRICT=1`) passes when actual hash matches pinned hash.
//! 2. Happy path: permissive mode (`WASM_HASH_STRICT=0`) allows `PLACEHOLDER` with a warning.
//! 3. Failure path 1: strict mode (`WASM_HASH_STRICT=1`) fails closed when the pin is `PLACEHOLDER`.
//! 4. Failure path 2: hash mismatch fails closed with error naming expected and actual hashes.
//! 5. Failure path 3: malformed pin formats (empty, invalid hex length, non-hex characters).
//! 6. Pin rotation logic: rewriting `PLACEHOLDER` or old hash updates the pin cleanly.
//! 7. Repository pin consistency: the checked-in `wasm-hash.pin` satisfies all structural invariants.

#![cfg(test)]

const CHECKED_IN_PIN: &str = include_str!("../wasm-hash.pin");

/// Parse the pinned hash from a wasm-hash.pin file content, ignoring comments and whitespace.
fn parse_pin_file(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return Some(trimmed.to_string());
    }
    None
}

/// Verification result matching Makefile's wasm-hash-pin gate.
#[derive(Debug, PartialEq, Eq)]
enum PinCheckResult {
    OkMatch,
    WarnPlaceholder,
    ErrStrictPlaceholder,
    ErrMismatch { expected: String, actual: String },
    ErrMissingPin,
    ErrMalformedPin(String),
}

/// Pure simulator of Makefile's wasm-hash-pin logic.
fn verify_pin(actual_hash: &str, pin_content: &str, strict: bool) -> PinCheckResult {
    let pinned = match parse_pin_file(pin_content) {
        Some(p) => p,
        None => return PinCheckResult::ErrMissingPin,
    };

    if pinned == "PLACEHOLDER" {
        if strict {
            PinCheckResult::ErrStrictPlaceholder
        } else {
            PinCheckResult::WarnPlaceholder
        }
    } else {
        // Enforce 64-char hex format
        if pinned.len() != 64 || !pinned.chars().all(|c| c.is_ascii_hexdigit()) {
            return PinCheckResult::ErrMalformedPin(pinned);
        }
        if actual_hash == pinned {
            PinCheckResult::OkMatch
        } else {
            PinCheckResult::ErrMismatch {
                expected: pinned,
                actual: actual_hash.to_string(),
            }
        }
    }
}

/// Pure simulator of Makefile's wasm-hash-update sed logic.
fn rotate_pin(current_pin_content: &str, new_hash: &str) -> String {
    let mut out = String::new();
    for line in current_pin_content.lines() {
        let trimmed = line.trim();
        if trimmed == "PLACEHOLDER"
            || (trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()))
        {
            out.push_str(new_hash);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

// ── Happy path tests ──────────────────────────────────────────────────────────

#[test]
fn test_wasm_hash_pin_matching_hash_strict_mode_passes() {
    let hash = "da94f06adb42560232352ba1b88ab5f3dd9275b684a80056883630ab9a45004d";
    let pin_content = format!("# Pinned hash\n{}\n", hash);

    assert_eq!(
        verify_pin(hash, &pin_content, true),
        PinCheckResult::OkMatch,
        "matching hash must pass in strict mode"
    );

    assert_eq!(
        verify_pin(hash, &pin_content, false),
        PinCheckResult::OkMatch,
        "matching hash must pass in permissive mode"
    );
}

#[test]
fn test_wasm_hash_pin_placeholder_permissive_mode_passes() {
    let hash = "da94f06adb42560232352ba1b88ab5f3dd9275b684a80056883630ab9a45004d";
    let pin_content = "# Placeholder\nPLACEHOLDER\n";

    assert_eq!(
        verify_pin(hash, pin_content, false),
        PinCheckResult::WarnPlaceholder,
        "permissive mode (WASM_HASH_STRICT=0) must allow PLACEHOLDER with warning"
    );
}

// ── Failure path tests ────────────────────────────────────────────────────────

#[test]
fn test_wasm_hash_pin_placeholder_strict_mode_fails() {
    let hash = "da94f06adb42560232352ba1b88ab5f3dd9275b684a80056883630ab9a45004d";
    let pin_content = "# Placeholder\nPLACEHOLDER\n";

    assert_eq!(
        verify_pin(hash, pin_content, true),
        PinCheckResult::ErrStrictPlaceholder,
        "strict mode (WASM_HASH_STRICT=1) must fail closed when pin contains PLACEHOLDER"
    );
}

#[test]
fn test_wasm_hash_pin_hash_mismatch_fails() {
    let actual_hash = "1111111111111111111111111111111111111111111111111111111111111111";
    let pinned_hash = "2222222222222222222222222222222222222222222222222222222222222222";
    let pin_content = format!("# Pinned hash\n{}\n", pinned_hash);

    assert_eq!(
        verify_pin(actual_hash, &pin_content, true),
        PinCheckResult::ErrMismatch {
            expected: pinned_hash.to_string(),
            actual: actual_hash.to_string(),
        },
        "hash mismatch must fail closed with expected and actual hashes in strict mode"
    );

    assert_eq!(
        verify_pin(actual_hash, &pin_content, false),
        PinCheckResult::ErrMismatch {
            expected: pinned_hash.to_string(),
            actual: actual_hash.to_string(),
        },
        "hash mismatch must also fail closed in permissive mode"
    );
}

#[test]
fn test_wasm_hash_pin_malformed_pin_fails() {
    let hash = "da94f06adb42560232352ba1b88ab5f3dd9275b684a80056883630ab9a45004d";

    // Empty pin file
    assert_eq!(
        verify_pin(hash, "", true),
        PinCheckResult::ErrMissingPin,
        "empty pin file must fail as missing pin"
    );

    // Comments only
    assert_eq!(
        verify_pin(hash, "# Only comments\n# Second comment\n", true),
        PinCheckResult::ErrMissingPin,
        "comment-only pin file must fail as missing pin"
    );

    // Truncated hash (< 64 chars)
    let truncated = "da94f06a";
    assert_eq!(
        verify_pin(hash, truncated, true),
        PinCheckResult::ErrMalformedPin(truncated.to_string()),
        "truncated hash must fail as malformed"
    );

    // Non-hex characters
    let non_hex = "da94f06adb42560232352ba1b88ab5f3dd9275b684a80056883630ab9a4500ZZ";
    assert_eq!(
        verify_pin(hash, non_hex, true),
        PinCheckResult::ErrMalformedPin(non_hex.to_string()),
        "non-hex characters must fail as malformed"
    );
}

// ── Pin rotation workflow ─────────────────────────────────────────────────────

#[test]
fn test_wasm_hash_pin_rotation_workflow() {
    let initial_content = "# Initial\nPLACEHOLDER\n";
    let new_hash = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

    // 1. Before rotation: strict check fails on PLACEHOLDER
    assert_eq!(
        verify_pin(new_hash, initial_content, true),
        PinCheckResult::ErrStrictPlaceholder
    );

    // 2. Rotate pin
    let rotated = rotate_pin(initial_content, new_hash);

    // 3. After rotation: strict check passes
    assert_eq!(
        verify_pin(new_hash, &rotated, true),
        PinCheckResult::OkMatch,
        "strict check must succeed immediately following pin rotation"
    );

    // 4. Secondary rotation (updating an existing hash)
    let next_hash = "9999999999999999999999999999999999999999999999999999999999999999";
    let second_rotated = rotate_pin(&rotated, next_hash);
    assert_eq!(
        verify_pin(next_hash, &second_rotated, true),
        PinCheckResult::OkMatch,
        "secondary pin rotation must update cleanly"
    );
}

// ── Repository sanity check ───────────────────────────────────────────────────

#[test]
fn test_checked_in_wasm_hash_pin_format() {
    let pinned = parse_pin_file(CHECKED_IN_PIN).expect("wasm-hash.pin must contain a valid entry");

    // The checked in pin must either be PLACEHOLDER or a valid 64-character lowercase hex string
    if pinned != "PLACEHOLDER" {
        assert_eq!(
            pinned.len(),
            64,
            "checked-in wasm-hash.pin must be exactly 64 characters"
        );
        assert!(
            pinned
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "checked-in wasm-hash.pin must be lowercase hex"
        );
    }
}
