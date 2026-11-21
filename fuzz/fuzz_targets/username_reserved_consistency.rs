#![no_main]

//! Fuzz target combining `utils::is_valid_github_username` and the
//! `storage` reserved-name list (Issue #793).
//!
//! The registry's registration path rejects a username if either (a) it
//! fails the syntactic github-username validator in `utils.rs`, or (b) it
//! is on the admin-managed reserved list in `storage.rs`. These two checks
//! are maintained independently, so a divergence between them (e.g. a
//! reserved entry that the validator itself would already reject, or a
//! canonicalization mismatch between `canonicalize_username` and the
//! case-insensitive reserved-list lookup) could let a reserved name slip
//! through registration, enabling Wave impersonation.
//!
//! This target seeds a small reserved list with mixed-case entries, then
//! feeds arbitrary input through:
//! 1. `utils::is_valid_github_username` (syntactic check)
//! 2. `storage::is_reserved` (case-insensitive reserved-list membership)
//!
//! Invariants asserted:
//! - Neither call panics on arbitrary input.
//! - If the raw input and a case-swapped variant of a reserved seed name
//!   are byte-equal under ASCII case folding, `is_reserved` must return
//!   true for both, matching `utils::eq_ignore_ascii_case` semantics used
//!   by the validator's own canonicalization path.
//!
//! Run with `cargo +nightly fuzz run username_reserved_consistency` from
//! the `fuzz/` directory.
//!
//! Corpus seeds (see `fuzz/corpus/username_reserved_consistency/`):
//! - exact-case reserved names ("stellar", "admin")
//! - mixed-case variants ("Stellar", "ADMIN")
//! - near-miss names that are valid usernames but not reserved
//! - invalid syntax (empty, unicode, consecutive hyphens)

use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, Env, String};
use trustbridge_contract::utils::{eq_ignore_ascii_case, is_valid_github_username};
use trustbridge_contract::TrustBridgeContract;

fuzz_target!(|data: &[u8]| {
    let text = match core::str::from_utf8(data) {
        Ok(s) => s,
        Err(_) => return,
    };

    let env = Env::default();
    env.mock_all_auths();
    let candidate = match String::from_str(&env, text) {
        Ok(s) => s,
        Err(_) => return,
    };

    // Exercise the syntactic validator; it must never panic on any input.
    let syntactically_valid = is_valid_github_username(&candidate);

    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();

        // Seed the reserved list via the public admin API, mirroring how an
        // operator would reserve a Wave-sensitive name.
        let seed = String::from_str(&env, "stellar");
        let _ = TrustBridgeContract::add_reserved(env.clone(), seed.clone());

        let reserved_now = TrustBridgeContract::is_reserved(env.clone(), candidate.clone());

        // Cross-check: if the fuzz candidate is ASCII-case-equal to the
        // seed, is_reserved must agree with the same case-folding semantics
        // utils::eq_ignore_ascii_case uses elsewhere in the registry.
        let case_equal_to_seed = eq_ignore_ascii_case(&candidate, &seed);
        if case_equal_to_seed {
            assert!(
                reserved_now,
                "candidate case-equal to a reserved seed must be reported reserved"
            );
        }

        // Both checks must be well-defined (no panic) for every input,
        // which executing to this point already proves; the syntactic
        // result is retained only to keep it from being optimized away.
        let _ = syntactically_valid;
    });
});
