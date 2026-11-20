#![no_main]

//! Fuzz target for `get_public_paginated` / `get_registered_paginated` cursor
//! decode (Issue #215, extended in Issue #650).
//!
//! # BytesN<8> width invariant
//!
//! Both paginated entry points accept `cursor: Option<BytesN<8>>`. The
//! Soroban type system enforces the width at the ABI boundary — a value with
//! any length other than exactly **8 bytes** cannot be expressed as `BytesN<8>`
//! and is rejected before it reaches `decode_cursor`. This target therefore
//! only exercises the 8-byte path and explicitly returns early for other
//! lengths, documenting the type-level width gate rather than trying to
//! circumvent it.
//!
//! **max_len context:** `cargo fuzz run export_cursor -- -max_len=9` gives the
//! fuzzer exactly one byte of cursor (8) plus one byte for the limit field.
//! Raising `-max_len` beyond 9 does not change the cursor coverage (the target
//! only reads bytes 0..8 as cursor and byte 8 as limit), but it allows the
//! engine to evolve richer input distributions. Run with `-max_len=32` or
//! higher when doing a local soak; CI uses `-max_len=9` for speed.
//!
//! # Input layout (unchanged from original; extended oracle below)
//!
//! ```text
//! bytes 0..8   cursor bytes for BytesN<8>
//!                 offset     = u32::from_be_bytes(bytes[0..4])
//!                 generation = u32::from_be_bytes(bytes[4..8])
//! byte  8      limit (u8 → u32; 0 exercises DEFAULT_PAGE_LIMIT fallback;
//!                              values > 100 exercise MAX_PAGE_LIMIT clamping)
//! ```
//!
//! # Oracle (extended)
//!
//! Precondition: 3-record fixture (alice, bob, carol), index generation = 0.
//!
//! | generation | offset | Expected result |
//! |:----------:|:------:|:----------------|
//! | 0          | 0..=3  | `Ok` page with `(3-offset).min(effective_limit)` records |
//! | 0          | > 3    | `Err(InvalidCursor)` — offset past registry count |
//! | non-0      | any    | `Err(InvalidCursor)` — stale generation |
//!
//! Limit edge cases verified on every `Ok` path:
//! - `limit == 0`   → effective_limit = 20 (DEFAULT_PAGE_LIMIT)
//! - `limit == 100` → effective_limit = 100 (MAX_PAGE_LIMIT, exact boundary)
//! - `limit > 100`  → effective_limit = 100 (clamped, never an error)
//! - `has_more == next_cursor.is_some()` on every valid page
//! - `page.total == 3` on every valid page

use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, BytesN, Env, String, Vec};
use trustbridge_contract::{ContractError, TrustBridgeContract};

/// Mirrors storage::DEFAULT_PAGE_LIMIT — kept inline to avoid importing
/// internal storage constants into the fuzz crate.
const DEFAULT_PAGE_LIMIT: u32 = 20;

/// Mirrors storage::MAX_PAGE_LIMIT.
const MAX_PAGE_LIMIT: u32 = 100;

/// Number of records in the fixture.
const FIXTURE_COUNT: u32 = 3;

/// Index generation after setup (no removals have occurred).
const LIVE_GENERATION: u32 = 0;

fuzz_target!(|data: &[u8]| {
    // Require exactly 8 bytes of cursor + 1 byte of limit = 9 bytes minimum.
    // The BytesN<8> contract type only accepts 8-byte cursors; shorter inputs
    // cannot construct a valid cursor — return early to mirror that gate.
    let Ok(cursor_bytes) = <[u8; 8]>::try_from(data.get(..8).unwrap_or_default()) else {
        return;
    };

    // Byte 8: limit field. Absent → 0 (triggers DEFAULT_PAGE_LIMIT path).
    let limit = u32::from(data.get(8).copied().unwrap_or(0));

    // ── Build fixture ────────────────────────────────────────────────────────
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
    });

    // ── Decode the raw cursor fields ─────────────────────────────────────────
    let offset = u32::from_be_bytes([
        cursor_bytes[0],
        cursor_bytes[1],
        cursor_bytes[2],
        cursor_bytes[3],
    ]);
    let generation = u32::from_be_bytes([
        cursor_bytes[4],
        cursor_bytes[5],
        cursor_bytes[6],
        cursor_bytes[7],
    ]);

    let cursor = BytesN::from_array(&env, &cursor_bytes);

    // ── Invoke both paginated paths with the same cursor ─────────────────────
    // Both `get_public_paginated` and `get_registered_paginated` share the
    // same `decode_cursor` implementation; exercising both here ensures the
    // fuzz coverage applies to the full dual-path surface.
    let pub_result = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_public_paginated(env.clone(), Some(cursor.clone()), limit)
    });
    let adm_result = env.as_contract(&contract_id, || {
        TrustBridgeContract::get_registered_paginated(env.clone(), Some(cursor.clone()), limit)
    });

    // ── Oracle ───────────────────────────────────────────────────────────────
    if generation == LIVE_GENERATION && offset <= FIXTURE_COUNT {
        // Valid cursor: both paths must succeed with bounded, consistent pages.
        let effective_limit = if limit == 0 {
            DEFAULT_PAGE_LIMIT
        } else {
            limit.min(MAX_PAGE_LIMIT)
        };
        let expected_records = (FIXTURE_COUNT - offset).min(effective_limit);

        let pub_page = pub_result.expect("valid cursor must return Ok on public path");
        assert_eq!(
            pub_page.records.len(),
            expected_records,
            "public: offset={offset} limit={limit} effective={effective_limit}"
        );
        assert_eq!(pub_page.total, FIXTURE_COUNT);
        assert_eq!(
            pub_page.has_more,
            pub_page.next_cursor.is_some(),
            "public: has_more must equal next_cursor.is_some()"
        );

        let adm_page = adm_result.expect("valid cursor must return Ok on admin path");
        assert_eq!(
            adm_page.records.len(),
            expected_records,
            "admin: offset={offset} limit={limit} effective={effective_limit}"
        );
        assert_eq!(adm_page.total, FIXTURE_COUNT);
        assert_eq!(
            adm_page.has_more,
            adm_page.next_cursor.is_some(),
            "admin: has_more must equal next_cursor.is_some()"
        );

        // Both paths must agree on record count, total, and has_more.
        assert_eq!(
            pub_page.records.len(),
            adm_page.records.len(),
            "public and admin paths must return the same record count"
        );
    } else {
        // Invalid cursor: stale generation or offset beyond registry count.
        // Both paths must return InvalidCursor and must NOT panic.
        assert_eq!(
            pub_result,
            Err(ContractError::InvalidCursor),
            "public: stale/out-of-range cursor must return InvalidCursor \
             (generation={generation}, offset={offset})"
        );
        assert_eq!(
            adm_result,
            Err(ContractError::InvalidCursor),
            "admin: stale/out-of-range cursor must return InvalidCursor \
             (generation={generation}, offset={offset})"
        );
    }
});
