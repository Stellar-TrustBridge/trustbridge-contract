#![no_main]

//! Fuzz target for `get_public_paginated` cursor decode edge cases (Issue #650 extension).
//!
//! # Input layout
//!
//! ```text
//! byte  0     : cursor variant tag
//!                 0x00 = no cursor (None)
//!                 0x01 = 8-byte cursor (BytesN<8>, the only accepted width)
//!                 0x02 = short cursor (< 8 bytes; should be ignored gracefully)
//!                 0x03 = long cursor (> 8 bytes; should be ignored gracefully)
//!                 anything else = no cursor
//! bytes 1..9  : raw cursor bytes when tag == 0x01 (offset u32 BE at [1..5], generation u32 BE at [5..9])
//! byte  9     : limit byte (0 exercises the DEFAULT_PAGE_LIMIT fallback, >100 exercises clamping)
//! byte  10    : registry size to pre-populate (0..=20, clamped)
//! ```
//!
//! # What this target verifies
//!
//! - No panic on any input byte sequence.
//! - An 8-byte cursor with the live generation and an in-range offset returns `Ok`.
//! - Any other cursor returns `Err(ContractError::InvalidCursor)`.
//! - `limit == 0` never errors (DEFAULT_PAGE_LIMIT fallback).
//! - Limits above MAX_PAGE_LIMIT (100) are clamped, never rejected.
//! - Offset exactly equal to registry count returns an empty page, not an error.
//! - Uninitialized contract returns `Err(ContractError::NotInitialized)`.
//! - `has_more == next_cursor.is_some()` on every valid page.
//!
//! # BytesN width invariant
//!
//! `get_public_paginated` accepts `cursor: Option<BytesN<8>>`. The Soroban
//! type system enforces the width at the ABI boundary — a cursor with any
//! length other than exactly 8 bytes cannot be constructed as `BytesN<8>` and
//! would be rejected before reaching `decode_cursor`. This target covers the
//! 8-byte path and explicitly skips other lengths (mirroring `export_cursor.rs`),
//! documenting that the type boundary is the enforced widths gate.
//!
//! **Cursor wire format** (internal, opaque to callers):
//!   bytes 0..4 — flat-index offset, big-endian u32
//!   bytes 4..8 — index generation, big-endian u32
//! Callers must never construct or parse this; pass back `next_cursor` as-is.

use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, BytesN, Env, String, Vec};
use trustbridge_contract::{ContractError, TrustBridgeContract};

/// Maximum registry size the fuzzer will pre-populate.
const MAX_REGISTRY_SIZE: u32 = 20;

/// Mirrors storage::DEFAULT_PAGE_LIMIT — do not import to keep this target
/// independent of internal storage constants.
const DEFAULT_PAGE_LIMIT: u32 = 20;

/// Mirrors storage::MAX_PAGE_LIMIT.
const MAX_PAGE_LIMIT: u32 = 100;

fuzz_target!(|data: &[u8]| {
    // ── Uninitialized contract (no data needed) ──────────────────────────────
    // Verify that an uninitialized contract always returns NotInitialized
    // regardless of cursor bytes, without panicking.
    {
        let env_uninit = Env::default();
        let contract_uninit = env_uninit.register(TrustBridgeContract, ());
        let result = env_uninit.as_contract(&contract_uninit, || {
            TrustBridgeContract::get_public_paginated(env_uninit.clone(), None, 10)
        });
        assert_eq!(
            result,
            Err(ContractError::NotInitialized),
            "uninitialized contract must return NotInitialized"
        );
    }

    // ── Minimum viable input: at least 2 bytes (tag + limit) ────────────────
    if data.len() < 2 {
        return;
    }

    let cursor_tag = data[0];
    let limit = u32::from(data.last().copied().unwrap_or(1));

    // Registry size to pre-populate, clamped to MAX_REGISTRY_SIZE.
    let registry_size: u32 = data
        .get(10)
        .map(|&b| (b as u32).min(MAX_REGISTRY_SIZE))
        .unwrap_or(3);

    // ── Set up an initialized contract with `registry_size` records ──────────
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
        for i in 0..registry_size {
            let name = alloc::format!("user{i:03}");
            TrustBridgeContract::register(
                env.clone(),
                String::from_str(&env, &name),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        }
    });

    // Current index generation is 0 (no removals).
    let current_generation: u32 = 0;

    // ── Build the cursor from the input layout ───────────────────────────────
    let cursor: Option<BytesN<8>> = match cursor_tag {
        // 0x00: None — always a valid start-from-beginning call.
        0x00 => None,

        // 0x01: 8-byte cursor — the only valid non-None cursor width.
        // Requires at least 9 bytes of input (tag + 8 cursor bytes).
        0x01 => {
            let Ok(cursor_bytes) =
                <[u8; 8]>::try_from(data.get(1..9).unwrap_or_default())
            else {
                return;
            };
            Some(BytesN::from_array(&env, &cursor_bytes))
        }

        // 0x02 / 0x03: shorter or longer than 8 bytes — not representable as
        // BytesN<8> and cannot reach decode_cursor. Skip gracefully.
        // This documents the type-system enforced width boundary.
        0x02 | 0x03 => return,

        // All other tags: treat as no cursor, exercising the None path.
        _ => None,
    };

    // ── Exercise the entry point ─────────────────────────────────────────────
    let result = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), cursor.clone(), limit)
    });

    match (cursor_tag, cursor.as_ref()) {
        // None cursor: always valid — returns first page.
        (0x00, None) | (_, None) => {
            let page = result.expect("None cursor must always return Ok");

            // Bounded page size.
            let effective_limit = if limit == 0 {
                DEFAULT_PAGE_LIMIT
            } else {
                limit.min(MAX_PAGE_LIMIT)
            };
            assert!(
                page.records.len() <= effective_limit,
                "page.records.len() ({}) must not exceed effective_limit ({})",
                page.records.len(),
                effective_limit
            );
            assert_eq!(page.total, registry_size);
            assert_eq!(
                page.has_more,
                page.next_cursor.is_some(),
                "has_more must equal next_cursor.is_some()"
            );
        }

        // 8-byte cursor: valid iff generation matches and offset is in range.
        (0x01, Some(cursor_bytes)) => {
            let raw = cursor_bytes.to_array();
            let offset = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]);
            let generation = u32::from_be_bytes([raw[4], raw[5], raw[6], raw[7]]);

            if generation == current_generation && offset <= registry_size {
                let page = result
                    .expect("in-generation in-range cursor must return Ok");

                let effective_limit = if limit == 0 {
                    DEFAULT_PAGE_LIMIT
                } else {
                    limit.min(MAX_PAGE_LIMIT)
                };
                // Offset equal to registry_size is valid: returns an empty last page.
                let expected_len = (registry_size - offset).min(effective_limit);
                assert_eq!(
                    page.records.len(),
                    expected_len,
                    "offset={offset} limit={limit} effective_limit={effective_limit}"
                );
                assert_eq!(page.total, registry_size);
                assert_eq!(
                    page.has_more,
                    page.next_cursor.is_some(),
                    "has_more must equal next_cursor.is_some()"
                );
            } else {
                assert_eq!(
                    result,
                    Err(ContractError::InvalidCursor),
                    "stale-generation or out-of-range cursor must return InvalidCursor \
                     (generation={generation}, current={current_generation}, \
                     offset={offset}, registry_size={registry_size})"
                );
            }
        }

        _ => {}
    }
});

extern crate alloc;
