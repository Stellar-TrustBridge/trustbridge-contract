# Event Indexing Notes

TrustBridge emits events so dashboards and indexers can build a readable contributor timeline.

## Lag detection (Issue #282)

Before polling for new events, call `get_last_event_ledger()` on the contract.
It returns the ledger **sequence** of the most recently emitted event, or `0`
if the instance has not emitted any event yet.

Compare the result against your indexer's persisted watermark (the last fully
applied ledger sequence):

- `watermark < last_event_ledger` → lag detected; fetch and apply events from
  `(watermark+1, last_event_ledger]`, then advance the watermark.
- `watermark >= last_event_ledger` → no known contract lag.

The call is a single cheap instance-storage read. It is available while the
contract is paused, so freeze windows do not block reconciliation. See
[DASHBOARD_SYNC.md](DASHBOARD_SYNC.md#event-lag-signal-issue-282) for the full
polling loop recipe and constraints.

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
