//! Automated proof for the copy-paste post-deploy/upgrade probe sequence in
//! docs/DEPLOYMENT.md (Issue #863): `get_health_probes` is read-only, answers
//! while paused, and fails closed with `admin_set: false` when uninitialized.
//!
//! Run with `cargo test --test deployment_probe`.

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env,
};
use trustbridge_contract::{ContractError, TrustBridgeContract, TrustBridgeContractClient};

const NOW: u64 = 1_700_000_000;

fn deploy(env: &Env) -> TrustBridgeContractClient<'static> {
    env.mock_all_auths();
    env.ledger().set_timestamp(NOW);
    TrustBridgeContractClient::new(env, &env.register(TrustBridgeContract, ()))
}

/// Happy path: after `initialize`, the documented probe reports healthy values.
#[test]
fn post_deploy_probe_reports_healthy_initialized_contract() {
    let env = Env::default();
    let client = deploy(&env);
    client.initialize(&Address::generate(&env));

    let probes = client.get_health_probes();
    assert!(!probes.paused);
    assert!(probes.admin_set);
    assert_eq!(probes.index_gen, 0);
}

/// Failure path 1: uninitialized/wrong deploy. `get_health` reports
/// `NotInitialized` (code 2) while the probe still answers with
/// `admin_set: false`, as documented in DEPLOYMENT.md.
#[test]
fn post_deploy_probe_fails_closed_when_uninitialized() {
    let env = Env::default();
    let client = deploy(&env);

    assert_eq!(
        client.try_get_health(),
        Err(Ok(ContractError::NotInitialized))
    );
    let probes = client.get_health_probes();
    assert!(!probes.admin_set);
    assert!(!probes.paused);
    assert_eq!(probes.index_gen, 0);
}

/// Failure path 2: paused/frozen after upgrade. The probe still answers,
/// reports `paused: true`, and clears after `unpause` (reason code 4).
#[test]
fn post_deploy_probe_reports_pause_flag_and_recovers() {
    let env = Env::default();
    let client = deploy(&env);
    client.initialize(&Address::generate(&env));

    client.pause(&1);
    let probes = client.get_health_probes();
    assert!(probes.paused);
    assert!(probes.admin_set);

    client.unpause(&4);
    assert!(!client.get_health_probes().paused);
}
