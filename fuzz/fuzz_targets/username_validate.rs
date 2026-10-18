#![no_main]

use libfuzzer_sys::fuzz_target;
use soroban_sdk::{Env, String};
use trustbridge_contract::utils::is_valid_github_username;

fuzz_target!(|data: &[u8]| {
    // Feed arbitrary bytes (invalid UTF-8, null bytes, over-long inputs) into
    // the GitHub username validator. It must never panic and must
    // deterministically accept/reject.
    let env = Env::default();
    let text = std::str::from_utf8(data).unwrap_or("");
    let Ok(s) = String::from_str(&env, text) else {
        return;
    };
    let _ = is_valid_github_username(&s);
});
