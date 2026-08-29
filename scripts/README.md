# `scripts/`

Operational helper scripts for the TrustBridge registry contract. All are
POSIX-ish `bash`, take configuration from environment variables, keep secrets
out of the repo, and default to **testnet** (never mainnet).

| Script | Purpose |
|---|---|
| `deploy.sh` | Build + deploy + `initialize` a fresh instance. |
| `demo_e2e.sh` | One-shot happy path: register → verify → lookup → export. |
| `event_indexer.sh` | **Reference event indexer** — tails contract events into local JSONL with an on-disk resume cursor (Issue #288). See below. |
| `export_registry.sh` | Page the admin export into a single registry JSON snapshot. |
| `validate_registry.sh` | Diff an export JSON against live on-chain state. |
| `ttl_keeper.sh` | Walk the index and bump persistent-entry TTLs. |
| `bulk_verify.sh` / `bulk_revoke.sh` | Batched verify / revoke from a username list, with RPC pacing. |
| `simulate_pause.sh` | Exercise the pause / unpause lifecycle. |
| `futurenet_smoke_test.sh` | End-to-end smoke test against futurenet. |
| `storage_rent_estimator.py` | Estimate on-chain storage entry counts from `docs/storage-rent-estimator.inputs.v1.json`; warns on `CHUNK_SIZE` drift vs `src/storage.rs` (Issue #290). |

---

## `event_indexer.sh` — reference event-indexing service

`docs/DASHBOARD_SYNC.md` and `docs/EVENT_INDEXING.md` describe how a dashboard
or indexer should consume TrustBridge events, but until Issue #288 there was no
runnable indexer in the repo — `demo_e2e.sh` is a one-shot and does not tail
anything. This script is the missing reference implementation.

### What it does

1. Calls the Stellar RPC [`getEvents`](https://developers.stellar.org/docs/data/rpc/api-reference/methods/getEvents)
   method on a loop, filtered to one `contractId`.
2. Appends every event as one JSON object per line to
   `.indexer/events-<network>.jsonl`.
3. Writes the RPC pagination cursor to `.indexer/cursor-<network>.json` after
   **every** batch (atomically, via a temp file + `mv`). A restart resumes from
   that cursor — **idempotent restart**.
4. De-duplicates on the RPC event `id` using `.indexer/seen-<network>.txt`, so a
   ledger reorg or an overlapping re-read never appends the same event twice.
5. Handles the empty stream (advances the cursor past empty windows), RPC
   errors (linear backoff, bounded retries), and a pruned/too-old cursor
   (falls back to a cold re-scan).

It is a **reference** indexer for local dev, futurenet, and testnet. It is not
a production hosted indexer: no cloud account, no database, no secrets — all
state is three files under `DATA_DIR`.

### Run it

```bash
# Follow testnet from ~1 day back, forever:
CONTRACT_ID=C... ./scripts/event_indexer.sh

# Drain to head once and exit (cron / CI):
CONTRACT_ID=C... ONESHOT=1 ./scripts/event_indexer.sh

# Local / futurenet RPC, explicit start ledger:
CONTRACT_ID=C... RPC_URL=http://localhost:8000/soroban/rpc \
  START_LEDGER=1 ONESHOT=1 ./scripts/event_indexer.sh

# Offline: replay a canned RPC response, no network:
MOCK_RESPONSE=./scripts/testdata/getEvents.sample.json \
  CONTRACT_ID=C_MOCK ONESHOT=1 ./scripts/event_indexer.sh
```

### Verify resume behavior locally

```bash
rm -rf .indexer
MOCK_RESPONSE=./scripts/testdata/getEvents.sample.json CONTRACT_ID=C_MOCK ONESHOT=1 ./scripts/event_indexer.sh
wc -l .indexer/events-testnet.jsonl          # => 2
# Run it again — the cursor on disk is replayed, nothing is double-written:
MOCK_RESPONSE=./scripts/testdata/getEvents.sample.json CONTRACT_ID=C_MOCK ONESHOT=1 ./scripts/event_indexer.sh
wc -l .indexer/events-testnet.jsonl          # => still 2
```

### Environment variables

See the header comment of `event_indexer.sh` for the full list. The common
ones: `CONTRACT_ID` (required), `RPC_URL`, `NETWORK`, `DATA_DIR`,
`START_LEDGER`, `LOOKBACK`, `POLL_SECONDS`, `PAGE_LIMIT`, `ONESHOT`,
`MOCK_RESPONSE`.

### Output schema

Each line of `events-<network>.jsonl`:

```json
{
  "id": "0001000042-0000000001",
  "ledger_sequence": 1000042,
  "ledger_closed_at": "2026-01-15T12:00:05Z",
  "contract_id": "C...",
  "tx_hash": "a1b2c3...",
  "type": "contract",
  "topic": ["<base64 xdr>", "..."],
  "value": "<base64 xdr>",
  "in_successful_contract_call": true,
  "indexed_at": "2026-08-29T00:00:00Z"
}
```

`(ledger_sequence, tx_hash)` plus the decoded topic symbol is the idempotency
key described in `docs/DASHBOARD_SYNC.md`. Topic/value XDR decoding is left to
the consumer (`stellar xdr decode`, or the SDK of your dashboard's language) —
this script is deliberately decode-agnostic so it stays dependency-light.

---

## `storage_rent_estimator.py` — on-chain storage rent estimator

Turns `docs/storage-rent-estimator.inputs.v1.json` (spec: `docs/STORAGE_RENT_ESTIMATOR.md`)
into concrete entry counts so you do not have to reverse-engineer the docs.

```bash
# Persistent + instance entry counts for 250 contributors, 3 role holders:
python3 scripts/storage_rent_estimator.py --users 250 --roles 3 --lastact 100

python3 scripts/storage_rent_estimator.py --users 1000 --json
```

The estimator is a pure function over the versioned JSON: it counts entries
only (XLM conversion needs operator-supplied `network_rent_params`). It also
compares `chunk_size` in the inputs JSON against `CHUNK_SIZE` in
`src/storage.rs` and prints a warning to stderr on any drift.

Golden-output test: `bash scripts/test_storage_rent_estimator.sh`.
