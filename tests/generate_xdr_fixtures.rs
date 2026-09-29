//! Generate XDR fixtures for TypeScript differential testing.
//!
//! This test creates golden XDR outputs from contract invocations and events
//! that TypeScript clients can decode to verify bindings correctness.
//!
//! Run with:
//!   cargo test generate_xdr_fixtures -- --ignored --nocapture
//!
//! Then copy each printed block into the corresponding file under
//! `ts-differential-tests/fixtures/`.  The Makefile target `xdr-fixtures`
//! does this automatically:
//!   make xdr-fixtures
//!
//! # Fixtures produced
//!
//! | Fixture name                     | What it tests                        |
//! |----------------------------------|--------------------------------------|
//! | `get_address_octocat`            | `ContributorRecord` XDR decode       |
//! | `batch_remove_proposed_event`    | `BatchRemoveProposedEvent` XDR decode|
//! | `batch_remove_executed_event`    | `BatchRemoveExecutedEvent` XDR decode|
//! | `batch_remove_cancelled_event`   | `BatchRemoveCancelledEvent` XDR decode|
//!
//! Each fixture is a base64-encoded XDR `ScVal` (the value half of a contract
//! event or return value) paired with a `.meta` JSON sidecar that records the
//! expected decoded field values for the TypeScript assertions.

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Events as _},
    Address, Env, String, Symbol, TryFromVal, Vec,
};
use trustbridge_contract::{
    BatchRemoveCancelledEvent, BatchRemoveExecutedEvent, BatchRemoveProposedEvent, Role,
    TrustBridgeContract,
};

fn s(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

/// Find the data ScVal for an event whose first topic matches `symbol_name`.
/// Returns the base64-encoded XDR of the event value.
fn find_event_xdr(env: &Env, contract_id: &Address, symbol_name: &str) -> soroban_sdk::Bytes {
    let target = Symbol::new(env, symbol_name);
    for (source, topics, data) in env.events().all() {
        if source == *contract_id {
            if let Some(first) = topics.get(0) {
                if Symbol::try_from_val(env, &first)
                    .map(|s| s == target)
                    .unwrap_or(false)
                {
                    // Serialize the event data ScVal to XDR bytes.
                    return data.to_xdr(env);
                }
            }
        }
    }
    panic!("event '{symbol_name}' not found in event log");
}

/// Print a fixture block in the format the Makefile and README expect.
fn print_fixture(name: &str, xdr_bytes: &soroban_sdk::Bytes, meta_json: &str) {
    // Convert XDR bytes to base64.  soroban_sdk::Bytes implements
    // IntoIterator so we collect to a std::vec::Vec<u8> and encode.
    let raw: std::vec::Vec<u8> = xdr_bytes.iter().collect();
    let b64 = base64_encode(&raw);

    println!();
    println!("=== FIXTURE: {name} ===");
    println!("--- {name}.xdr ---");
    println!("{b64}");
    println!("--- {name}.meta.json ---");
    println!("{meta_json}");
    println!("=== END FIXTURE: {name} ===");
}

/// Minimal base64 encoder (no external dep, works in #[no_std] test harness).
fn base64_encode(input: &[u8]) -> std::string::String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = std::string::String::new();
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i] as u32;
        let b1 = if i + 1 < input.len() {
            input[i + 1] as u32
        } else {
            0
        };
        let b2 = if i + 2 < input.len() {
            input[i + 2] as u32
        } else {
            0
        };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((n >> 18) & 0x3F) as usize] as char);
        out.push(CHARS[((n >> 12) & 0x3F) as usize] as char);
        if i + 1 < input.len() {
            out.push(CHARS[((n >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < input.len() {
            out.push(CHARS[(n & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

// ---------------------------------------------------------------------------
// Fixture 1: ContributorRecord — get_address("octocat")
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn generate_xdr_fixtures() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });

    let username = s(&env, "octocat");
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(env.clone(), username.clone(), user.clone(), Vec::new(&env))
            .unwrap();
    });

    env.as_contract(&contract_id, || {
        let record = TrustBridgeContract::get_address(env.clone(), username.clone())
            .expect("record should exist");

        let xdr_bytes = record.to_xdr(&env);
        let raw: std::vec::Vec<u8> = xdr_bytes.iter().collect();
        let b64 = base64_encode(&raw);
        let addr_str = record.stellar_address.to_string();

        println!();
        println!("=== FIXTURE: get_address_octocat ===");
        println!("--- get_address_octocat.xdr ---");
        println!("{b64}");
        println!("--- get_address_octocat.address ---");
        println!("{addr_str}");
        println!("=== END FIXTURE: get_address_octocat ===");
    });

    // ---------------------------------------------------------------------------
    // Fixtures 2–4: BatchRemove events
    // ---------------------------------------------------------------------------
    //
    // We need a second admin (Role::Admin) so we can exercise the dual-control
    // flow end to end.  The events are captured after each contract call and
    // serialized to XDR.

    let second = Address::generate(&env);
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::set_role(env.clone(), second.clone(), Role::Admin).unwrap();
    });

    // Register 4 users so the batch is above the threshold.
    env.mock_all_auths();
    let mut names = Vec::new(&env);
    env.as_contract(&contract_id, || {
        for i in 0u32..4 {
            let name = s(&env, &std::format!("batchuser{i:02}"));
            TrustBridgeContract::register(
                env.clone(),
                name.clone(),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
            names.push_back(name);
        }
        TrustBridgeContract::set_batch_remove_threshold(env.clone(), 3).unwrap();
    });

    // ── Fixture 2: BatchRemoveProposedEvent ──────────────────────────────────

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();
    });

    let proposed_event_xdr = find_event_xdr(&env, &contract_id, "batch_remove_proposed_event");
    let proposed_by_str = admin.to_string();
    print_fixture(
        "batch_remove_proposed_event",
        &proposed_event_xdr,
        &std::format!(
            r#"{{"event_kind":"batch_remove_proposed","proposed_by":"{proposed_by_str}","count":4}}"#
        ),
    );

    // ── Fixture 3: BatchRemoveExecutedEvent ──────────────────────────────────

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::execute_batch_remove(env.clone(), second.clone()).unwrap();
    });

    let executed_event_xdr = find_event_xdr(&env, &contract_id, "batch_remove_executed_event");
    let executed_by_str = second.to_string();
    print_fixture(
        "batch_remove_executed_event",
        &executed_event_xdr,
        &std::format!(
            r#"{{"event_kind":"batch_remove_executed","executed_by":"{executed_by_str}","proposed_by":"{proposed_by_str}","count":4,"successful":4}}"#
        ),
    );

    // Re-register the users so we can produce a cancel fixture next.
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        for i in 0u32..4 {
            let name = s(&env, &std::format!("batchuser{i:02}"));
            TrustBridgeContract::register(
                env.clone(),
                name.clone(),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        }
        TrustBridgeContract::propose_batch_remove(env.clone(), admin.clone(), names.clone())
            .unwrap();
    });

    // ── Fixture 4: BatchRemoveCancelledEvent ─────────────────────────────────

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::cancel_batch_remove(env.clone(), admin.clone()).unwrap();
    });

    let cancelled_event_xdr = find_event_xdr(&env, &contract_id, "batch_remove_cancelled_event");
    print_fixture(
        "batch_remove_cancelled_event",
        &cancelled_event_xdr,
        &std::format!(
            r#"{{"event_kind":"batch_remove_cancelled","cancelled_by":"{proposed_by_str}","proposed_by":"{proposed_by_str}"}}"#
        ),
    );

    println!();
    println!(
        "Paste each block above into the corresponding file under ts-differential-tests/fixtures/"
    );
    println!("Or run: make xdr-fixtures");
}
