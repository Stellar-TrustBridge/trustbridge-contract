//! Input validation helpers for TrustBridge contract operations.
//!
//! Validation runs before authentication and before any storage write, so a
//! malformed username is rejected at the cheapest possible point. Everything
//! here works on a fixed stack buffer: the contract is `#![no_std]` and must
//! not allocate on the validation path.

// NOTE: The crate-level #![allow(dead_code)] has been removed (Issue #248).
// Each helper that is not yet wired into a call site carries its own per-item
// allow with an explanation below.

use soroban_sdk::{Address, Env, String};

/// GitHub caps usernames at 39 characters.
pub const MAX_USERNAME_LEN: u32 = 39;

/// Stellar strkey for the well-known "zero" G-address: the base32 encoding of
/// an all-zero 32-byte ed25519 public key, with a valid checksum. No private
/// key can ever exist for it, so it can never satisfy a `require_auth`.
pub const ZERO_ADDRESS_STRKEY: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";

/// True when `address` is the well-known zero/burn address.
///
/// On a live network `stellar_address.require_auth()` alone would already
/// reject this address, since nobody holds its private key. But
/// `mock_all_auths` in tests and local dev sandboxes bypasses that check
/// entirely, so a caller could otherwise register the zero address as a
/// live entry. Checking explicitly also gives dashboard and indexer
/// consumers a typed error instead of an opaque auth failure.
pub fn is_zero_address(env: &Env, address: &Address) -> bool {
    *address == Address::from_str(env, ZERO_ADDRESS_STRKEY)
}

/// Stack buffer size for username copies. Sized above `MAX_USERNAME_LEN`, so
/// an over-long username is rejected on length before it is ever read.
const USERNAME_BUF: usize = 64;
const USERNAME_BUF_LEN: usize = USERNAME_BUF;

/// Copies a username into a fixed stack buffer.
///
/// Returns `None` when the username is empty or does not fit, which callers
/// treat as a validation failure rather than truncating.
fn copy_into_buf(s: &String, buf: &mut [u8; USERNAME_BUF]) -> Option<usize> {
    let len = s.len() as usize;
    if len == 0 || len > USERNAME_BUF {
        return None;
    }
    s.copy_into_slice(&mut buf[..len]);
    Some(len)
}

/// Check if a string is empty.
///
/// Staged: not yet wired into a call site in `lib.rs` — kept so future
/// validation layers can use it without re-introducing the logic.
#[allow(dead_code)] // Issue #248: covered by tests; staged for future input-guard call sites.
pub fn is_empty(s: &String) -> bool {
    s.is_empty()
}

/// Check if a string is empty or contains only ASCII whitespace.
///
/// Staged: not yet wired into a call site in `lib.rs` — intended for a
/// display-name validation layer that is tracked but not yet shipped.
#[allow(dead_code)] // Issue #248: covered by tests; staged for display-name validation.
pub fn is_empty_or_whitespace(s: &String) -> bool {
    let len = s.len() as usize;
    if len == 0 {
        return true;
    }
    if len > USERNAME_BUF {
        // Too long to inspect on the stack, but definitively not whitespace-only
        // for any input this contract accepts.
        return false;
    }
    let mut buf = [0u8; USERNAME_BUF];
    s.copy_into_slice(&mut buf[..len]);
    buf[..len].iter().all(u8::is_ascii_whitespace)
}

/// Validate that a GitHub username follows GitHub's own rules.
///
/// Accepted:
/// - 1 to 39 characters (`MAX_USERNAME_LEN`)
/// - ASCII alphanumerics, hyphens, and underscores only
/// - first and last character alphanumeric
/// - no consecutive hyphens
///
/// Underscores are not valid on GitHub itself but are accepted here so that
/// registrations made before validation existed remain readable and removable.
/// Rejecting them would strand those records: `remove` looks the username up by
/// exact key, so a name that cannot be expressed can never be cleaned up.
///
/// ## Unicode rejection policy
///
/// GitHub usernames are ASCII-only. Any username containing a non-ASCII byte —
/// including multi-byte UTF-8 sequences for accented letters (é, ü, ñ), emoji,
/// CJK characters, or Cyrillic/Arabic/Hebrew homoglyphs — is rejected with
/// `InvalidUsername`.
///
/// The check is byte-wise: `String::len()` returns a byte count, not a Unicode
/// scalar count. Any multi-byte UTF-8 sequence has a leading byte ≥ 0x80, which
/// is not an ASCII alphanumeric (0x30–0x39, 0x41–0x5A, 0x61–0x7A), a hyphen
/// (0x2D), or an underscore (0x5F), so the per-byte character check rejects it.
/// This makes homoglyph substitution attacks (e.g. Cyrillic 'а' for ASCII 'a')
/// impossible — the bytes differ even if the glyphs look the same.
///
/// The validation path never allocates: the contract is `#![no_std]` and
/// operates on a fixed 64-byte stack buffer.
pub fn is_valid_github_username(s: &String) -> bool {
    if s.len() > MAX_USERNAME_LEN {
        return false;
    }

    let mut buf = [0u8; USERNAME_BUF];
    let len = match copy_into_buf(s, &mut buf) {
        Some(len) => len,
        None => return false,
    };
    let bytes = &buf[..len];

    // First and last characters must be alphanumeric.
    if !bytes[0].is_ascii_alphanumeric() || !bytes[len - 1].is_ascii_alphanumeric() {
        return false;
    }

    // Walk every byte: reject non-ASCII (> 0x7F), reject disallowed ASCII
    // punctuation, and reject consecutive hyphens.
    let mut prev_was_hyphen = false;
    for &b in bytes.iter() {
        // Non-ASCII byte — covers all multi-byte UTF-8 sequences.
        if !b.is_ascii() {
            return false;
        }
        // Consecutive hyphens not allowed (GitHub rule).
        if b == b'-' {
            if prev_was_hyphen {
                return false;
            }
            prev_was_hyphen = true;
        } else {
            prev_was_hyphen = false;
            // All non-hyphen characters must be alphanumeric or underscore.
            if !b.is_ascii_alphanumeric() && b != b'_' {
                return false;
            }
        }
    }

    true
}

/// Case-insensitive comparison of two usernames.
///
/// GitHub usernames are case-insensitive, so this is what an off-chain
/// verification workflow should use when matching a registration against a
/// GitHub identity. Note that storage keys are still case-*sensitive*: this
/// compares two values, it does not normalise them.
pub fn eq_ignore_ascii_case(a: &String, b: &String) -> bool {
    if a.len() != b.len() {
        return false;
    }
    if a.is_empty() {
        return true;
    }

    let mut buf_a = [0u8; USERNAME_BUF];
    let mut buf_b = [0u8; USERNAME_BUF];
    let (len_a, len_b) = match (copy_into_buf(a, &mut buf_a), copy_into_buf(b, &mut buf_b)) {
        (Some(la), Some(lb)) => (la, lb),
        // Both failed to buffer: both are either empty (already handled above)
        // or too long for the stack buffer — report unequal in that case.
        _ => return false,
    };

    buf_a[..len_a]
        .iter()
        .zip(buf_b[..len_b].iter())
        .all(|(x, y)| x.eq_ignore_ascii_case(y))
}

/// Folds a GitHub username to its canonical storage-key form.
///
/// GitHub logins are case-insensitive, so `Alice` and `alice` name the same
/// account. Every persistent storage key keyed by a username is built from
/// this canonical form (ASCII-lowercased), so a case variant of an existing
/// login can never create a second, independent record — see
/// `docs/SECURITY.md#username-case-folding`.
///
/// The fold is byte-wise ASCII-only: only bytes in `b'A'..=b'Z'` are lowered.
/// Any byte `>= 0x80` (part of a multi-byte UTF-8 sequence) is left
/// untouched, so this never changes the byte length of its input and never
/// performs a Unicode-aware case fold — GitHub usernames are ASCII-only
/// (`is_valid_github_username`), and this function does not attempt to
/// canonicalize non-ASCII input beyond leaving it as-is. Homoglyph
/// normalization is explicitly out of scope (tracked separately).
///
/// A username too long for the fixed stack buffer (longer than
/// `MAX_USERNAME_LEN` could ever be, since registration already rejects
/// those) is returned unchanged rather than folded, since it can never be a
/// valid registration key anyway.
#[must_use]
pub fn canonicalize_username(env: &Env, s: &String) -> String {
    let len = s.len() as usize;
    if len == 0 || len > USERNAME_BUF {
        return s.clone();
    }

    let mut buf = [0u8; USERNAME_BUF];
    s.copy_into_slice(&mut buf[..len]);
    buf[..len].make_ascii_lowercase();

    match core::str::from_utf8(&buf[..len]) {
        Ok(lowered) => String::from_str(env, lowered),
        // Unreachable for any input that was valid UTF-8 going in: lowercasing
        // ASCII bytes in place can never break UTF-8 validity. Kept as a safe
        // fallback rather than a panic on the no_std validation path.
        Err(_) => s.clone(),
    }
}

/// Calculate the percentage of verified contributors out of total.
///
/// Staged: not yet wired into a call site — intended for the dashboard
/// stats endpoint once a percentage field is added to `Stats`.
#[allow(dead_code)] // Issue #248: covered by tests; staged for Stats percentage field.
pub fn calculate_verification_percentage(verified: u32, total: u32) -> u32 {
    if total == 0 {
        return 0;
    }
    ((verified as u64 * 100) / (total as u64)) as u32
}

/// Generate a timestamped event ID for audit trails.
///
/// Staged: not yet wired into a call site — intended for a deduplicated
/// audit-log path that assigns unique IDs to emitted events.
#[allow(dead_code)] // Issue #248: covered by tests; staged for audit-log event-ID assignment.
pub fn generate_event_id(env: &Env, nonce: u32) -> u64 {
    let timestamp = env.ledger().timestamp();
    (timestamp << 32) | (nonce as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    fn s(env: &Env, value: &str) -> String {
        String::from_str(env, value)
    }

    // ── Basic helpers ─────────────────────────────────────────────────────────

    #[test]
    fn test_is_empty() {
        let env = Env::default();
        assert!(is_empty(&s(&env, "")));
        assert!(!is_empty(&s(&env, " ")));
        assert!(!is_empty(&s(&env, "alice")));
    }

    #[test]
    fn test_is_empty_or_whitespace() {
        let env = Env::default();
        assert!(is_empty_or_whitespace(&s(&env, "")));
        assert!(is_empty_or_whitespace(&s(&env, "   ")));
        assert!(!is_empty_or_whitespace(&s(&env, "hello")));
    }

    // ── Valid username acceptance ─────────────────────────────────────────────

    #[test]
    fn test_accepts_valid_usernames() {
        let env = Env::default();
        assert!(is_valid_github_username(&s(&env, "alice")));
        assert!(is_valid_github_username(&s(&env, "bob-smith")));
        assert!(is_valid_github_username(&s(&env, "user_123")));
        assert!(is_valid_github_username(&s(&env, "foo-bar-baz")));
        assert!(is_valid_github_username(&s(&env, "a")));
        assert!(is_valid_github_username(&s(&env, "Z")));
        assert!(is_valid_github_username(&s(&env, "9")));
    }

    #[test]
    fn test_max_length_boundary() {
        let env = Env::default();
        // 39 chars — exactly at limit, must accept
        assert!(is_valid_github_username(&s(
            &env,
            "abcdefghijklmnopqrstuvwxyz0123456789abc"
        )));
        // 40 chars — one over limit, must reject
        assert!(!is_valid_github_username(&s(
            &env,
            "abcdefghijklmnopqrstuvwxyz0123456789abcd"
        )));
    }

    // ── ASCII rejection cases ─────────────────────────────────────────────────

    #[test]
    fn test_empty_username_rejected() {
        let env = Env::default();
        assert!(!is_valid_github_username(&s(&env, "")));
    }

    #[test]
    fn test_hyphen_or_underscore_at_boundary_rejected() {
        let env = Env::default();
        assert!(!is_valid_github_username(&s(&env, "-invalid")));
        assert!(!is_valid_github_username(&s(&env, "invalid-")));
        assert!(!is_valid_github_username(&s(&env, "_leading")));
        assert!(!is_valid_github_username(&s(&env, "trailing_")));
    }

    #[test]
    fn test_disallowed_ascii_punctuation_rejected() {
        let env = Env::default();
        assert!(!is_valid_github_username(&s(&env, "a@invalid")));
        assert!(!is_valid_github_username(&s(&env, "dot.name")));
        assert!(!is_valid_github_username(&s(&env, "has space")));
        assert!(!is_valid_github_username(&s(&env, "slash/name")));
        assert!(!is_valid_github_username(&s(&env, "col:on")));
    }

    #[test]
    fn test_ascii_control_characters_rejected() {
        let env = Env::default();
        assert!(!is_valid_github_username(&s(&env, "user\x00name")));
        assert!(!is_valid_github_username(&s(&env, "user\x09name"))); // tab
        assert!(!is_valid_github_username(&s(&env, "user\x0aname"))); // newline
    }

    // ── Consecutive hyphens (Issue #70) ───────────────────────────────────────

    /// Consecutive hyphens must be rejected — this is a rule that was
    /// documented but not enforced before Wave #69.
    #[test]
    fn test_consecutive_hyphens_rejected() {
        let env = Env::default();
        assert!(!is_valid_github_username(&s(&env, "foo--bar")));
        assert!(!is_valid_github_username(&s(&env, "foo---bar")));
        assert!(!is_valid_github_username(&s(&env, "a--b")));
        // Single hyphens at any interior position remain valid
        assert!(is_valid_github_username(&s(&env, "foo-bar")));
        assert!(is_valid_github_username(&s(&env, "f-o-o")));
    }

    // ── Unicode rejection policy (Wave #69 / Issue #70) ──────────────────────
    //
    // GitHub usernames are ASCII-only. Every non-ASCII byte — whether a lone
    // high byte or part of a multi-byte UTF-8 sequence — must be rejected.
    //
    // The on-chain check is byte-wise: `is_ascii()` returns false for any byte
    // > 0x7F, which covers every non-ASCII codepoint regardless of encoding.
    // This prevents homoglyph attacks where a Cyrillic 'а' (U+0430) is
    // substituted for ASCII 'a' (U+0061) — the byte sequences differ even
    // though the glyphs may look identical.

    /// Latin-extended characters (é, ü, ñ) must be rejected.
    ///
    /// `café` = [0x63, 0x61, 0x66, 0xC3, 0xA9] — 0xC3 is the leading byte of
    /// U+00E9 LATIN SMALL LETTER E WITH ACUTE and is not ASCII.
    #[test]
    fn test_unicode_latin_extended_rejected() {
        let env = Env::default();
        assert!(
            !is_valid_github_username(&s(&env, "caf\u{e9}")),
            "café must be rejected (U+00E9 é is non-ASCII)"
        );
        assert!(
            !is_valid_github_username(&s(&env, "na\u{ef}ve")),
            "naïve must be rejected (U+00EF ï is non-ASCII)"
        );
        assert!(
            !is_valid_github_username(&s(&env, "jalape\u{f1}o")),
            "jalapeño must be rejected (U+00F1 ñ is non-ASCII)"
        );
    }

    /// Emoji must be rejected.
    ///
    /// Emoji are encoded as 3- or 4-byte UTF-8 sequences whose leading byte is
    /// ≥ 0xE0 or 0xF0.  Neither qualifies as ASCII.
    #[test]
    fn test_unicode_emoji_rejected() {
        let env = Env::default();
        // U+1F600 GRINNING FACE — 4-byte sequence [0xF0, 0x9F, 0x98, 0x80]
        assert!(
            !is_valid_github_username(&s(&env, "user\u{1f600}")),
            "emoji suffix must be rejected"
        );
        // U+2764 HEAVY BLACK HEART — 3-byte sequence [0xE2, 0x9D, 0xA4]
        assert!(
            !is_valid_github_username(&s(&env, "user\u{2764}")),
            "heart emoji must be rejected"
        );
        // Emoji-only username
        assert!(
            !is_valid_github_username(&s(&env, "\u{1f600}\u{1f600}")),
            "emoji-only username must be rejected"
        );
    }

    /// CJK (Chinese, Japanese, Korean) characters must be rejected.
    ///
    /// CJK codepoints start at U+4E00 and are encoded as 3-byte UTF-8
    /// sequences — leading bytes 0xE4–0xE9.
    #[test]
    fn test_unicode_cjk_rejected() {
        let env = Env::default();
        // U+4E2D CJK UNIFIED IDEOGRAPH (中) — [0xE4, 0xB8, 0xAD]
        assert!(
            !is_valid_github_username(&s(&env, "\u{4e2d}user")),
            "CJK prefix must be rejected"
        );
        // U+3042 HIRAGANA LETTER A (あ) — [0xE3, 0x81, 0x82]
        assert!(
            !is_valid_github_username(&s(&env, "\u{3042}user")),
            "Hiragana prefix must be rejected"
        );
        // CJK-only username
        assert!(
            !is_valid_github_username(&s(&env, "\u{4e2d}\u{6587}")),
            "CJK-only username must be rejected"
        );
    }

    /// RTL script characters (Arabic, Hebrew) must be rejected.
    ///
    /// These are 2-byte UTF-8 sequences in the range 0xD5–0xDB.
    #[test]
    fn test_unicode_arabic_and_rtl_rejected() {
        let env = Env::default();
        // U+0645 ARABIC LETTER MEEM (م) — [0xD9, 0x85]
        assert!(
            !is_valid_github_username(&s(&env, "\u{0645}user")),
            "Arabic prefix must be rejected"
        );
        // U+05D0 HEBREW LETTER ALEF (א) — [0xD7, 0x90]
        assert!(
            !is_valid_github_username(&s(&env, "\u{05d0}user")),
            "Hebrew prefix must be rejected"
        );
    }

    /// Cyrillic and Greek homoglyph attacks must be rejected.
    ///
    /// These are among the most dangerous Unicode spoofing vectors: characters
    /// that look visually identical (or nearly identical) to ASCII letters but
    /// occupy different codepoints and byte sequences.  Because validation is
    /// byte-wise rather than glyph-wise, even a perfect-looking lookalike is
    /// caught by the `is_ascii()` gate.
    #[test]
    fn test_unicode_homoglyph_attack_rejected() {
        let env = Env::default();
        // U+0430 CYRILLIC SMALL LETTER A (а) — looks like ASCII 'a', encoded [0xD0, 0xB0]
        assert!(
            !is_valid_github_username(&s(&env, "\u{0430}lice")),
            "Cyrillic 'a' homoglyph prefix must be rejected"
        );
        // U+03BF GREEK SMALL LETTER OMICRON (ο) — looks like ASCII 'o', encoded [0xCF, 0xBF]
        assert!(
            !is_valid_github_username(&s(&env, "b\u{03bf}b")),
            "Greek omicron homoglyph must be rejected"
        );
        // U+0435 CYRILLIC SMALL LETTER IE (е) — looks like ASCII 'e', encoded [0xD0, 0xB5]
        assert!(
            !is_valid_github_username(&s(&env, "al\u{0435}x")),
            "Cyrillic 'e' homoglyph must be rejected"
        );
        // U+0441 CYRILLIC SMALL LETTER ES (с) — looks like ASCII 'c', encoded [0xD1, 0x81]
        assert!(
            !is_valid_github_username(&s(&env, "\u{0441}arol")),
            "Cyrillic 'c' homoglyph prefix must be rejected"
        );
    }

    /// Usernames composed entirely of non-ASCII characters must be rejected.
    #[test]
    fn test_unicode_all_non_ascii_rejected() {
        let env = Env::default();
        // Entirely Cyrillic
        assert!(
            !is_valid_github_username(&s(&env, "\u{0430}\u{043b}\u{0438}\u{0441}\u{0430}")),
            "all-Cyrillic username must be rejected"
        );
        // Entirely CJK
        assert!(
            !is_valid_github_username(&s(&env, "\u{4e2d}\u{6587}")),
            "all-CJK username must be rejected"
        );
    }

    /// Non-ASCII characters embedded anywhere in an otherwise valid-looking
    /// ASCII username must still be rejected.
    #[test]
    fn test_unicode_embedded_at_any_position_rejected() {
        let env = Env::default();
        // Non-ASCII in the middle
        assert!(!is_valid_github_username(&s(&env, "al\u{00e9}ce")));
        // Non-ASCII near the end (U+00E9 encodes as [0xC3, 0xA9]; 0xC3 is > 0x7F)
        // The trailing 0xA9 is also non-ASCII, but first byte is caught first.
        assert!(!is_valid_github_username(&s(&env, "alice\u{00e9}")));
        // Non-ASCII at the start
        assert!(!is_valid_github_username(&s(&env, "\u{00e9}alice")));
    }

    /// A lone high byte (invalid UTF-8 / raw non-ASCII) must be rejected.
    ///
    /// This guards against crafted byte sequences that might not be valid
    /// Unicode but still contain bytes above 0x7F.
    #[test]
    fn test_raw_high_byte_rejected() {
        let env = Env::default();
        // The soroban_sdk String::from_str takes a &str (valid UTF-8), so we
        // use the closest single-byte non-ASCII codepoint U+0080 PADDING CHAR
        // which encodes as [0xC2, 0x80] — both bytes are > 0x7F.
        assert!(!is_valid_github_username(&s(&env, "user\u{0080}name")));
        // U+00FF LATIN SMALL LETTER Y WITH DIAERESIS — [0xC3, 0xBF]
        assert!(!is_valid_github_username(&s(&env, "user\u{00ff}")));
    }

    // ── Confirm pure-ASCII valid cases still pass after policy hardening ───────

    /// Regression: adding the Unicode gate must not break any currently-valid
    /// ASCII username shape.
    #[test]
    fn test_valid_ascii_still_accepted_after_unicode_hardening() {
        let env = Env::default();
        let valid = [
            "octocat",
            "alice",
            "bob123",
            "user-name",
            "user_name",
            "a1b2c3",
            "ALLCAPS",
            "MixedCase",
            "x",
            "a-b",
            "foo-bar-baz",
            "abc_def_123",
        ];
        for name in &valid {
            assert!(
                is_valid_github_username(&s(&env, name)),
                "{name} must still be accepted after unicode hardening"
            );
        }
    }

    // ── Case-insensitive comparison ───────────────────────────────────────────

    #[test]
    fn test_eq_ignore_ascii_case() {
        let env = Env::default();
        assert!(eq_ignore_ascii_case(&s(&env, "Alice"), &s(&env, "alice")));
        assert!(eq_ignore_ascii_case(&s(&env, "BOB-1"), &s(&env, "bob-1")));
        assert!(!eq_ignore_ascii_case(&s(&env, "alice"), &s(&env, "bob")));
        assert!(!eq_ignore_ascii_case(&s(&env, "alice"), &s(&env, "alice1")));
    }

    // ── Username case-folding (Issue #194) ────────────────────────────────────

    #[test]
    fn test_canonicalize_username_lowercases() {
        let env = Env::default();
        assert_eq!(
            canonicalize_username(&env, &s(&env, "Alice")),
            s(&env, "alice")
        );
        assert_eq!(
            canonicalize_username(&env, &s(&env, "OCTOCAT")),
            s(&env, "octocat")
        );
        assert_eq!(
            canonicalize_username(&env, &s(&env, "Bob-Smith_42")),
            s(&env, "bob-smith_42")
        );
    }

    #[test]
    fn test_canonicalize_username_already_lower_is_unchanged() {
        let env = Env::default();
        assert_eq!(
            canonicalize_username(&env, &s(&env, "octocat")),
            s(&env, "octocat")
        );
    }

    #[test]
    fn test_canonicalize_username_idempotent() {
        let env = Env::default();
        let once = canonicalize_username(&env, &s(&env, "MixedCase"));
        let twice = canonicalize_username(&env, &once);
        assert_eq!(once, twice);
    }

    #[test]
    fn test_canonicalize_username_case_variants_collide() {
        let env = Env::default();
        let variants = ["alice", "Alice", "ALICE", "aLiCe"];
        let canonical = canonicalize_username(&env, &s(&env, variants[0]));
        for v in &variants {
            assert_eq!(
                canonicalize_username(&env, &s(&env, v)),
                canonical,
                "{v} must fold to the same canonical key as {}",
                variants[0]
            );
        }
    }

    #[test]
    fn test_canonicalize_username_never_changes_byte_length() {
        let env = Env::default();
        for name in ["a", "Z", "MixedCase123", "foo-BAR_baz", "OCTOCAT"] {
            let original = s(&env, name);
            let folded = canonicalize_username(&env, &original);
            assert_eq!(
                folded.len(),
                original.len(),
                "ASCII case-folding must never change byte length ({name})"
            );
        }
    }

    #[test]
    fn test_canonicalize_username_empty_is_unchanged() {
        let env = Env::default();
        assert_eq!(canonicalize_username(&env, &s(&env, "")), s(&env, ""));
    }

    // ── Percentage helper ─────────────────────────────────────────────────────

    #[test]
    fn test_calculate_verification_percentage() {
        assert_eq!(calculate_verification_percentage(0, 100), 0);
        assert_eq!(calculate_verification_percentage(50, 100), 50);
        assert_eq!(calculate_verification_percentage(100, 100), 100);
        assert_eq!(calculate_verification_percentage(1, 3), 33);
        assert_eq!(calculate_verification_percentage(10, 0), 0);
    }

    #[test]
    fn test_percentage_does_not_overflow_at_u32_max() {
        assert_eq!(calculate_verification_percentage(u32::MAX, u32::MAX), 100);
    }

    // ── Event ID helper ───────────────────────────────────────────────────────

    /// `generate_event_id` packs the ledger timestamp into the high 32 bits and
    /// the nonce into the low 32 bits.
    #[test]
    fn test_generate_event_id_encodes_timestamp_and_nonce() {
        let env = Env::default();
        env.ledger().set_timestamp(42);
        let id = generate_event_id(&env, 7);
        assert_eq!(id >> 32, 42, "high 32 bits must be the ledger timestamp");
        assert_eq!(id & 0xFFFF_FFFF, 7, "low 32 bits must be the nonce");
    }

    #[test]
    fn test_generate_event_id_nonce_zero() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000_000);
        let id = generate_event_id(&env, 0);
        assert_eq!(id, 1_000_000u64 << 32);
    }

    #[test]
    fn test_generate_event_id_different_nonces_differ() {
        let env = Env::default();
        env.ledger().set_timestamp(100);
        let id_a = generate_event_id(&env, 1);
        let id_b = generate_event_id(&env, 2);
        assert_ne!(id_a, id_b);
    }
}
