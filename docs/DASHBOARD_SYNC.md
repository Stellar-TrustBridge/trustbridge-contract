# Dashboard & Indexer Sync Guide

The TrustBridge dashboard and indexer consumers combine Soroban contract state with Horizon API checks to ensure secure, efficient payout readiness and contributor index synchronization.

## ABI Event Reference

All contract events are documented with their topic and data field layouts in
[docs/ABI.md#events](ABI.md#events). Indexers should use that section as the
source of truth for topic symbols, field names, and types — mismatches between
docs and on-chain `#[contractevent]` definitions are tracked as documentation
bugs.

Key events to watch:

| Event | Topic symbol | Key data fields |
|---|---|---|
| `RegisteredEvent` | `registered_event` | `stellar_address`, `timestamp` |
| `VerifiedEvent` | `verified_event` | `stellar_address`, `timestamp` |
| `VerificationRevokedEvent` | `verification_revoked_event` | `stellar_address`, `timestamp` |
| `RemovedEvent` | `removed_event` | `stellar_address`, `timestamp` |
| `UpgradedEvent` | `upgraded_event` | `version`, `timestamp` |
| `PausedEvent` / `UnpausedEvent` | `paused_event` / `unpaused_event` | `timestamp` |
| `RoleGrantedEvent` / `RoleRevokedEvent` | `role_granted_event` / `role_revoked_event` | `role`, `admin`, `timestamp` / `admin`, `timestamp` |

> **Note:** `RoleRevokedEvent` does **not** include the `role` field in its data
> payload. If your indexer needs to know which role was revoked, correlate the
> revocation with the most recent `RoleGrantedEvent` for that address.

## Features & Integration Overview

1. **Chunked Username Index (Issue #2)**: Contributor usernames are stored in chunked persistent vectors (50 items per chunk) to avoid storage entry size limits at scale.
2. **Paginated Cursor Export (Issue #1)**: Export endpoints (`get_registered_paginated` and `get_public_paginated`) accept a zero-based offset `cursor` and item count `limit` to retrieve records deterministically without exceeding gas or frame limits.
3. **Hardened Public Reads & Emergency Pause (Issue #3)**: `get_public_paginated` allows unauthenticated dashboard reads with capped limits (`MAX_PAGE_LIMIT = 100`) and enforces emergency contract pause states.
4. **Makefile Admin Invoke Targets (Issue #30)**: Convenient CLI commands for operators to query and manage registry state.

Contract verification proves the registry entry was approved; Horizon readiness proves the address can receive the selected asset.

## has_record lookup optimization (Wave #40)

`has_record(github_username) -> bool` is now exposed as a contract entry
point. Dashboard and indexer consumers that only need an existence check
(e.g. "is this username already registered?" during a form validation, or a
membership check while paging through webhook events) should call it instead
of `get_address`:

- `has_record` avoids deserializing the full `ContributorRecord`.
- `get_address` should still be used whenever the caller actually needs
  `stellar_address`, `registered_at`, or `verified`.

Tests for this behavior live alongside the contract in `src/lib.rs`
(`test_has_record_reflects_registration_state`) and `src/storage.rs`
(`test_has_record_true_after_set_record`).

## Paginated registry reads (Wave #41 / Issue #143)

`get_all_registered` returns the entire index in one call, which doesn't
scale as the registry grows. Use `get_registered_page(offset, limit)`
instead when syncing incrementally — it walks the same admin-gated index but
in bounded chunks, so a dashboard/indexer sync job can page through without
risking a resource-limit failure on a large registry. See
`test_get_registered_page_paginates_and_gates_on_admin` in `src/lib.rs`.

## Event Idempotency & Replay Handling

Issue #135: Horizon/RPC replays and worker retries are normal operating conditions, not
failure modes. An indexer that treats every delivery as new will double-count
registrations or resurrect a contributor after they were removed. This
section is the spec for handling replays, gaps, and duplicate deliveries of
`RegisteredEvent`, `VerifiedEvent`, `VerificationRevokedEvent`, and
`RemovedEvent` without corrupting off-chain state.

### Idempotency key

None of the contract's events carry a sequence number in their payload — see
`src/events.rs`. The payload alone (`github_username`, `stellar_address`,
`timestamp`) is not unique: the same contributor can be registered, removed,
and re-registered, producing multiple `RegisteredEvent`s with the same topic.

The uniqueness comes from the delivery envelope Horizon/RPC attaches to every
event, not from the contract payload. Key every stored event on:

```
(github_username, event_type, ledger_sequence, tx_hash)
```

- `event_type` — the event's topic symbol (`registered_event`,
  `verified_event`, `verification_revoked_event`, `removed_event`, …)
- `ledger_sequence` — the ledger the event was emitted in; also the ordering
  key (see "Out-of-order handling" below)
- `tx_hash` — the transaction hash that emitted it; distinguishes two events
  of the same type for the same username emitted in different transactions
  within the same ledger

`(ledger_sequence, tx_hash)` alone is sufficient to deduplicate a single
delivery; `github_username` and `event_type` are included so a lookup by
contributor doesn't require a join back to raw envelope data.

### Event → action → duplicate handling

| Event | Expected indexer action | Duplicate delivery |
|-------|--------------------------|---------------------|
| `RegisteredEvent` | Upsert `(github_username → stellar_address)`; reset local `verified` to `false` unless a `VerifiedEvent` for the same `(ledger_sequence, tx_hash)` ordering is already applied | No-op — same key already applied, re-applying the same payload is a harmless overwrite |
| `VerifiedEvent` | Set local `verified = true` for `github_username` | No-op if the key was already applied; if `verified` is already `true`, applying it again is still a safe overwrite |
| `VerificationRevokedEvent` | Set local `verified = false` for `github_username` | Same as above — idempotent overwrite |
| `RemovedEvent` | Delete (or tombstone) the local record for `github_username` | No-op if already deleted/tombstoned |

Applying the same event twice is always safe **provided duplicates are
detected by key first** — every action above is a last-write-wins overwrite
on a single field or record, not an increment or counter update. Never
implement indexer-side counters (e.g. "times verified") by counting event
occurrences; use `get_stats()` / `get_public_paginated` reads against the
contract as the source of truth for aggregate counts instead.

### Out-of-order handling

Horizon delivery order is not guaranteed to match ledger order under replay
or catch-up conditions. Two rules keep out-of-order delivery from producing
the wrong final state:

1. **Order by `(ledger_sequence, tx_hash-relative-order)` before applying,
   not by delivery order.** If a `RemovedEvent` and a later `RegisteredEvent`
   for the same username arrive out of order, applying them in delivery order
   instead of ledger order can leave the record deleted when it should exist
   (or vice versa).
2. **Track the last-applied `ledger_sequence` per `github_username`.** Before
   applying an event, compare its `ledger_sequence` to the last one recorded
   for that username. If the incoming event is older, it is a gap-fill or a
   late replay of something already superseded — record it for audit purposes
   but do not let it overwrite newer state.

For gaps (a missing ledger range in the delivery stream), reconcile against
on-chain state directly rather than waiting for the missing event: call
`get_public_paginated` (or `get_address` for a single username) and treat its
result as authoritative. The event stream is a change-notification
optimization; the contract's own storage is always the ground truth.

### Example: duplicate delivery fixture

```json
{
  "event_type": "verified_event",
  "github_username": "octocat",
  "stellar_address": "GABCDEF...",
  "timestamp": 1732800000,
  "ledger_sequence": 1000042,
  "tx_hash": "a1b2c3..."
}
```

A replay test in the sibling indexer repo applies this record twice and
asserts the local `verified` flag is `true` exactly once — i.e. the second
apply is detected as a duplicate by `(ledger_sequence, tx_hash)` and produces
no state change, no duplicate row, and no double count in any aggregate.

See [ABI.md — Events](ABI.md#events) for the full topic/payload reference per
event type.

---

## Pending Re-verification (Issue #208)

When a contributor who was previously **verified** re-registers to a **different
Stellar address**, the contract:

1. Clears their `verified` flag and decrements the verified count.
2. Sets a `pending_reverify` flag for that username in persistent storage.

This flag signals that a new off-chain GitHub identity check is required — the
new Stellar address has not yet been linked to the GitHub account.

### Reading pending-reverify state

Two public read endpoints expose this flag (no auth required, work while paused):

| Endpoint | Use |
|---|---|
| `get_pending_reverify(github_username)` | Check a single username — returns `bool` |
| `get_pending_reverify_page(offset, limit)` | Paginated scan — returns `Vec<String>` of usernames with the flag set |

`get_pending_reverify` returns `false` for:
- Usernames that have never been registered.
- Usernames that have been removed.
- Usernames whose flag was cleared (because they were successfully re-verified).

### Dashboard sync workflow

1. **On `RegisteredEvent`** where the old and new `stellar_address` differ, call
   `get_pending_reverify(username)` to confirm the flag was set. Queue the
   contributor for a re-verification workflow.
2. **On `VerifiedEvent`**, the flag is cleared automatically — no additional call needed.
3. **Periodic reconciliation**: Call `get_pending_reverify_page(0, 100)` to build
   the full list of contributors awaiting re-check. This is cheaper than scanning
   all registrations individually and is authoritative.

### Relationship to verification flow

```
register(new_address)          →  pending_reverify = true
verify(username)               →  pending_reverify cleared (flag removed)
remove(username)               →  record deleted (flag irrelevant)
```

The flag is **write-once per address-change cycle** — re-calling `register`
with the same new address (e.g. to extend TTL) does not re-set the flag if
it was already cleared by `verify`.
