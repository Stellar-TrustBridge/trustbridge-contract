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
`src/events.rs`.

| Topic symbol | `event_kind` | `category` |
|---|---|---|
| `registered_event` | `registered` | `registry` |
| `removed_event` | `removed` | `registry` |
| `renamed_event` | `renamed` | `registry` |
| `verified_event` | `verified` | `attest` |
| `verification_revoked_event` | `verification_revoked` | `attest` |
| `verification_configured_event` | `verification_configured` | `attest` |
| `challenge_started_event` | `challenge_started` | `challenge` |
| `challenge_cancelled_event` | `challenge_cancelled` | `challenge` |
| `challenge_completed_event` | `challenge_completed` | `challenge` |
| `role_granted_event` | `role_granted` | `role` |
| `role_revoked_event` | `role_revoked` | `role` |
| `role_grant_pending_event` | `role_grant_pending` | `role` |
| `role_grant_cancelled_event` | `role_grant_cancelled` | `role` |
| `paused_event` | `paused` | `admin` |
| `unpaused_event` | `unpaused` | `admin` |
| `emergency_paused_event` | `emergency_paused` | `admin` |
| `emergency_cleared_event` | `emergency_cleared` | `admin` |
| `guardian_changed_event` | `guardian_changed` | `admin` |
| `rotation_requested_event` | `rotation_requested` | `admin` |
| `rotation_executed_event` | `rotation_executed` | `admin` |
| `rotation_cancelled_event` | `rotation_cancelled` | `admin` |
| `upgraded_event` | `upgraded` | `upgrade` |
| `upgrade_attested_event` | `upgrade_attested` | `upgrade` |
| `attestation_cleared_event` | `attestation_cleared` | `upgrade` |
| `batch_remove_proposed_event` | `batch_remove_proposed` | `batch` |
| `batch_remove_executed_event` | `batch_remove_executed` | `batch` |
| `batch_remove_cancelled_event` | `batch_remove_cancelled` | `batch` |

An event whose `topic[0]` does not match any row is written with
`event_kind: "unknown"` and `category: "unclassified"`. This is intentional: an
indexer pointed at a newer contract version must keep recording events it does not
recognise — losing the stream entirely at the moment an unknown event appears would
be worse than a gap in classification.

### Adding a new event kind

1. Add the `#[contractevent]` struct to `src/events.rs`.
2. Add a row to the `TOPIC_TABLE` in `scripts/event_indexer.sh`:
   `<topic_symbol>|<event_kind>|<category>`
3. `scripts/check_event_topics.sh` will fail CI until both sides agree.

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
