# Event Indexing

Reference guide for tailing TrustBridge contract events into a local store.

Related docs: [README](../README.md) · [ARCHITECTURE](ARCHITECTURE.md) ·
[DASHBOARD_SYNC](DASHBOARD_SYNC.md) · [SECURITY](SECURITY.md) ·
[ADMIN_RUNBOOK](ADMIN_RUNBOOK.md)

---

## Overview

TrustBridge contract state is best consumed through its event stream rather than
repeated full-registry exports. Events are cheaper to read, capture every mutation
atomically, and include enough context (topic symbol, ledger sequence, tx hash) for
idempotent processing.

`scripts/event_indexer.sh` is the reference implementation: a dependency-light bash
poller that tails the Stellar RPC `getEvents` endpoint, writes one JSON object per
event to an append-only JSONL file, persists a cursor between runs, and deduplicates
on the RPC event `id`. It is designed for local dev, Futurenet, and testnet — not a
production hosted service. No cloud account, database, or secret is required; all
state is a small set of files on disk.

---

## Quick Start

```bash
# Follow the event stream on testnet:
CONTRACT_ID=C... ./scripts/event_indexer.sh

# One-shot drain to head (useful in CI or cron):
CONTRACT_ID=C... ONESHOT=1 ./scripts/event_indexer.sh

# Local / Futurenet RPC, start from a known ledger:
CONTRACT_ID=C... RPC_URL=http://localhost:8000/soroban/rpc \
  START_LEDGER=1000 ONESHOT=1 ./scripts/event_indexer.sh

# Offline demo using a canned RPC response (no network required):
MOCK_RESPONSE=./scripts/testdata/getEvents.sample.json \
  CONTRACT_ID=C_MOCK ONESHOT=1 ./scripts/event_indexer.sh
```

---

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CONTRACT_ID` | _(required)_ | Deployed contract ID to filter events for. Not required when `MOCK_RESPONSE` is set. |
| `RPC_URL` | `https://soroban-testnet.stellar.org` | Stellar RPC endpoint. Futurenet: `https://rpc-futurenet.stellar.org` |
| `NETWORK` | `testnet` | Informational tag written into state file names and the cursor JSON. |
| `DATA_DIR` | `./.indexer` | Directory for all indexer state and output files. |
| `START_LEDGER` | _(latest − LOOKBACK)_ | Ledger to start from on a **cold start** (no cursor on disk). Ignored once a cursor exists. |
| `LOOKBACK` | `17280` | Ledgers to rewind from "latest" on a cold start (~1 day at 5 s per ledger). Ignored once a cursor exists. |
| `POLL_SECONDS` | `5` | Seconds to sleep between polls when following the live stream. |
| `PAGE_LIMIT` | `100` | Events requested per RPC page (RPC max is 10 000). |
| `ONESHOT` | _(unset)_ | Set to `1` to do a single drain to head then exit 0. Useful for CI and cron. |
| `MOCK_RESPONSE` | _(unset)_ | Path to a canned `getEvents` JSON-RPC response. When set, no network call is made and the loop runs exactly once. |
| `MAX_RETRIES` | `5` | Consecutive RPC failures tolerated before giving up. |

---

## Files Written Under `DATA_DIR`

All three files are safe to delete:

| File | Purpose |
|------|---------|
| `events-<network>.jsonl` | Append-only event log. One JSON object per line. |
| `cursor-<network>.json` | Pagination cursor and progress counters. Removing this forces a cold re-scan. |
| `seen-<network>.txt` | Recent event ids for dedup across restarts. Bounded at `SEEN_MAX_LINES` (50 000). Removing it only weakens dedup for the overlap window. |

### Cursor file schema

```json
{
  "network": "testnet",
  "cursor": "<opaque RPC cursor string or null>",
  "last_ledger": 12345678,
  "last_id": "<most recent event id or null>",
  "updated_at": "2026-09-29T12:00:00Z",
  "event_count": 42
}
```

### Event log record shape

Each line of `events-<network>.jsonl` is a normalized JSON object:

```json
{
  "id": "<rpc event id>",
  "ledger_sequence": 12345678,
  "ledger_closed_at": "2026-09-29T12:00:00Z",
  "contract_id": "C...",
  "tx_hash": "<transaction hash or null>",
  "type": "contract",
  "topic": ["<base64 ScVal>", "..."],
  "topic_symbol": "registered_event",
  "event_kind": "registered",
  "category": "registry",
  "value": "<base64 ScVal>",
  "in_successful_contract_call": true,
  "indexed_at": "2026-09-29T12:00:00Z"
}
```

The `topic_symbol`, `event_kind`, and `category` fields are decoded from `topic[0]`
by the indexer so a dashboard can filter by event kind without decoding XDR itself.
See [Topic Classification](#topic-classification) below.

---

## Cold Start vs. Cursor Resume

### Cold start (no cursor on disk)

1. The indexer calls `getLatestLedger` to find the current chain head.
2. It subtracts `LOOKBACK` (default 17 280 ≈ 24 h) to compute `start_ledger`, clamped to 1.
3. If `START_LEDGER` is explicitly set, that value overrides the computed start.
4. The first `getEvents` call includes `startLedger: <start_ledger>` and no cursor.

### Cursor resume (cursor file exists)

1. The stored cursor is read from `cursor-<network>.json`.
2. All subsequent `getEvents` calls pass `pagination.cursor: <cursor>` and omit
   `startLedger` — the cursor already encodes the position.
3. If the RPC rejects the cursor (e.g. it has been pruned from the node's history),
   the indexer falls back to a cold start and logs the event.

### Idempotent restart

Restarting the indexer mid-batch is always safe:

- Any event whose `id` already appears in `seen-<network>.txt` is skipped.
- The cursor is written atomically after each page (via a `mktemp` + `mv` rename),
  so a crash mid-write cannot corrupt it.
- Removing the cursor forces a full re-scan; removing `seen-*.txt` only widens the
  dedup window slightly at the overlap.

---

## Topic Classification

Every `#[contractevent]` struct in `src/events.rs` derives its first topic as the
struct name in `snake_case` (e.g. `RegisteredEvent` → `"registered_event"`). The
indexer maps these symbols to a stable `event_kind` and a coarse `category` so
consumers can filter by category without parsing XDR.

The full table lives in `scripts/event_indexer.sh` (`TOPIC_TABLE`).
`scripts/check_event_topics.sh` fails CI if the table falls out of step with
`src/events.rs`; it's the source of truth for the check — don't duplicate its
logic elsewhere.

### Collision failure messages (Issue #794)

If two event structs derive the same snake_case topic (Issue #643 — e.g. a
struct that isn't PascalCase, such as `fooEvent`, colliding with `FooEvent`),
the CI failure names **both** colliding structs with their exact
`src/events.rs:<line>` location and suggests the fix, instead of only
printing the bare topic string:

```
event-topics: duplicate event topics derived from src/events.rs (two structs map to the same topic):
  topic "foo_event"                    collides between:
    - FooEvent (src/events.rs:12)
    - fooEvent (src/events.rs:48)
event-topics: rename one of the colliding structs so their snake_case names differ.
```

`scripts/test_check_event_topics_message.sh` (`make event-topics-message-test`)
pins this message format against a fixture collision so a future edit to the
script can't silently regress back to a bare topic name with no location.

The table below lists **every** published topic (Issue #754). The **Subgraph**
column records how [`docs/subgraph/README.md`](subgraph/README.md) handles it:

- `` `EntityName` ``: mapped 1:1 to a Graph entity (see *Event → entity mapping*)
- `waived`: deliberately not modelled in the subgraph, with a reason in the
  README's *Explicit subgraph event waivers* table. Consumers still get it from
  this raw indexer. Operational signals such as pause, upgrade and index
  generation fall in this group, so watch them here rather than in the subgraph.

`make subgraph-parity` (`scripts/check_subgraph_parity.py`, run in CI) fails if a
topic is missing from or stale in this table, if `event_kind` / `category` drift
from `TOPIC_TABLE`, or if the Subgraph column disagrees with the README.

| Topic symbol | `event_kind` | `category` | Subgraph |
|---|---|---|---|
| `registered_event` | `registered` | `registry` | `RegisteredEvent` |
| `removed_event` | `removed` | `registry` | `RemovedEvent` |
| `renamed_event` | `renamed` | `registry` | `RenamedEvent` |
| `bot_status_changed_event` | `bot_status_changed` | `registry` | `BotStatusChangedEvent` |
| `verified_event` | `verified` | `attest` | `VerifiedEvent` |
| `verification_revoked_event` | `verification_revoked` | `attest` | `VerificationRevokedEvent` |
| `verification_configured_event` | `verification_configured` | `attest` | waived |
| `challenge_started_event` | `challenge_started` | `challenge` | `ChallengeStartedEvent` |
| `challenge_cancelled_event` | `challenge_cancelled` | `challenge` | `ChallengeCancelledEvent` |
| `challenge_completed_event` | `challenge_completed` | `challenge` | `ChallengeCompletedEvent` |
| `role_granted_event` | `role_granted` | `role` | waived |
| `role_revoked_event` | `role_revoked` | `role` | waived |
| `role_grant_pending_event` | `role_grant_pending` | `role` | `RoleGrantPendingEvent` |
| `role_grant_cancelled_event` | `role_grant_cancelled` | `role` | waived |
| `paused_event` | `paused` | `admin` | waived |
| `unpaused_event` | `unpaused` | `admin` | waived |
| `emergency_paused_event` | `emergency_paused` | `admin` | waived |
| `emergency_cleared_event` | `emergency_cleared` | `admin` | waived |
| `guardian_changed_event` | `guardian_changed` | `admin` | waived |
| `rotation_requested_event` | `rotation_requested` | `admin` | `RotationRequestedEvent` |
| `rotation_executed_event` | `rotation_executed` | `admin` | `RotationExecutedEvent` |
| `rotation_cancelled_event` | `rotation_cancelled` | `admin` | `RotationCancelledEvent` |
| `upgraded_event` | `upgraded` | `upgrade` | waived |
| `upgrade_attested_event` | `upgrade_attested` | `upgrade` | waived |
| `attestation_cleared_event` | `attestation_cleared` | `upgrade` | waived |
| `wasm_staged_event` | `wasm_staged` | `upgrade` | waived |
| `staged_wasm_cleared_event` | `staged_wasm_cleared` | `upgrade` | waived |
| `upgrade_proposed_event` | `upgrade_proposed` | `upgrade` | waived |
| `upgrade_approved_event` | `upgrade_approved` | `upgrade` | waived |
| `upgrade_proposal_executed_event` | `upgrade_proposal_executed` | `upgrade` | waived |
| `upgrade_proposal_cancelled_event` | `upgrade_proposal_cancelled` | `upgrade` | waived |
| `batch_remove_proposed_event` | `batch_remove_proposed` | `batch` | waived |
| `batch_remove_executed_event` | `batch_remove_executed` | `batch` | waived |
| `batch_remove_cancelled_event` | `batch_remove_cancelled` | `batch` | waived |
| `audit_config_changed_event` | `audit_config_changed` | `audit` | waived |
| `oracle_allowlist_rotated_event` | `oracle_allowlist_rotated` | `admin` | waived |

### `AttestationClearedEvent` payload

`src/events.rs` defines this event with the following fields; keep decoders
aligned with these names, types, and topic annotations:

| Field | Type | Location | Meaning |
|---|---|---|---|
| `wasm_hash` | `BytesN<32>` | Topic (`topic[1]`) | Hash of the WASM whose pending attestation was withdrawn |
| `expires_at` | `u64` | Event value | Expiration timestamp on the withdrawn attestation |
| `timestamp` | `u64` | Event value | Ledger timestamp when it was cleared |

The raw event's `topic[0]` is the symbol `attestation_cleared_event`; the
contract-event decoder reads `wasm_hash` from `topic[1]` and the value tuple in
declaration order as `expires_at`, then `timestamp`. In the normalized JSONL
record, the indexer keeps topics and `value` as base64 ScVals, so clients that
need typed fields must decode the XDR rather than assuming `value` is JSON.
For example, the decoded shape is:

```text
topics: [Symbol("attestation_cleared_event"), BytesN<32>(wasm_hash)]
value:  (expires_at: u64, timestamp: u64)
```

Dashboards should clear the pending-attestation indicator for the matching
contract and `wasm_hash`, using the stable event id for replay deduplication.
This event is emitted only when a stored attestation is actively removed. It
does **not** disable attestation-required mode; that setting changes only via
`set_attestation_required`. The event remains waived from the contributor
subgraph and is available through the raw event stream.

An event whose `topic[0]` does not match any row is written with
`event_kind: "unknown"` and `category: "unclassified"`. This is intentional: an
indexer pointed at a newer contract version must keep recording events it does not
recognise — losing the stream entirely at the moment an unknown event appears would
be worse than a gap in classification.

### Adding a new event kind

1. Add the `#[contractevent]` struct to `src/events.rs`.
2. Add a row to the `TOPIC_TABLE` in `scripts/event_indexer.sh`:
   `<topic_symbol>|<event_kind>|<category>`
3. Add a row to the table above, with the Subgraph column set to the mapped
   entity or `waived`.
4. Add either a mapping row (plus a `schema.graphql` entity) or a waiver row with
   a reason to [`docs/subgraph/README.md`](subgraph/README.md).
5. `scripts/check_event_topics.sh` and `make subgraph-parity` fail CI until all
   sides agree (`python3 scripts/check_subgraph_parity.py --self-test` proves the
   drift cases are caught).

---

## Mock Mode (Offline Testing)

Set `MOCK_RESPONSE` to a path containing a canned JSON-RPC `getEvents` response.
The indexer will process it exactly once and exit, with no network calls and no
`CONTRACT_ID` requirement.

```bash
MOCK_RESPONSE=./scripts/testdata/getEvents.sample.json \
  CONTRACT_ID=C_MOCK ONESHOT=1 ./scripts/event_indexer.sh
```

The sample fixture in `scripts/testdata/getEvents.sample.json` contains a small set
of representative events for local smoke-testing. It is the same fixture referenced
by the offline example in `docs/DASHBOARD_SYNC.md`.

---

## Retry and Backoff

The indexer retries on transient RPC failures up to `MAX_RETRIES` (default 5)
consecutive failures before giving up with exit code 1:

- Each retry sleeps for `POLL_SECONDS × retry_count` seconds (linear backoff).
- A pruned cursor triggers a one-time cold-start fallback rather than counting as a
  failure.
- Success on any page resets the consecutive-failure counter.

Operators running the indexer as a cron job or systemd service should treat exit
code 1 as an alert and check `RPC_URL` availability.

---

## Relation to `get_last_event_ledger`

`TrustBridgeContract::get_last_event_ledger()` returns the ledger sequence of the
most recently emitted event. An indexer can compare its own ingestion watermark
(the `last_ledger` field in the cursor file) against this value to detect lag
without an additional RPC call. A watermark equal to the contract's value means the
indexer is caught up; a lower value means there are unprocessed events.

This is documented in the ABI as a monitoring primitive — see
[ABI.md](ABI.md) and the `get_last_event_ledger` entry point in `src/lib.rs`.

---

## Relation to Full Registry Exports

For bulk registry reads, prefer `get_public_paginated` (no auth, pause-exempt) over
`get_all_registered` (admin-only, linear scan). For ongoing sync, the event stream
is cheaper and more precise than repeated full exports:

| Approach | Auth | Works while paused | Cost |
|---|---|---|---|
| `get_all_registered` | Admin required | Yes (admin-gated) | O(n) per call |
| `get_public_paginated` | None | Yes (Issue #294) | O(page) per call |
| Event stream (`getEvents`) | None | Events are permanent on-chain | O(new events only) |

The event stream does not replace paginated reads for an initial cold sync of a
large registry — use `get_public_paginated` to bootstrap, then switch to the event
stream to stay current.

---

## Dashboard Integration

The `DASHBOARD_SYNC.md` document describes how a dashboard should combine the event
stream with paginated exports to maintain a consistent local mirror. Key points:

- Idempotency key for upserts: `(ledger_sequence, tx_hash)` — this pair is stable
  across restarts and re-reads.
- Dedup on `event.id` (the RPC field), not on `(ledger_sequence, tx_hash)` alone —
  one transaction can emit multiple events of the same type.
- Use `topic_symbol` from the normalized record (already decoded by the indexer) to
  route events to the correct handler without re-decoding `topic[0]`.
- `category: "batch"` events (`batch_remove_proposed`, `batch_remove_executed`,
  `batch_remove_cancelled`) indicate bulk removals; reconcile `removed_event` records
  emitted in the same transaction to update the local mirror atomically.

---

## Pending Re-verification Signals and Poll API

**No dedicated event is emitted when the `pending_reverify` flag
(`PENDING_REVERIFY_KEY` / `pend_rev`) is set or cleared.** `src/events.rs`
defines no `pending_reverify_*` event, and none of the lifecycle events carry a
`pending_reverify` field.

### Why event-only tracking is insufficient

Existing lifecycle events only provide indirect hints:

| Action | Storage effect on `pend_rev` | Event emitted | Why events alone are not authoritative |
|---|---|---|---|
| `register` / `register_sponsored` (address change on verified user) | Sets `pend_rev = true` | `registered_event` | Payload omits prior `verified` state, so indexers cannot tell from the event alone whether reverify was set without prior state |
| `execute_address_rotation` (on verified user) | Sets `pend_rev = true` | `rotation_executed_event` | Payload omits prior `verified` state |
| `rename` | Migrates existing `pend_rev` or sets it if `verification_cleared` | `renamed_event` | `verification_cleared` is `false` if the old handle was already unverified with `pend_rev = true`, yet the flag still moved to `new_username` |
| `verify` / `batch_verify` / `verify_with_oracle_proof` | Clears `pend_rev` | `verified_event` | Fires on initial verify too, not only reverify clearance |
| `remove` / `batch_remove` / `complete_challenge` | Clears `pend_rev` | `removed_event` | Fires on all removals regardless of `pend_rev` |

### Recommended poll API

Wave reverify pipelines and indexers must poll the read-only, unauthenticated,
pause-exempt contract entrypoint:

```bash
stellar contract invoke --id $CONTRACT_ID --network testnet \
  -- get_pending_reverify --offset 0 --limit 100
```

- Start at `offset = 0`, `limit = 100` (`MAX_PAGE_LIMIT`; `limit = 0` uses
  `DEFAULT_PAGE_LIMIT = 20`) and advance `offset += limit` until
  `offset >= get_stats().total`.
- Check `get_health_probes().index_gen` before and after a multi-page poll; if
  `index_gen` changed mid-scan (from a `remove` or `rename`), restart from
  `offset = 0`.
- Handle the three documented `Fatal` errors: `NotInitialized` (`2`),
  `InvalidBatchSize` (`14`, `limit > 100`), and `NetworkMismatch` (`30`). See
  [ABI.md § Error codes & integrator notes for `get_pending_reverify`](ABI.md#error-codes--integrator-notes-for-get_pending_reverify).

---

## Security Considerations

- The indexer reads events but never submits transactions. It holds no signing key.
- `RPC_URL` should be an HTTPS endpoint; plain `http://` is acceptable only for
  local dev.
- `events-<network>.jsonl` contains public on-chain data. Treat it as untrusted
  input when feeding it into downstream systems.
- A high-volume `registered_event` or `verified_event` burst does not indicate an
  attack at the indexer layer — it may indicate a wave or a compromised Verifier key.
  Cross-reference with the `category: "role"` stream to correlate role changes.
- See [SECURITY.md](SECURITY.md) for the full threat model, including the
  Verifier rate-limiting control (Issue #292) that caps burst writes per ledger.

---

## Operational Runbook

### Starting the indexer

```bash
# Copy and fill in env vars:
cp .env.example .env
source .env

# Follow forever (background):
CONTRACT_ID=$CONTRACT_ID nohup ./scripts/event_indexer.sh \
  >> .indexer/indexer.log 2>&1 &
echo $! > .indexer/indexer.pid

# Or as a systemd service — see docs/DEPLOYMENT.md.
```

### Checking indexer lag

```bash
# Current chain head vs. indexer watermark:
LAST_LEDGER=$(jq '.last_ledger' .indexer/cursor-testnet.json)
CHAIN_HEAD=$(curl -s "$RPC_URL" -X POST \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' \
  | jq '.result.sequence')
echo "Lag: $((CHAIN_HEAD - LAST_LEDGER)) ledgers"
```

### Resetting to re-index from scratch

```bash
# Remove cursor to force cold start (events file is preserved):
rm .indexer/cursor-testnet.json .indexer/seen-testnet.txt

# Remove everything including the event log:
rm -rf .indexer/
```

### Recovering from a pruned cursor

The indexer detects a pruned cursor automatically and falls back to a cold start,
logging:

```
[...] stored cursor rejected by RPC — falling back to cold start
```

No operator action is required unless you need the gap filled. In that case, archive
the current `events-<network>.jsonl`, remove the cursor, and re-run from a
`START_LEDGER` that predates the gap.
