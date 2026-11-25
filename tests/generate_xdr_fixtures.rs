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
use trustbridge_contract::{Role, TrustBridgeContract};

fn s(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

/// Find the data ScVal for an event whose first topic matches `symbol_name`.
/// Returns the base64-encoded XDR of the event value.
fn find_event_xdr(env: &Env, contract_id: &Address, symbol_name: &str) -> soroban_sdk::Bytes {
    use soroban_sdk::xdr::{ContractEventBody, Limits, ScVal, WriteXdr};
    let target = ScVal::try_from_val(env, &Symbol::new(env, symbol_name).to_val()).unwrap();
    for event in env.events().all().filter_by_contract(contract_id).events() {
        let ContractEventBody::V0(v0) = &event.body;
        if v0.topics.first() == Some(&target) {
            // Serialize the event data ScVal to XDR bytes.
            let raw = v0.data.to_xdr(Limits::none()).unwrap();
            return soroban_sdk::Bytes::from_slice(env, &raw);
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

        let xdr_bytes = soroban_sdk::xdr::ToXdr::to_xdr(record.clone(), &env);
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

    // Issue #599: emitted event payload fixtures. These are deterministic
    // synthetic host events (not transactions or upgrade execution).
    // This avoids invoking update_current_contract_wasm in a fixture test.
    let hash = soroban_sdk::BytesN::from_array(&env, &[0xAB; 32]);
    env.as_contract(&contract_id, || {
        use trustbridge_contract::{
            EventDomain, RoleGrantedEvent, RoleRevokedEvent, UpgradeApprovedEvent,
            UpgradeProposalCancelledEvent, UpgradeProposalExecutedEvent, UpgradeProposedEvent,
            UpgradedEvent,
        };
        let domain = EventDomain::new(&env, (1, 1, 0));
        UpgradedEvent {
            new_wasm_hash: hash.clone(),
            version: (1, 1, 0),
            timestamp: 1_700_000_000,
            domain: domain.clone(),
        }
        .publish(&env);
        RoleGrantedEvent {
            address: user.clone(),
            role: 1,
            admin: admin.clone(),
            timestamp: 1_700_000_001,
            domain: domain.clone(),
        }
        .publish(&env);
        RoleRevokedEvent {
            address: user.clone(),
            admin: admin.clone(),
            timestamp: 1_700_000_002,
            domain,
        }
        .publish(&env);
        UpgradeProposedEvent {
            proposal_id: 7,
            proposed_by: admin.clone(),
            wasm_hash: hash.clone(),
            executable_at: 1_700_000_100,
            timestamp: 1_700_000_003,
        }
        .publish(&env);
        UpgradeApprovedEvent {
            proposal_id: 7,
            approved_by: second.clone(),
            approval_count: 2,
            timestamp: 1_700_000_004,
        }
        .publish(&env);
        UpgradeProposalExecutedEvent {
            proposal_id: 7,
            executed_by: admin.clone(),
            wasm_hash: hash.clone(),
            approval_count: 2,
            timestamp: 1_700_000_005,
        }
        .publish(&env);
        UpgradeProposalCancelledEvent {
            proposal_id: 8,
            cancelled_by: admin.clone(),
            wasm_hash: hash,
            timestamp: 1_700_000_006,
        }
        .publish(&env);
    });

    for (name, kind, timestamp) in [
        ("upgraded_event", "upgraded", 1_700_000_000u64),
        ("role_granted_event", "role_granted", 1_700_000_001),
        ("role_revoked_event", "role_revoked", 1_700_000_002),
        ("upgrade_proposed_event", "upgrade_proposed", 1_700_000_003),
        ("upgrade_approved_event", "upgrade_approved", 1_700_000_004),
        (
            "upgrade_proposal_executed_event",
            "upgrade_proposal_executed",
            1_700_000_005,
        ),
        (
            "upgrade_proposal_cancelled_event",
            "upgrade_proposal_cancelled",
            1_700_000_006,
        ),
    ] {
        let payload = find_event_xdr(&env, &contract_id, name);
        print_fixture(
            name,
            &payload,
            &std::format!(
                r#"{{"fixture_version":1,"event_kind":"{kind}","ledger_hint":1000042,"timestamp":{timestamp}}}"#
            ),
        );
    }

    println!();
    println!(
        "Paste each block above into the corresponding file under ts-differential-tests/fixtures/"
    );
    println!("Or run: make xdr-fixtures");
}
