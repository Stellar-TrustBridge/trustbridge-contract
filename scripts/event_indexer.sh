#!/usr/bin/env bash
# Reference event-indexing service for local dashboard sync (Issue #288).
#
# DASHBOARD_SYNC.md and EVENT_INDEXING.md describe an indexer that tails
# TrustBridge contract events into a local store, but no such process lived in
# this repo — contributors had to guess the shape. This is that reference
# implementation: a small, dependency-light poller that
#
#   1. calls the Stellar RPC `getEvents` method on a loop,
#   2. appends every event as one JSON object per line (JSONL) to a data file,
#   3. persists the RPC pagination cursor to disk after each batch, so a
#      restart resumes exactly where it left off (idempotent restart), and
#   4. de-duplicates on the RPC event `id`, so a reorg or an overlapping
#      re-read never writes the same event twice.
#
# It is a REFERENCE indexer for local dev / futurenet / testnet — not a
# production hosted service. No cloud account, database, or secret is required;
# all state is two files on disk.
#
# ---------------------------------------------------------------------------
# Usage
# ---------------------------------------------------------------------------
#   CONTRACT_ID=C... ./scripts/event_indexer.sh
#
#   # against a local / futurenet RPC, from a known ledger, one pass only:
#   CONTRACT_ID=C... RPC_URL=http://localhost:8000/soroban/rpc \
#     START_LEDGER=1000 ONESHOT=1 ./scripts/event_indexer.sh
#
#   # offline demo against a canned RPC response (no network):
#   MOCK_RESPONSE=./scripts/testdata/getEvents.sample.json \
#     CONTRACT_ID=C_MOCK ONESHOT=1 ./scripts/event_indexer.sh
#
# ---------------------------------------------------------------------------
# Environment variables
# ---------------------------------------------------------------------------
#   CONTRACT_ID    Deployed contract ID to filter events for (required unless
#                  MOCK_RESPONSE is set).
#   RPC_URL        Stellar RPC endpoint. Default: https://soroban-testnet.stellar.org
#                  Futurenet: https://rpc-futurenet.stellar.org
#   NETWORK        Informational tag written into the state file (default: testnet).
#   DATA_DIR       Directory for indexer state + output (default: ./.indexer).
#   START_LEDGER   Ledger to start from on a COLD start (no cursor on disk).
#                  Default: latest ledger reported by the RPC minus LOOKBACK.
#   LOOKBACK       Ledgers to rewind from "latest" on a cold start (default: 17280,
#                  ~1 day). Ignored once a cursor exists.
#   POLL_SECONDS   Sleep between polls in the follow loop (default: 5).
#   PAGE_LIMIT     Events requested per RPC page (default: 100, RPC max is 10000).
#   ONESHOT        If set to 1, do a single drain to head and exit 0 (for CI /
#                  cron). Otherwise follow forever until SIGINT/SIGTERM.
#   MOCK_RESPONSE  Path to a file containing a canned getEvents JSON-RPC
#                  response. When set, no network call is made and the loop
#                  runs exactly once. Used by the docs' offline example and by
#                  scripts/testdata/.
#   MAX_RETRIES    Consecutive RPC failures tolerated before giving up (default: 5).
#
# ---------------------------------------------------------------------------
# Files written under DATA_DIR
# ---------------------------------------------------------------------------
#   events-<network>.jsonl   append-only event log, one JSON object per line
#   cursor-<network>.json    { "cursor": <str|null>, "last_ledger": <int>,
#                              "last_id": <str|null>, "updated_at": <iso8601>,
#                              "event_count": <int> }
#   seen-<network>.txt       recent event ids (bounded) for dedup across restarts
#
# All three are safe to delete: removing cursor-*.json forces a cold re-scan
# from START_LEDGER/LOOKBACK; removing seen-*.txt only weakens dedup for the
# overlap window; removing events-*.jsonl loses the local log.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CONTRACT_ID="${CONTRACT_ID:-}"
RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"
NETWORK="${NETWORK:-testnet}"
DATA_DIR="${DATA_DIR:-./.indexer}"
LOOKBACK="${LOOKBACK:-17280}"
POLL_SECONDS="${POLL_SECONDS:-5}"
PAGE_LIMIT="${PAGE_LIMIT:-100}"
ONESHOT="${ONESHOT:-}"
MOCK_RESPONSE="${MOCK_RESPONSE:-}"
MAX_RETRIES="${MAX_RETRIES:-5}"
SEEN_MAX_LINES="${SEEN_MAX_LINES:-50000}"

if ! command -v jq >/dev/null 2>&1; then
  echo "ERROR: jq is required." >&2
  exit 1
fi
if [[ -z "$MOCK_RESPONSE" ]] && ! command -v curl >/dev/null 2>&1; then
  echo "ERROR: curl is required (or set MOCK_RESPONSE for an offline run)." >&2
  exit 1
fi
if [[ -z "$CONTRACT_ID" && -z "$MOCK_RESPONSE" ]]; then
  echo "ERROR: set CONTRACT_ID=<C...> (or MOCK_RESPONSE=<file> for an offline run)." >&2
  exit 1
fi

mkdir -p "$DATA_DIR"
EVENTS_FILE="${DATA_DIR}/events-${NETWORK}.jsonl"
CURSOR_FILE="${DATA_DIR}/cursor-${NETWORK}.json"
SEEN_FILE="${DATA_DIR}/seen-${NETWORK}.txt"
touch "$EVENTS_FILE" "$SEEN_FILE"

now_iso() { date -u +"%Y-%m-%dT%H:%M:%SZ"; }

log() { echo "[$(now_iso)] $*" >&2; }

# --- cursor persistence -----------------------------------------------------

read_cursor()      { [[ -f "$CURSOR_FILE" ]] && jq -r '.cursor // empty' "$CURSOR_FILE" || true; }
read_last_ledger() { [[ -f "$CURSOR_FILE" ]] && jq -r '.last_ledger // 0' "$CURSOR_FILE" || echo 0; }
read_event_count() { [[ -f "$CURSOR_FILE" ]] && jq -r '.event_count // 0' "$CURSOR_FILE" || echo 0; }

write_cursor() {
  # $1 cursor (may be empty), $2 last_ledger, $3 last_id (may be empty), $4 event_count
  local tmp
  tmp="$(mktemp "${CURSOR_FILE}.XXXX")"
  jq -n \
    --arg cursor "${1:-}" \
    --argjson last_ledger "${2:-0}" \
    --arg last_id "${3:-}" \
    --arg updated_at "$(now_iso)" \
    --argjson event_count "${4:-0}" \
    --arg network "$NETWORK" \
    '{
      network: $network,
      cursor: (if $cursor == "" then null else $cursor end),
      last_ledger: $last_ledger,
      last_id: (if $last_id == "" then null else $last_id end),
      updated_at: $updated_at,
      event_count: $event_count
    }' > "$tmp"
  mv -f "$tmp" "$CURSOR_FILE"   # atomic replace — a crash mid-write cannot corrupt the cursor
}

have_seen() { grep -qxF "$1" "$SEEN_FILE"; }

mark_seen() {
  echo "$1" >> "$SEEN_FILE"
  # Keep the dedup file bounded: trim to the most recent SEEN_MAX_LINES ids.
  local lines
  lines="$(wc -l < "$SEEN_FILE")"
  if (( lines > SEEN_MAX_LINES )); then
    tail -n "$SEEN_MAX_LINES" "$SEEN_FILE" > "${SEEN_FILE}.trim" && mv -f "${SEEN_FILE}.trim" "$SEEN_FILE"
  fi
}

# --- RPC -------------------------------------------------------------------

rpc_latest_ledger() {
  curl -sS --fail --max-time 20 -X POST "$RPC_URL" \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' \
    | jq -r '.result.sequence'
}

# Emits the raw JSON-RPC response for one getEvents page on stdout.
# Args: $1 = startLedger (used only when $2 is empty), $2 = cursor
rpc_get_events() {
  local start_ledger="$1" cursor="$2" pagination
  if [[ -n "$cursor" ]]; then
    pagination="$(jq -n --arg c "$cursor" --argjson l "$PAGE_LIMIT" '{cursor:$c, limit:$l}')"
  else
    pagination="$(jq -n --argjson l "$PAGE_LIMIT" '{limit:$l}')"
  fi

  local req
  req="$(jq -n \
    --argjson start "${start_ledger:-0}" \
    --arg cid "$CONTRACT_ID" \
    --argjson pagination "$pagination" \
    --arg use_start "$([[ -z "$cursor" ]] && echo yes || echo no)" \
    '{
      jsonrpc: "2.0", id: 1, method: "getEvents",
      params: (
        {
          filters: [ { type: "contract", contractIds: [ $cid ], topics: [] } ],
          pagination: $pagination
        }
        + (if $use_start == "yes" then { startLedger: $start } else {} end)
      )
    }')"

  curl -sS --fail --max-time 30 -X POST "$RPC_URL" \
    -H 'Content-Type: application/json' -d "$req"
}

# --- event processing ----------------------------------------------------

# Reads a JSON-RPC response on stdin, appends new events to EVENTS_FILE,
# echoes "<next_cursor>\t<max_ledger>\t<appended_count>\t<last_id>" on stdout.
process_response() {
  local resp="$1" err
  err="$(jq -r '.error.message // empty' <<<"$resp")"
  if [[ -n "$err" ]]; then
    log "RPC error: $err"
    return 1
  fi

  local events next_cursor latest_ledger
  events="$(jq -c '.result.events // []' <<<"$resp")"
  next_cursor="$(jq -r '.result.cursor // empty' <<<"$resp")"
  latest_ledger="$(jq -r '.result.latestLedger // 0' <<<"$resp")"

  local appended=0 max_ledger last_id=""
  max_ledger="$(read_last_ledger)"

  local n i
  n="$(jq 'length' <<<"$events")"
  for (( i = 0; i < n; i++ )); do
    local ev id ledger
    ev="$(jq -c ".[$i]" <<<"$events")"
    id="$(jq -r '.id' <<<"$ev")"
    ledger="$(jq -r '.ledger // 0' <<<"$ev")"

    if have_seen "$id"; then
      continue   # reorg / overlap re-read → already logged, skip
    fi

    # Normalize into the shape DASHBOARD_SYNC.md keys idempotency on:
    # (ledger_sequence, tx_hash) plus the topic symbol.
    jq -c \
      --arg indexed_at "$(now_iso)" \
      '{
        id: .id,
        ledger_sequence: (.ledger // .inSuccessfulContractCall // 0),
        ledger_closed_at: .ledgerClosedAt,
        contract_id: .contractId,
        tx_hash: (.txHash // .transactionHash // null),
        type: .type,
        topic: (.topic // .topics),
        value: .value,
        in_successful_contract_call: (.inSuccessfulContractCall // true),
        indexed_at: $indexed_at
      }' <<<"$ev" >> "$EVENTS_FILE"

    mark_seen "$id"
    appended=$((appended + 1))
    last_id="$id"
    (( ledger > max_ledger )) && max_ledger="$ledger"
  done

  printf '%s\t%s\t%s\t%s\n' "$next_cursor" "$max_ledger" "$appended" "$last_id"
}

# --- main loop -----------------------------------------------------------

RUNNING=1
trap 'RUNNING=0; log "signal received — finishing current batch then exiting"' INT TERM

cold_start_ledger() {
  if [[ -n "${START_LEDGER:-}" ]]; then
    echo "$START_LEDGER"; return
  fi
  local latest
  if latest="$(rpc_latest_ledger 2>/dev/null)" && [[ "$latest" =~ ^[0-9]+$ ]]; then
    local start=$((latest - LOOKBACK))
    (( start < 1 )) && start=1
    echo "$start"
  else
    echo 1
  fi
}

main() {
  local cursor start_ledger total retries=0
  cursor="$(read_cursor)"
  total="$(read_event_count)"

  if [[ -n "$cursor" ]]; then
    log "resuming from stored cursor (last_ledger=$(read_last_ledger), event_count=$total)"
    start_ledger=0
  elif [[ -n "$MOCK_RESPONSE" ]]; then
    start_ledger=0
  else
    start_ledger="$(cold_start_ledger)"
    log "cold start from ledger $start_ledger (RPC_URL=$RPC_URL, contract=$CONTRACT_ID)"
  fi

  while (( RUNNING )); do
    local resp
    if [[ -n "$MOCK_RESPONSE" ]]; then
      resp="$(cat "$MOCK_RESPONSE")"
    elif ! resp="$(rpc_get_events "$start_ledger" "$cursor")"; then
      retries=$((retries + 1))
      log "getEvents call failed (attempt $retries/$MAX_RETRIES)"
      if (( retries >= MAX_RETRIES )); then
        log "giving up after $MAX_RETRIES consecutive failures"
        exit 1
      fi
      sleep $(( POLL_SECONDS * retries ))   # linear backoff
      continue
    fi
    retries=0

    local line next_cursor max_ledger appended last_id
    if ! line="$(process_response "$resp")"; then
      # RPC returned a JSON-RPC error object (e.g. cursor too old / pruned).
      # On a pruned cursor, fall back to a cold start rather than spin.
      if jq -e '.error.message | test("cursor"; "i") // false' <<<"$resp" >/dev/null 2>&1; then
        log "stored cursor rejected by RPC — falling back to cold start"
        cursor=""
        start_ledger="$(cold_start_ledger)"
        write_cursor "" "$start_ledger" "" "$total"
        continue
      fi
      retries=$((retries + 1))
      (( retries >= MAX_RETRIES )) && { log "repeated RPC errors — exiting"; exit 1; }
      sleep "$POLL_SECONDS"
      continue
    fi

    IFS=$'\t' read -r next_cursor max_ledger appended last_id <<<"$line"
    total=$((total + appended))

    # Persist progress even when appended == 0: the cursor still advances past
    # an empty window, and re-reading it on restart must not replay it.
    if [[ -n "$next_cursor" ]]; then
      cursor="$next_cursor"
      start_ledger=0
    fi
    write_cursor "$cursor" "${max_ledger:-0}" "$last_id" "$total"

    if (( appended > 0 )); then
      log "appended $appended event(s) → $EVENTS_FILE (total=$total, through ledger $max_ledger)"
    fi

    if [[ -n "$MOCK_RESPONSE" ]]; then
      log "mock run complete: $appended new event(s), total=$total"
      break
    fi

    # A short page (fewer than PAGE_LIMIT events) means we have reached head.
    local page_len
    page_len="$(jq -r '.result.events | length' <<<"$resp")"
    if (( page_len < PAGE_LIMIT )); then
      if [[ -n "$ONESHOT" ]]; then
        log "reached head (oneshot) — total=$total, exiting 0"
        break
      fi
      sleep "$POLL_SECONDS"
    fi
  done
}

main
