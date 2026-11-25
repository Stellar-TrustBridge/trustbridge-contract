//! `get_health` vs `get_health_probes` semantics that the SLOs in
//! docs/CONTRACT_HEALTH.md (Issue #756) rely on: which probe errors and which
//! never does, and the healthy/unhealthy field values alert rules key on.
//!
//! Run with `cargo test --test health_probes`.
//! Issue #848 adds the attestation-required divergence alert mapping checks.

#![cfg(test)]

const HEALTH_DOC: &str = include_str!("../docs/CONTRACT_HEALTH.md");

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, BytesN, Env, String, Vec,
};
use trustbridge_contract::{ContractError, TrustBridgeContract, TrustBridgeContractClient};

// Realistic ledger time: `cooldown_remaining_secs` counts from the last upgrade
// timestamp, which is 0 for a never-upgraded deployment.
const NOW: u64 = 1_700_000_000;

fn deploy(env: &Env) -> TrustBridgeContractClient<'static> {
    env.mock_all_auths();
    env.ledger().set_timestamp(NOW);
    TrustBridgeContractClient::new(env, &env.register(TrustBridgeContract, ()))
}

fn initialized() -> (Env, TrustBridgeContractClient<'static>, Address) {
    let env = Env::default();
    let client = deploy(&env);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client, admin)
}

/// Happy path: both probes agree on a healthy, initialized deployment.
#[test]
fn healthy_deployment_reports_healthy_values_on_both_probes() {
    let (env, client, _admin) = initialized();
    client.register(
        &String::from_str(&env, "octocat"),
        &Address::generate(&env),
        &Vec::new(&env),
    );

    let health = client.get_health();
    assert!(!health.paused);
    assert_eq!(health.total, 1);
    assert_eq!(health.cooldown_remaining_secs, 0);
    assert!(!health.attestation_present);

    let probes = client.get_health_probes();
    assert!(!probes.paused && probes.admin_set);
}

/// Failure path 1 — uninitialized deployment: `get_health` *errors* with
/// `NotInitialized`, while `get_health_probes` *never errors* and fails closed
/// with `admin_set: false`.
#[test]
fn uninitialized_get_health_errors_but_probes_answer() {
    let env = Env::default();
    let client = deploy(&env);

    assert_eq!(
        client.try_get_health(),
        Err(Ok(ContractError::NotInitialized))
    );
    let probes = client.get_health_probes();
    assert!(!probes.admin_set, "probe must fail closed, not error");
    assert!(!probes.paused);
    assert_eq!(probes.index_gen, 0);
}

/// Failure path 2 — paused contract: both probes still answer (they are the
/// incident tools) and both report `paused: true`.
#[test]
fn paused_contract_is_reported_by_both_probes() {
    let (_env, client, _admin) = initialized();
    client.pause(&1);

    assert!(
        client.get_health().paused,
        "get_health must work while paused"
    );
    assert!(
        client.get_health_probes().paused,
        "get_health_probes must work while paused"
    );

    client.unpause(&1);
    assert!(!client.get_health().paused);
}

/// Failure path 3 — removals bump `index_gen` (dashboards must re-sync).
#[test]
fn removal_bumps_index_gen() {
    let (env, client, admin) = initialized();
    let user = String::from_str(&env, "octocat");
    client.register(&user, &Address::generate(&env), &Vec::new(&env));
    let gen0 = client.get_health_probes().index_gen;

    client.remove(&admin, &user);
    assert!(client.get_health_probes().index_gen > gen0);
}

/// Upgrade-window signals: a live attestation is reported only until it expires.
#[test]
fn attestation_present_only_until_expiry() {
    let (env, client, _admin) = initialized();
    client.attest_upgrade(&BytesN::from_array(&env, &[9; 32]), &(NOW + 60));
    assert!(client.get_health().attestation_present);

    env.ledger().set_timestamp(NOW + 60);
    assert!(!client.get_health().attestation_present);
}

/// A configured cooldown with no upgrade yet reports nothing remaining.
#[test]
fn cooldown_configured_without_upgrade_reports_zero_remaining() {
    let (_env, client, _admin) = initialized();
    client.set_cooldown(&86_400);
    let health = client.get_health();
    assert_eq!(health.cooldown_secs, 86_400);
    assert_eq!(health.cooldown_remaining_secs, 0);
}

// ── Attestation divergence (Issue #848) ──────────────────────────────────────

/// Happy baseline: no attestation and no required flag is healthy, not a page.
#[test]
fn absent_attestation_without_required_flag_is_baseline() {
    let (_env, client, _admin) = initialized();
    assert!(!client.get_health().attestation_present);
    assert!(!client.get_attestation_required());
}

/// Failure path 1 — the divergence itself: required flag set, no attestation
/// published. The health field stays false; the policy read must be paired with
/// it to produce the documented page/warn.
#[test]
fn required_flag_with_no_attestation_is_a_divergence() {
    let (_env, client, admin) = initialized();
    client.set_attestation_required(&admin, &true);

    assert!(!client.get_health().attestation_present);
    assert!(client.get_attestation_required());
}

/// Failure path 2 — a required attestation that expires leaves the same
/// divergence: presence is false again, while the policy remains required.
#[test]
fn required_attestation_expiry_leaves_divergence_visible() {
    let (env, client, admin) = initialized();
    client.attest_upgrade(&BytesN::from_array(&env, &[9; 32]), &(NOW + 60));
    client.set_attestation_required(&admin, &true);
    assert!(client.get_health().attestation_present);

    env.ledger().set_timestamp(NOW + 60);
    assert!(!client.get_health().attestation_present);
    assert!(client.get_attestation_required());
}

/// The runbook mapping is machine-checked so a doc edit cannot silently drop
/// the page/warn rule for this divergence.
#[test]
fn health_doc_documents_divergence_alert_mapping() {
    assert!(HEALTH_DOC.contains("Attestation divergence alert mapping (Issue #848)"));
    assert!(HEALTH_DOC.contains("| `true` | `false` |"));
    assert!(HEALTH_DOC.contains("AttestationRequired"));
    assert!(HEALTH_DOC.contains("get_attestation_required()"));
}
