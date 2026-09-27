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

## AuditConfig

`AuditConfig` controls on-chain audit behavior. Its fields, types, and default
values are part of the stable ABI surface and are documented here so external
consumers can reason about audit trails without reading the contract source.

- **Fields (in encoding order):**
  1. `enabled` (`bool`) — whether audit event recording is active. **Default:** `true`
  2. `retention_epochs` (`u32`) — number of epochs audit records are retained before pruning. **Default:** `30`
  3. `max_events_per_epoch` (`u32`) — upper bound on audit events recorded per epoch. **Default:** `1024`
  4. `require_actor` (`bool`) — whether each audit event must carry a non-empty actor. **Default:** `true`

### Defaults

When no `AuditConfig` has been explicitly stored, the contract MUST behave as if
the defaults above are in effect. Defaults are chosen to keep audit trails on by
default and to bound storage growth; disabling auditing or lowering retention is
an explicit, authorized action.

### Mutation authorization

`AuditConfig` may only be mutated by authorized roles. Unauthorized callers MUST
be rejected and the stored configuration MUST remain unchanged. The authorization
check is enforced in the contract (see `src/audit.rs` / `src/lib.rs`) and is
covered by tests that assert unauthorized mutation attempts are rejected.

When the `AuditConfig` layout or defaults intentionally change, update this
document and the corresponding tests in the same change.
