#![no_main]

use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, BytesN, Env, String, Vec};
use trustbridge_contract::{ContractError, TrustBridgeContract};

fuzz_target!(|data: &[u8]| {
    // The public export accepts a fixed-width opaque cursor. Rejecting other
    // lengths here mirrors the contract's BytesN<8> input boundary.
    let Ok(cursor_bytes) = <[u8; 8]>::try_from(data.get(..8).unwrap_or_default()) else {
        return;
    };
    let limit = u32::from(data.get(8).copied().unwrap_or(1));

    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin).unwrap();
        for name in ["alice", "bob", "carol"] {
            TrustBridgeContract::register(
                env.clone(),
                String::from_str(&env, name),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        }

        let cursor = BytesN::from_array(&env, &cursor_bytes);
        let result = TrustBridgeContract::get_public_paginated(env.clone(), Some(cursor), limit);
        let offset = u32::from_be_bytes(cursor_bytes[..4].try_into().unwrap());
        let generation = u32::from_be_bytes(cursor_bytes[4..].try_into().unwrap());
        if generation == 0 && offset <= 3 {
            let page = result.expect("valid cursor must return a page");
            let expected = (3 - offset).min(if limit == 0 { 100 } else { limit });
            assert_eq!(page.records.len(), expected);
            assert_eq!(page.total, 3);
            assert_eq!(page.has_more, page.next_cursor.is_some());
        } else {
            assert_eq!(result, Err(ContractError::InvalidCursor));
        }
    });
});
