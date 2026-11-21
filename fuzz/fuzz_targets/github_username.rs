#![no_main]

//! Fuzz target for `utils::is_valid_github_username`.
//!
//! Feeds structured UTF-8 inputs (arbitrary byte slices interpreted as strings)
//! into the validator and asserts two invariants:
//!
//! 1. **No panics** — the validator must handle all byte patterns without
//!    trapping or unwinding.
//! 2. **Non-ASCII rejection** — any input containing a byte ≥ 0x80 must be
//!    rejected.  GitHub usernames are ASCII-only; multi-byte UTF-8 sequences
//!    for accented letters, emoji, or homoglyph Cyrillic/Arabic/Hebrew
//!    characters must never be accepted.
//!
//! Run with `cargo +nightly fuzz run github_username` from the workspace root.
//! See `docs/CONTRIBUTING.md` for the nightly-toolchain requirement.

use libfuzzer_sys::fuzz_target;
use soroban_sdk::{Env, String};
use trustbridge_contract::utils::is_valid_github_username;

fuzz_target!(|data: &[u8]| {
    let env = Env::default();

    // Attempt to interpret the raw bytes as UTF-8; non-UTF-8 sequences can't
    // form a soroban `String`, so we skip them — they're covered by the
    // separate `username_validate` target which accepts the raw rejection path.
    let text = match core::str::from_utf8(data) {
        Ok(s) => s,
        Err(_) => return,
    };

    let s = match String::from_str(&env, text) {
        Ok(s) => s,
        Err(_) => return,
    };

    let accepted = is_valid_github_username(&s);

    // Invariant: any byte ≥ 0x80 must be rejected — non-ASCII is not allowed
    // in GitHub usernames and homoglyph substitutions must be impossible.
    if data.iter().any(|&b| b >= 0x80) {
        assert!(
            !accepted,
            "non-ASCII input must never be accepted: {:?}",
            text
        );
    }

    // Length invariant: accepted inputs must not exceed MAX_USERNAME_LEN (39).
    if accepted {
        assert!(
            s.len() <= trustbridge_contract::utils::MAX_USERNAME_LEN,
            "accepted username exceeds maximum length: {:?}",
            text
        );
    }
});
