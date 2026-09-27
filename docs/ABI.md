# ABI Notes

This document tracks the stable ABI surface exposed by the TrustBridge contract,
including storage layout versions and field ordering that external consumers
(notably dashboard export consumers) depend on.

## export_page layout

The `export_page` storage encoding is versioned and pinned by a golden layout
file at `trustbridge-contract/abi/export_page.layout.golden`. The golden file is
the source of truth for the encoded field order and types; the contract's
`src/storage.rs` must encode `export_page` to match it exactly.

- **Layout version:** `1`
- **Fields (in encoding order):**
  1. `page_id`
  2. `owner`
  3. `entries`
  4. `updated_at`

### Drift enforcement

The test suite in `trustbridge-contract/tests/export_page_layout.rs` validates
the current `export_page` storage encoding against
`abi/export_page.layout.golden`. Any change to the encoding that is not reflected
in the golden file causes these tests to fail, so CI fails on layout drift.

When the layout intentionally changes, bump the layout version above, update
`abi/export_page.layout.golden`, and update the expectations in
`tests/export_page_layout.rs` in the same change.
