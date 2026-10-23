//! Replay-idempotency fixture for Issue #283.
//!
//! None of the contract's `#[contractevent]` payloads carry a sequence number,
//! so a consumer that keys only on `(github_username, event_type, timestamp)`
//! double-applies `RegisteredEvent` / `VerifiedEvent` on every indexer
//! reconnect. This test pins the **stable event ID** scheme documented in
//! `docs/DASHBOARD_SYNC.md` ("Stable event ID (Issue #283)") and proves a
//! consumer that keys on it is replay-safe.
//!
//! The scheme is derived entirely from the Horizon/RPC delivery envelope, not
//! from the event payload:
//!
//! ```text
//! event_id = "{network_id}:{contract_id}:{ledger_sequence}:{tx_hash}:{event_index}"
//! ```
//!
//! Uniqueness scope is global: `network_id` and `contract_id` are embedded, so
//! an id is stable for the life of one contract instance on one network and
//! never collides with a redeploy or another network (Issue #226 domain
//! separation, applied to the id itself).
//!
//! `tests/testdata/event_replay_fixture.json` is the language-neutral copy of
//! the same deliveries for consumers in other stacks; this test asserts the two
//! stay in sync.

#![cfg(test)]

/// One raw event delivery, exactly as an indexer receives it.
struct Delivery {
    event_type: &'static str,
    github_username: &'static str,
    stellar_address: &'static str,
    ledger_sequence: u64,
    tx_hash: &'static str,
    event_index: u32,
}

// Explicit model catalogue; checked against the source #[contractevent] declarations.
const EVENT_TYPES: &[&str] = &[
    "registered_event",
    "bot_status_changed_event",
    "removed_event",
    "verified_event",
    "verification_revoked_event",
    "upgraded_event",
    "paused_event",
    "unpaused_event",
    "emergency_paused_event",
    "emergency_cleared_event",
    "guardian_changed_event",
    "role_grant_pending_event",
    "role_grant_cancelled_event",
    "renamed_event",
    "rotation_requested_event",
    "rotation_executed_event",
    "rotation_cancelled_event",
    "wasm_staged_event",
    "upgrade_attested_event",
    "attestation_cleared_event",
    "role_granted_event",
    "role_revoked_event",
    "batch_remove_proposed_event",
    "batch_remove_executed_event",
    "batch_remove_cancelled_event",
    "challenge_started_event",
    "challenge_cancelled_event",
    "challenge_completed_event",
    "staged_wasm_cleared_event",
    "upgrade_proposed_event",
    "upgrade_approved_event",
    "upgrade_proposal_executed_event",
    "upgrade_proposal_cancelled_event",
    "verification_configured_event",
    "audit_config_changed_event",
    "admin_transfer_proposed_event",
    "admin_transfer_cancelled_event",
    "admin_transfer_executed_event",
];

const NETWORK_ID: &str = "cee0302d59844d32bdca915c8203dd44b33fbb7edc19051ea37abedf28ecd472";
const CONTRACT_ID: &str = "CDTRUSTBRIDGEEXAMPLECONTRACTID000000000000000000000000AAAA";

/// The documented derivation. Pure function of the delivery envelope.
fn event_id(d: &Delivery) -> String {
    format!(
        "{NETWORK_ID}:{CONTRACT_ID}:{}:{}:{}",
        d.ledger_sequence, d.tx_hash, d.event_index
    )
}

#[rustfmt::skip]
fn deliveries() -> Vec<Delivery> {
    let addr_a = "GA7QYNF7SOWQ3GLR2BGMZEHXAVIRZA4KVWLTJJFC7MGXUA74P7UJVSGZ";
    let addr_b = "GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFSHONUCEOASW7QC7OX2H";
    let tx1 = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f901";
    let tx2 = "b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f901a2";
    let tx3 = "c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f901a2b3";
    let tx5 = "e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f901a2b3c4d5";
    let tx4 = "d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f901a2b3c4";
    vec![
        // 1. register
        Delivery { event_type: "registered_event", github_username: "octocat", stellar_address: addr_a, ledger_sequence: 1_000_042, tx_hash: tx1, event_index: 0 },
        // 2. verify
        Delivery { event_type: "verified_event", github_username: "octocat", stellar_address: addr_a, ledger_sequence: 1_000_050, tx_hash: tx2, event_index: 0 },
        // 3. revoke
        Delivery { event_type: "verification_revoked_event", github_username: "octocat", stellar_address: addr_a, ledger_sequence: 1_000_061, tx_hash: tx3, event_index: 1 },
        // 4. exact replay of #2 on reconnect
        Delivery { event_type: "verified_event", github_username: "octocat", stellar_address: addr_a, ledger_sequence: 1_000_050, tx_hash: tx2, event_index: 0 },
        // 5. exact replay of #1 on catch-up
        Delivery { event_type: "registered_event", github_username: "octocat", stellar_address: addr_a, ledger_sequence: 1_000_042, tx_hash: tx1, event_index: 0 },
        // 6. genuine re-registration in a later ledger
        Delivery { event_type: "registered_event", github_username: "octocat", stellar_address: addr_b, ledger_sequence: 1_002_000, tx_hash: tx4, event_index: 0 },
        // One distinct envelope for every contract event type.
        Delivery { event_type: "registered_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 1 },
        Delivery { event_type: "bot_status_changed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 2 },
        Delivery { event_type: "removed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 3 },
        Delivery { event_type: "verified_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 4 },
        Delivery { event_type: "verification_revoked_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 5 },
        Delivery { event_type: "upgraded_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 6 },
        Delivery { event_type: "paused_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 7 },
        Delivery { event_type: "unpaused_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 8 },
        Delivery { event_type: "emergency_paused_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 9 },
        Delivery { event_type: "emergency_cleared_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 10 },
        Delivery { event_type: "guardian_changed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 11 },
        Delivery { event_type: "role_grant_pending_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 12 },
        Delivery { event_type: "role_grant_cancelled_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 13 },
        Delivery { event_type: "renamed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 14 },
        Delivery { event_type: "rotation_requested_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 15 },
        Delivery { event_type: "rotation_executed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 16 },
        Delivery { event_type: "rotation_cancelled_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 17 },
        Delivery { event_type: "wasm_staged_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 18 },
        Delivery { event_type: "upgrade_attested_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 19 },
        Delivery { event_type: "attestation_cleared_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 20 },
        Delivery { event_type: "role_granted_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 21 },
        Delivery { event_type: "role_revoked_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 22 },
        Delivery { event_type: "batch_remove_proposed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 23 },
        Delivery { event_type: "batch_remove_executed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 24 },
        Delivery { event_type: "batch_remove_cancelled_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 25 },
        Delivery { event_type: "challenge_started_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 26 },
        Delivery { event_type: "challenge_cancelled_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 27 },
        Delivery { event_type: "challenge_completed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 28 },
        Delivery { event_type: "staged_wasm_cleared_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 29 },
        Delivery { event_type: "upgrade_proposed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 30 },
        Delivery { event_type: "upgrade_approved_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 31 },
        Delivery { event_type: "upgrade_proposal_executed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 32 },
        Delivery { event_type: "upgrade_proposal_cancelled_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 33 },
        Delivery { event_type: "verification_configured_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 34 },
        Delivery { event_type: "audit_config_changed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 35 },
        Delivery { event_type: "admin_transfer_proposed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 36 },
        Delivery { event_type: "admin_transfer_cancelled_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 37 },
        Delivery { event_type: "admin_transfer_executed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 38 },
        // Exact reconnect deliveries for every event type.
        Delivery { event_type: "registered_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 1 },
        Delivery { event_type: "bot_status_changed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 2 },
        Delivery { event_type: "removed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 3 },
        Delivery { event_type: "verified_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 4 },
        Delivery { event_type: "verification_revoked_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 5 },
        Delivery { event_type: "upgraded_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 6 },
        Delivery { event_type: "paused_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 7 },
        Delivery { event_type: "unpaused_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 8 },
        Delivery { event_type: "emergency_paused_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 9 },
        Delivery { event_type: "emergency_cleared_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 10 },
        Delivery { event_type: "guardian_changed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 11 },
        Delivery { event_type: "role_grant_pending_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 12 },
        Delivery { event_type: "role_grant_cancelled_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 13 },
        Delivery { event_type: "renamed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 14 },
        Delivery { event_type: "rotation_requested_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 15 },
        Delivery { event_type: "rotation_executed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 16 },
        Delivery { event_type: "rotation_cancelled_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 17 },
        Delivery { event_type: "wasm_staged_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 18 },
        Delivery { event_type: "upgrade_attested_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 19 },
        Delivery { event_type: "attestation_cleared_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 20 },
        Delivery { event_type: "role_granted_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 21 },
        Delivery { event_type: "role_revoked_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 22 },
        Delivery { event_type: "batch_remove_proposed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 23 },
        Delivery { event_type: "batch_remove_executed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 24 },
        Delivery { event_type: "batch_remove_cancelled_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 25 },
        Delivery { event_type: "challenge_started_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 26 },
        Delivery { event_type: "challenge_cancelled_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 27 },
        Delivery { event_type: "challenge_completed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 28 },
        Delivery { event_type: "staged_wasm_cleared_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 29 },
        Delivery { event_type: "upgrade_proposed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 30 },
        Delivery { event_type: "upgrade_approved_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 31 },
        Delivery { event_type: "upgrade_proposal_executed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 32 },
        Delivery { event_type: "upgrade_proposal_cancelled_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 33 },
        Delivery { event_type: "verification_configured_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 34 },
        Delivery { event_type: "audit_config_changed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 35 },
        Delivery { event_type: "admin_transfer_proposed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 36 },
        Delivery { event_type: "admin_transfer_cancelled_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 37 },
        Delivery { event_type: "admin_transfer_executed_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 38 },
    ]
}

/// Minimal consumer model: last-write-wins per username, deduped by `event_id`.
#[derive(Default)]
struct Consumer {
    seen: std::collections::HashSet<String>,
    address: Option<String>,
    verified: bool,
    applied: u32,
    ignored: u32,
    per_type: std::collections::HashMap<&'static str, u32>,
}

impl Consumer {
    fn apply(&mut self, d: &Delivery) {
        let id = event_id(d);
        if !self.seen.insert(id) {
            self.ignored += 1;
            return;
        }
        self.applied += 1;
        *self.per_type.entry(d.event_type).or_default() += 1;
        match (d.event_type, d.github_username) {
            ("registered_event", "octocat") => {
                self.address = Some(d.stellar_address.to_string());
                self.verified = false;
            }
            ("verified_event", "octocat") => self.verified = true,
            ("verification_revoked_event", "octocat") => self.verified = false,
            ("removed_event", "octocat") => self.address = None,
            // Envelope-only model: record non-registry events by type and stable ID.
            // Their domain-specific state is handled by downstream projections.
            _ => {}
        }
    }
}

#[test]
fn event_replay_is_idempotent() {
    let deliveries = deliveries();
    assert_eq!(deliveries.len(), 6 + 2 * EVENT_TYPES.len());

    let mut once = Consumer::default();
    for d in &deliveries {
        once.apply(d);
    }

    // Replaying the whole stream again — a full re-sync — must not move state.
    let mut twice = Consumer::default();
    for d in deliveries.iter().chain(deliveries.iter()) {
        twice.apply(d);
    }

    assert_eq!(once.applied, 4 + EVENT_TYPES.len() as u32);
    assert_eq!(once.ignored, 2 + EVENT_TYPES.len() as u32);
    assert_eq!(
        once.address.as_deref(),
        Some("GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFSHONUCEOASW7QC7OX2H"),
        "final address is the re-registration target"
    );
    assert!(
        !once.verified,
        "revoke is the last verification-affecting event"
    );

    assert_eq!(
        twice.applied, once.applied,
        "second full pass applies nothing"
    );
    assert_eq!(twice.address, once.address);
    assert_eq!(twice.verified, once.verified);
    assert_eq!(twice.per_type, once.per_type);
    for event_type in EVENT_TYPES {
        assert!(
            once.per_type.contains_key(*event_type),
            "missing delivery for {event_type}"
        );
    }
}

#[test]
fn event_ids_match_the_language_neutral_fixture() {
    let fixture = include_str!("testdata/event_replay_fixture.json");
    let deliveries = deliveries();
    let from_json = |field: &str| -> Vec<String> {
        let prefix = format!("\"{field}\": \"");
        fixture
            .split(&prefix)
            .skip(1)
            .map(|s| s.split('"').next().unwrap().to_string())
            .collect()
    };
    let ids = from_json("event_id");
    let types = from_json("event_type");
    assert_eq!(
        ids.len(),
        deliveries.len(),
        "JSON/Rust delivery counts differ"
    );
    assert_eq!(
        types.len(),
        deliveries.len(),
        "JSON/Rust type counts differ"
    );
    for (i, d) in deliveries.iter().enumerate() {
        assert_eq!(ids[i], event_id(d), "JSON/Rust ID mismatch at delivery {i}");
        assert_eq!(
            types[i], d.event_type,
            "JSON/Rust event mismatch at delivery {i}"
        );
    }
}

#[test]
fn every_contractevent_has_replay_coverage() {
    let source = include_str!("../src/events.rs");
    let mut source_types = Vec::new();
    let mut pending = false;
    for line in source.lines() {
        if line.trim() == "#[contractevent]" {
            pending = true;
        }
        if pending {
            if let Some(name) = line.trim().strip_prefix("pub struct ") {
                let name = name.split_whitespace().next().unwrap();
                let mut topic = String::new();
                for (i, ch) in name.chars().enumerate() {
                    if ch.is_ascii_uppercase() && i > 0 {
                        topic.push('_');
                    }
                    topic.push(ch.to_ascii_lowercase());
                }
                source_types.push(topic);
                pending = false;
            }
        }
    }
    let documented: std::collections::HashSet<_> = EVENT_TYPES.iter().copied().collect();
    let discovered: std::collections::HashSet<_> =
        source_types.iter().map(String::as_str).collect();
    assert_eq!(
        documented, discovered,
        "update the replay fixture for every new #[contractevent]"
    );
    assert_eq!(documented.len(), EVENT_TYPES.len(), "duplicate event types");
    // A distinct applied delivery and an exact replay exist for each topic.
    for name in EVENT_TYPES {
        assert!(
            deliveries()
                .iter()
                .filter(|d| d.event_type == *name)
                .count()
                >= 2,
            "missing original/replay pair for {name}"
        );
    }
}
