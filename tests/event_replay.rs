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
}

impl Consumer {
    fn apply(&mut self, d: &Delivery) {
        let id = event_id(d);
        if !self.seen.insert(id) {
            self.ignored += 1;
            return;
        }
        self.applied += 1;
        match d.event_type {
            "registered_event" => {
                self.address = Some(d.stellar_address.to_string());
                self.verified = false;
            }
            "verified_event" => self.verified = true,
            "verification_revoked_event" => self.verified = false,
            "removed_event" => self.address = None,
            other => panic!("unhandled event_type in fixture: {other}"),
        }
    }
}

#[test]
fn event_replay_is_idempotent() {
    let deliveries = deliveries();
    assert!(
        deliveries.iter().all(|d| d.github_username == "octocat"),
        "fixture is a single-username replay scenario"
    );

    let mut once = Consumer::default();
    for d in &deliveries {
        once.apply(d);
    }

    // Replaying the whole stream again — a full re-sync — must not move state.
    let mut twice = Consumer::default();
    for d in deliveries.iter().chain(deliveries.iter()) {
        twice.apply(d);
    }

    assert_eq!(once.applied, 4, "four distinct events in the fixture");
    assert_eq!(once.ignored, 2, "two deliveries are exact replays");
    assert_eq!(
        once.address.as_deref(),
        Some("GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFSHONUCEOASW7QC7OX2H"),
        "final address is the re-registration target"
    );
    assert!(!once.verified, "revoke is the last verification-affecting event");

    assert_eq!(twice.applied, once.applied, "second full pass applies nothing");
    assert_eq!(twice.address, once.address);
    assert_eq!(twice.verified, once.verified);
}

#[test]
fn event_ids_match_the_language_neutral_fixture() {
    let fixture = include_str!("testdata/event_replay_fixture.json");
    for d in &deliveries() {
        let id = event_id(d);
        assert!(
            fixture.contains(&id),
            "fixture JSON is missing event_id {id}; regenerate testdata/event_replay_fixture.json"
        );
    }
}
