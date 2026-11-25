//! Automated proof for Issue #825: the subgraph handler checklist must stay
//! present in all three docs and point back to the event declarations.
//!
//! `check_checklist` returns a stable reason code for the first missing step;
//! the failure-path tests remove one step at a time so a future edit cannot
//! silently drop part of the checklist.

const SUBGRAPH: &str = include_str!("../docs/subgraph/README.md");
const INDEXING: &str = include_str!("../docs/EVENT_INDEXING.md");
const CONTRIBUTING: &str = include_str!("../docs/CONTRIBUTING.md");
const EVENTS_SRC: &str = include_str!("../src/events.rs");

const STEPS: [&str; 5] = [
    "**Add the event**",
    "**Update `EVENT_INDEXING.md`**",
    "**Add the subgraph handler**",
    "**Update the fixture**",
    "**Add the dashboard consumer note**",
];

fn check_checklist(
    subgraph: &str,
    indexing: &str,
    contributing: &str,
    events: &str,
) -> Result<(), &'static str> {
    if !subgraph.contains("## Contributor checklist: adding a contract event") {
        return Err("missing checklist section in docs/subgraph/README.md");
    }
    for (i, step) in STEPS.iter().enumerate() {
        if !subgraph.contains(step) {
            return Err(match i {
                0 => "missing step: add event",
                1 => "missing step: update EVENT_INDEXING",
                2 => "missing step: subgraph handler",
                3 => "missing step: fixture",
                _ => "missing step: dashboard consumer note",
            });
        }
    }
    if !indexing.contains("contributor checklist") {
        return Err("missing checklist cross-link in docs/EVENT_INDEXING.md");
    }
    if !contributing.contains("subgraph handler checklist") {
        return Err("missing checklist cross-link in docs/CONTRIBUTING.md");
    }
    if !events.contains("operator checklist") || !events.contains("docs/subgraph/README.md") {
        return Err("missing checklist cross-link in src/events.rs");
    }
    Ok(())
}

#[test]
fn checklist_is_complete_in_all_docs() {
    assert_eq!(
        check_checklist(SUBGRAPH, INDEXING, CONTRIBUTING, EVENTS_SRC),
        Ok(())
    );
}

#[test]
fn missing_event_indexing_step_is_rejected() {
    let without = SUBGRAPH.replace(STEPS[1], "");
    assert_eq!(
        check_checklist(&without, INDEXING, CONTRIBUTING, EVENTS_SRC),
        Err("missing step: update EVENT_INDEXING")
    );
}

#[test]
fn missing_dashboard_consumer_note_is_rejected() {
    let without = SUBGRAPH.replace(STEPS[4], "");
    assert_eq!(
        check_checklist(&without, INDEXING, CONTRIBUTING, EVENTS_SRC),
        Err("missing step: dashboard consumer note")
    );
}
