#![no_main]

use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};
use trustbridge_contract::{ContractError, TrustBridgeContract};

fuzz_target!(|data: &[u8]| {
    // Inputs are deliberately decoded without assuming a fixed length. Missing
    // bytes become zero, exercising empty and truncated caller encodings too.
    let mut fields = [0u8; 8];
    let n = data.len().min(fields.len());
    fields[..n].copy_from_slice(&data[..n]);
    let offset = u32::from_le_bytes(fields[..4].try_into().unwrap());
    let limit = u32::from_le_bytes(fields[4..].try_into().unwrap());

    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
        for name in ["alice", "bob", "carol"] {
            let username = String::from_str(&env, name);
            TrustBridgeContract::register(
                env.clone(),
                username.clone(),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
            TrustBridgeContract::verify(env.clone(), admin.clone(), username.clone()).unwrap();
            TrustBridgeContract::register(
                env.clone(),
                username,
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        }

        let result = TrustBridgeContract::get_pending_reverify(env.clone(), offset, limit);
        if limit > 100 {
            assert_eq!(result, Err(ContractError::InvalidBatchSize));
        } else {
            let page = result.expect("bounded hostile pagination must not fail");
            let effective = if limit == 0 { 20 } else { limit };
            let expected = 3u32.saturating_sub(offset).min(effective);
            assert_eq!(page.len(), expected);
        }
    });
});
