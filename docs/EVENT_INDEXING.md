# Event Indexing Notes

TrustBridge emits events so dashboards and indexers can build a readable contributor timeline.

## Reference indexer (`scripts/event_indexer.sh`, Issue #288)

A runnable reference indexer now lives in the repo. It polls the Stellar RPC
`getEvents` method, appends events to a local JSONL file, and persists the RPC
pagination cursor to disk so restarts resume exactly where they stopped
(idempotent restart). It de-duplicates on the RPC event `id`, so reorgs and
overlapping re-reads never double-write.

```bash
# follow testnet forever
CONTRACT_ID=C... ./scripts/event_indexer.sh

# drain once and exit (cron / CI)
CONTRACT_ID=C... ONESHOT=1 ./scripts/event_indexer.sh

# offline replay of a canned RPC response — no network
MOCK_RESPONSE=./scripts/testdata/getEvents.sample.json \
  CONTRACT_ID=C_MOCK ONESHOT=1 ./scripts/event_indexer.sh
```

State is three files under `DATA_DIR` (default `./.indexer/`):
`events-<network>.jsonl`, `cursor-<network>.json`, `seen-<network>.txt`. No
cloud account or database is required. See
[`scripts/README.md`](../scripts/README.md#event_indexersh--reference-event-indexing-service)
for the full guide, including how to validate resume behavior locally and a
run against futurenet.

This is a **reference** implementation for local sync — the filter/field
patterns below are the spec it follows; a production hosted indexer is out of
scope for this repo.

## Suggested consumer behavior

- Treat contract storage as the source of truth.
- Treat events as an append-only activity stream.
- Reconcile from storage after missed ledger ranges or indexer downtime.

## Useful event fields

Indexers should capture the GitHub username, Stellar address, verification flag changes, ledger sequence, and transaction hash whenever available from the host environment.

---

## Pause and freeze-window events

Three code paths can pause the contract. Indexers must subscribe to all three
event types to correctly detect freeze windows:

| Entry point | Event emitted | Condition |
|-------------|---------------|-----------|
| `pause` | `PausedEvent` | Always (admin-only) |
| `set_paused(true)` | `PausedEvent` | Only when state changes (idempotent) |
| `emergency_pause` | `EmergencyPausedEvent` | Only when state changes (admin or guardian, idempotent) |
| `unpause` | `UnpausedEvent` | Always (admin-only) |
| `set_paused(false)` | `UnpausedEvent` | Only when state changes (idempotent) |
| `clear_emergency_pause` | `EmergencyClearedEvent` | Only when state changes (admin-only) |

**Important:** `set_paused` previously emitted no event, making freeze windows
invisible to indexers when that path was used. As of Issue #197 it now emits
`PausedEvent` / `UnpausedEvent` on state transitions — identical to `pause` /
`unpause`. Idempotent calls (already-paused → pause again) produce no event and
leave indexer state unchanged.

### Emergency pause circuit breaker (Issue #196)

`EmergencyPausedEvent` carries `triggered_by` (admin or guardian address) and
`timestamp`. Indexers should treat this as equivalent to a normal `PausedEvent`
for freeze-window logic: all mutating operations are blocked while either the
normal pause **or** the emergency pause is active.

`EmergencyClearedEvent` is emitted exclusively by the admin when the emergency
pause is lifted. Only the admin can clear it — the guardian cannot. Use this
event to close the freeze window opened by `EmergencyPausedEvent`.

### Recommended indexer state machine

```
NORMAL ──── PausedEvent / EmergencyPausedEvent ──▶ FROZEN
FROZEN ──── UnpausedEvent ─────────────────────▶ NORMAL  (if no emergency pause)
FROZEN ──── EmergencyClearedEvent ─────────────▶ NORMAL  (if no normal pause)
```

Because both flags are independent, an indexer should track them separately and
consider the contract frozen when **either** is set. Query `is_paused()` and
`is_emergency_paused()` after any indexer gap to reconcile state directly from
the contract rather than relying solely on the event stream.

---

## Event domain separation (Issue #226)

Every event this contract emits carries a `domain` field:

```
EventDomain {
  contract_id:      Address,      // emitting contract instance
  network_id:       BytesN<32>,   // SHA-256 of the network passphrase
  contract_version: (u32,u32,u32),// contract version at emit time
  domain_version:   u32,          // schema version of this envelope (currently 1)
}
```

### The problem it solves

Before this, an event carried only its subject and a timestamp. An indexer
reconciling on `(github_username, event_type, timestamp)` — the only fields it
had — could not distinguish:

- a genuine re-registration of a username, from
- the *same* event re-read out of a redeployed contract's history, from
- an event produced by a different network entirely.

Redeploy and replay, and every historical registration either collapses into a
duplicate of a live record or appears as a brand-new user, depending on how the
indexer breaks the tie. Neither answer is right, and nothing in the payload
lets the indexer tell which it is looking at.

### What consumers should do

**Use `(domain.contract_id, domain.network_id)` as the deduplication scope.**
It is stable for the life of a deployment and differs across redeploys and
networks, which is exactly the distinction that was missing. Key your
idempotency records on it alongside whatever you already key on:

```
dedup_key = (domain.contract_id, domain.network_id, github_username, event_type, ledger_seq)
```

- **After a redeploy**, events from the new instance carry a different
  `contract_id`. Treat them as a separate stream; do not merge them into the
  old instance's timeline unless you have deliberately migrated state.
- **Across networks**, `network_id` differs even when `contract_id` collides.
  Never reconcile a testnet stream into a public-network table.
- **`contract_version`** attributes an event to the build that emitted it,
  which matters when an upgrade changes what an event means. It is read from
  instance storage, so it matches what `get_version` would return at that
  ledger.
- **`domain_version`** versions the envelope itself, not the contract. Branch on
  it if the envelope shape ever changes; it is `1` today. A consumer seeing an
  unknown `domain_version` should treat the remaining fields as
  forward-compatible rather than failing the event.

### Compatibility

This is an **additive** change: existing fields keep their names, types, and
topic assignments, and no event was removed or renamed. Consumers that ignore
unknown fields need no change to keep working — they simply do not get the
deduplication benefit. Consumers that parse events positionally must be updated,
because `domain` is appended to each payload.

---

## Payout delegation vs address rotation vs re-registration

Three different on-chain changes can look similar to an indexer that only
tracks "the address associated with a username changed." They are **not**
the same operation and must not be collapsed into one timeline entry:

| Change | Entry point(s) | Event(s) | What actually moves |
|---|---|---|---|
| Re-registration | `register` / `register_sponsored` | `registered_event` | `stellar_address` (identity) — same path as first-time registration |
| Address rotation | `request_address_rotation` / `execute_address_rotation` | `rotation_requested_event` / `rotation_executed_event` | `stellar_address` (identity), after a configurable delay |
| Payout delegation | `delegate_payout` / `undelegate_payout` | `payout_delegated_event` / `payout_delegation_revoked_event` | `payout_address` only — identity `stellar_address` is untouched |

Before `delegate_payout` existed, the only way to steer payouts to a
different address was implicit: `register`/`register_sponsored` carry
whatever `payout_address` was already on file forward unchanged, and nothing
distinguished "this re-registration happens to also be a rotation" from "this
is a routine re-registration." `delegate_payout` and `undelegate_payout` give
payout-address changes their own entry points and their own events, so an
indexer no longer has to infer intent from a shared `registered_event`.

- **`payout_delegated_event`** — data fields `github_username`,
  `stellar_address` (unchanged identity address), `delegate_address` (the new
  payout destination), `timestamp`, `domain`.
- **`payout_delegation_revoked_event`** — data fields `github_username`,
  `stellar_address`, `previous_delegate`, `timestamp`, `domain`.

**One live delegate at a time.** The contract enforces at most one active
payout delegate per registration: `delegate_payout` fails with
`AlreadyDelegated` (error code 34) if `payout_address` already differs from
`stellar_address` and from the requested new delegate. Callers must
`undelegate_payout` (which fails with `NoActiveDelegate`, code 35, if there is
nothing to revoke) before delegating to a different address. Indexers can
therefore treat "is there a live delegate" as a simple boolean per username —
it never needs to track a list.

**Storage note:** delegation reuses the existing `payout_address` field on
`ContributorRecord` rather than adding a parallel delegate-tracking structure,
so `get_address` / `get_public_paginated` continue to be the source of truth
for the current payout destination — no new read path is needed.
