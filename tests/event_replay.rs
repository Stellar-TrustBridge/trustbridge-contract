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
    "oracle_allowlist_rotated_event",
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
        Delivery { event_type: "oracle_allowlist_rotated_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 36 },
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
        Delivery { event_type: "oracle_allowlist_rotated_event", github_username: "", stellar_address: "", ledger_sequence: 1_003_000, tx_hash: tx5, event_index: 36 },
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


// ── Issue #850: IndexGenerationBumped payload replay ───────────────────────

/// A cursor-invalidating bump delivery. Dedupe key is the RPC envelope, while
/// the payload carries the generation the consumer must pin.
#[derive(Clone, Debug, Eq, PartialEq)]
struct BumpDelivery {
    previous_generation: u32,
    new_generation: u32,
    reason: &'static str,
    ledger_sequence: u64,
    tx_hash: &'static str,
    event_index: u32,
}

impl BumpDelivery {
    fn envelope_id(&self) -> String {
        format!(
            "{NETWORK_ID}:{CONTRACT_ID}:{}:{}:{}",
            self.ledger_sequence, self.tx_hash, self.event_index
        )
    }
}

fn bump_deliveries() -> Vec<BumpDelivery> {
    vec![
        BumpDelivery {
            previous_generation: 0,
            new_generation: 1,
            reason: "remove_from_index",
            ledger_sequence: 1_004_000,
            tx_hash: "f1f2f3f4f5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90",
            event_index: 0,
        },
        BumpDelivery {
            previous_generation: 0,
            new_generation: 1,
            reason: "remove_from_index",
            ledger_sequence: 1_004_000,
            tx_hash: "f1f2f3f4f5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90",
            event_index: 0,
        },
    ]
}

#[derive(Default)]
struct BumpConsumer {
    seen: std::collections::HashSet<String>,
    generation: Option<u32>,
    applied: u32,
    ignored: u32,
    rejected_stale: u32,
    rejected_malformed: u32,
}

impl BumpConsumer {
    fn apply(&mut self, delivery: &BumpDelivery) {
        if !self.seen.insert(delivery.envelope_id()) {
            self.ignored += 1;
            return;
        }
        if delivery.new_generation <= delivery.previous_generation {
            self.rejected_stale += 1;
            return;
        }
        if delivery.reason.is_empty() {
            self.rejected_malformed += 1;
            return;
        }
        self.generation = Some(delivery.new_generation);
        self.applied += 1;
    }

    fn accept_cursor(&self, cursor_generation: u32) -> bool {
        self.generation == Some(cursor_generation)
    }
}

#[test]
fn index_generation_bumped_replay_is_idempotent_and_matches_fixture() {
    let deliveries = bump_deliveries();
    let fixture = include_str!("testdata/index_generation_bumped_fixture.json");
    for delivery in &deliveries {
        let needle = format!("\"new_generation\": {}", delivery.new_generation);
        assert!(fixture.contains(&needle), "fixture lacks {needle}");
        assert!(fixture.contains(&format!("\"reason\": \"{}\"", delivery.reason)));
        assert!(fixture.contains(&format!("\"event_index\": {}", delivery.event_index)));
    }
    assert_eq!(fixture.matches("\"event_type\"").count(), deliveries.len());

    let mut consumer = BumpConsumer::default();
    for delivery in &deliveries {
        consumer.apply(delivery);
    }
    assert_eq!(consumer.applied, 1);
    assert_eq!(consumer.ignored, 1);
    assert_eq!(consumer.generation, Some(1));
    assert!(consumer.accept_cursor(1));
    assert!(!consumer.accept_cursor(0), "pre-bump cursor must be invalid");
}

#[test]
fn index_generation_bumped_rejects_stale_generation() {
    let mut consumer = BumpConsumer::default();
    let mut stale = bump_deliveries().remove(0);
    stale.previous_generation = 4;
    stale.new_generation = 4;
    consumer.apply(&stale);
    assert_eq!(consumer.rejected_stale, 1);
    assert_eq!(consumer.applied, 0);
    assert_eq!(consumer.generation, None);
}

#[test]
fn index_generation_bumped_rejects_malformed_empty_reason() {
    let mut consumer = BumpConsumer::default();
    let mut malformed = bump_deliveries().remove(0);
    malformed.reason = "";
    consumer.apply(&malformed);
    assert_eq!(consumer.rejected_malformed, 1);
    assert_eq!(consumer.applied, 0);
    assert_eq!(consumer.generation, None);

// ── Reverify event absence & authoritative poll API ──────────────────────────

#[test]
fn pending_reverify_flag_set_and_clear_emit_no_dedicated_events_and_poll_works() {
    use soroban_sdk::{
        testutils::{Address as _, Events as _},
        xdr::{ContractEventBody, ScVal},
        Address, Env, String as SString, Symbol, TryFromVal, Vec as SVec,
    };
    use trustbridge_contract::TrustBridgeContract;

    // 1. Docs and src/events.rs consistently document absence + poll API.
    let events_rs = include_str!("../src/events.rs");
    let indexing_md = include_str!("../docs/EVENT_INDEXING.md");
    let subgraph_readme = include_str!("../docs/subgraph/README.md");
    let abi_md = include_str!("../docs/ABI.md");
    assert!(!EVENT_TYPES.iter().any(|t| t.contains("reverify")));
    assert!(events_rs.contains("get_pending_reverify"));
    assert!(indexing_md.contains("Pending Re-verification Signals and Poll API"));
    assert!(indexing_md.contains("get_pending_reverify"));
    assert!(subgraph_readme.contains("get_pending_reverify"));
    assert!(abi_md.contains("get_pending_reverify"));

    // 2. Runtime check: address change on verified user sets pending_reverify
    // while emitting only `registered_event`, and `verify` clears it while
    // emitting only `verified_event`.
    let env = Env::default();
    let admin = Address::generate(&env);
    let addr_old = Address::generate(&env);
    let addr_new = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();

    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            SString::from_str(&env, "alice"),
            addr_old,
            SVec::new(&env),
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(
            env.clone(),
            admin.clone(),
            SString::from_str(&env, "alice"),
        )
        .unwrap();
    });

    // Re-register to new address -> sets pending_reverify = true, emits ONLY registered_event.
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            SString::from_str(&env, "alice"),
            addr_new,
            SVec::new(&env),
        )
        .unwrap();
    });
    let set_filtered = env.events().all().filter_by_contract(&contract_id);
    let set_events = set_filtered.events();
    assert_eq!(set_events.len(), 1);
    let ContractEventBody::V0(v0) = &set_events[0].body;
    let expected_reg = ScVal::try_from_val(&env, &Symbol::new(&env, "registered_event").to_val()).unwrap();
    assert_eq!(v0.topics.first(), Some(&expected_reg));

    let pending_after_set = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_pending_reverify(env.clone(), 0, 100).unwrap()
    });
    assert_eq!(
        pending_after_set,
        SVec::from_array(&env, [SString::from_str(&env, "alice")])
    );

    // Verify -> clears pending_reverify, emits ONLY verified_event.
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(
            env.clone(),
            admin.clone(),
            SString::from_str(&env, "alice"),
        )
        .unwrap();
    });
    let clear_filtered = env.events().all().filter_by_contract(&contract_id);
    let clear_events = clear_filtered.events();
    assert_eq!(clear_events.len(), 1);
    let ContractEventBody::V0(v0) = &clear_events[0].body;
    let expected_ver = ScVal::try_from_val(&env, &Symbol::new(&env, "verified_event").to_val()).unwrap();
    assert_eq!(v0.topics.first(), Some(&expected_ver));

    let pending_after_clear = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_pending_reverify(env.clone(), 0, 100).unwrap()
    });
    assert!(pending_after_clear.is_empty());
}

/// Failure path 1: an event-only indexer relying on `RenamedEvent` sees
/// `verification_cleared == false` when an already-unverified record with an
/// active `pending_reverify` flag is renamed, so without polling
/// `get_pending_reverify` it misses `alice2` in the reverify queue.
#[test]
fn event_only_tracking_misses_migrated_reverify_flag_on_rename() {
    use soroban_sdk::{
        testutils::{Address as _, Events as _},
        xdr::{ContractEventBody, ScVal},
        Address, Env, String as SString, Symbol, TryFromVal, Vec as SVec,
    };
    use trustbridge_contract::TrustBridgeContract;

    let env = Env::default();
    let admin = Address::generate(&env);
    let addr_old = Address::generate(&env);
    let addr_new = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.mock_all_auths();

    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            SString::from_str(&env, "alice"),
            addr_old,
            SVec::new(&env),
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::verify(
            env.clone(),
            admin.clone(),
            SString::from_str(&env, "alice"),
        )
        .unwrap();
    });
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            SString::from_str(&env, "alice"),
            addr_new.clone(),
            SVec::new(&env),
        )
        .unwrap();
    });

    // Rename alice -> alice2 while pending_reverify = true and verified = false.
    env.as_contract(&contract_id, || {
        TrustBridgeContract::rename(
            env.clone(),
            addr_new,
            SString::from_str(&env, "alice"),
            SString::from_str(&env, "alice2"),
        )
        .unwrap();
    });
    let rename_filtered = env.events().all().filter_by_contract(&contract_id);
    let rename_events = rename_filtered.events();
    assert_eq!(rename_events.len(), 1);
    let ContractEventBody::V0(v0) = &rename_events[0].body;
    let expected_topic = ScVal::try_from_val(&env, &Symbol::new(&env, "renamed_event").to_val()).unwrap();
    assert_eq!(v0.topics.first(), Some(&expected_topic));
    // `verification_cleared` is false in the event data map, yet `alice2` is
    // still flagged in `get_pending_reverify`.
    let key_scval = ScVal::try_from_val(&env, &Symbol::new(&env, "verification_cleared").to_val()).unwrap();
    let ScVal::Map(Some(map)) = &v0.data else {
        panic!("expected ScVal::Map for RenamedEvent data");
    };
    let cleared_val = map
        .iter()
        .find(|entry| entry.key == key_scval)
        .expect("verification_cleared field present in RenamedEvent")
        .val
        .clone();
    assert_eq!(cleared_val, ScVal::Bool(false));

    let pending = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_pending_reverify(env.clone(), 0, 100).unwrap()
    });
    assert_eq!(
        pending,
        SVec::from_array(&env, [SString::from_str(&env, "alice2")])
    );
}

/// Failure path 2: polling `get_pending_reverify` fails with documented error
/// codes on uninitialized contracts (`NotInitialized`, code 2), oversized limits
/// (`InvalidBatchSize`, code 14), and mismatched networks (`NetworkMismatch`, code 30).
#[test]
fn pending_reverify_poll_api_rejects_uninitialized_oversized_limit_and_wrong_network() {
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _, LedgerInfo},
        Address, Env,
    };
    use trustbridge_contract::{ContractError, TrustBridgeContract};

    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    let uninit = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_pending_reverify(env.clone(), 0, 20)
    });
    assert_eq!(uninit, Err(ContractError::NotInitialized));

    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin).unwrap();
    });

    let bad_limit = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_pending_reverify(env.clone(), 0, 101)
    });
    assert_eq!(bad_limit, Err(ContractError::InvalidBatchSize));

    let info = env.ledger().get();
    let mut other_net = [0u8; 32];
    other_net[0] = 0x55;
    env.ledger().set(LedgerInfo {
        network_id: other_net,
        ..info
    });
    let wrong_net = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_pending_reverify(env.clone(), 0, 20)
    });
    assert_eq!(wrong_net, Err(ContractError::NetworkMismatch));
}
