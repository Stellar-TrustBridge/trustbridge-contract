//! Generate XDR fixtures for TypeScript differential testing.
//!
//! This test creates golden XDR outputs from contract invocations
//! that TypeScript clients can decode to verify bindings correctness.
//!
//! Run with: cargo test generate_xdr_fixtures -- --ignored --exact

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String};
use trustbridge_contract::TrustBridgeContract;

fn s(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

#[test]
#[ignore]
fn generate_xdr_fixtures() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    // Initialize contract
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });

    // Register a test user
    let username = s(&env, "octocat");
    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            username.clone(),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    // Get address and serialize to XDR
    env.as_contract(&contract_id, || {
        let record = TrustBridgeContract::get_address(env.clone(), username.clone())
            .expect("record should exist");
        
        // Serialize the record to XDR for TypeScript to decode
        let xdr = record.to_xdr(&env);
        
        println!("=== XDR Fixture for get_address('octocat') ===");
        println!("{}", xdr);
        println!("=== Stellar Address ===");
        println!("{}", record.stellar_address.to_string());
    });
}
