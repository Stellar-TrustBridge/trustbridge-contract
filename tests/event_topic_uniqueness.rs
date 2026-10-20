//! Event topic uniqueness assertion (Issue #677).
//!
//! `#[contractevent]` derives each event's first topic from its struct name in
//! snake_case, so two structs in `src/events.rs` mapping to the same symbol
//! would collide on the event stream. `scripts/check_event_topics.sh` runs the
//! same check in CI; this test keeps it inside `cargo test` too.

use std::collections::BTreeMap;

const EVENTS_RS: &str = include_str!("../src/events.rs");

fn to_snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Returns every derived topic that more than one event struct maps to.
fn duplicate_topics(src: &str) -> Vec<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for line in src.lines() {
        if let Some(rest) = line.strip_prefix("pub struct ") {
            let name: String = rest
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect();
            if name.ends_with("Event") {
                *counts.entry(to_snake(&name)).or_default() += 1;
            }
        }
    }
    assert!(!counts.is_empty(), "no event structs found");
    counts
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(t, _)| t)
        .collect()
}

#[test]
fn test_events_rs_topics_are_unique() {
    assert_eq!(duplicate_topics(EVENTS_RS), Vec::<String>::new());
}

#[test]
fn test_duplicate_topic_is_detected() {
    let src = "pub struct FooBarEvent {}\npub struct BazEvent {}\npub struct FooBarEvent {}\n";
    assert_eq!(duplicate_topics(src), vec!["foo_bar_event".to_string()]);
}

#[test]
fn test_snake_case_matches_sdk_derivation() {
    assert_eq!(to_snake("StagedWasmClearedEvent"), "staged_wasm_cleared_event");
}
