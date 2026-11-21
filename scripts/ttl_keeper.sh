#!/usr/bin/env bash
# ttl_keeper.sh — Periodic TTL extender for the registry
#
# Walks the entire registry and extends the TTL of all records in batches.
# Permissionless: can be run by anyone, does not require admin auth.
#
# Usage:
#   CONTRACT_ID=C... SOURCE=keeper-identity NETWORK=testnet ./scripts/ttl_keeper.sh [--dry-run] [--batch-size 100]
#
# Environment variables:
#   CONTRACT_ID  — deployed contract ID (required)
#   SOURCE       — Stellar CLI identity to pay the transaction fee (required)
#   NETWORK      — testnet | mainnet | futurenet (default: testnet)
#
# Flags:
#   --dry-run    — walk the index and print batches, but do not send transactions
#   --batch-size — records to extend per transaction (default: the contract's cap)
#
# Related:
#   docs/STORAGE_RENT.md#keeper-implementation — why this exists and when to run it
#   Makefile target `ttl-keeper`               — the supported way to invoke it
#   tests/extend_registry_ttl.rs               — contract-side behaviour and limits

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

NETWORK="${NETWORK:-testnet}"
SOURCE="${SOURCE:-}"
CONTRACT_ID="${CONTRACT_ID:-}"
DRY_RUN=false
STELLAR="${STELLAR:-stellar}"

# ── Batch cap, read from the contract source ────────────────────────────────
#
# `extend_registry_ttl` validates its argument against
# `BatchConfig::default().max_batch_size`. Hard-coding the number here means
# the script keeps sending oversized batches after the contract tightens its
# cap, and every one comes back `InvalidBatchSize` — a keeper that appears to
# run and extends nothing, which is the worst way for this to fail because the
# records expire silently.
#
# Reading it out of the source instead makes drift a startup error.
read_rust_u32() {
    local file="$1" pattern="$2"
    sed -nE "s/.*${pattern}.*/\1/p" "$file" | head -n1 | tr -d '_'
}

CONTRACT_MAX_BATCH="$(read_rust_u32 src/batch.rs 'max_batch_size:[[:space:]]*([0-9_]+)')"
MAX_WRITE_BATCH="$(read_rust_u32 src/batch.rs 'pub const MAX_WRITE_BATCH:[[:space:]]*u32[[:space:]]*=[[:space:]]*([0-9_]+)')"

if [[ -z "$CONTRACT_MAX_BATCH" ]]; then
    echo "ERROR: could not read max_batch_size from src/batch.rs." >&2
    echo "       The keeper derives its batch cap from the contract; refusing" >&2
    echo "       to guess. Run this from a checkout of the contract repo." >&2
    exit 1
fi

BATCH_SIZE="$CONTRACT_MAX_BATCH"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)    DRY_RUN=true; shift ;;
        --batch-size) BATCH_SIZE="$2"; shift 2 ;;
        --contract)   CONTRACT_ID="$2"; shift 2 ;;
        --source)     SOURCE="$2"; shift 2 ;;
        --network)    NETWORK="$2"; shift 2 ;;
        -h|--help)
            grep '^#' "$0" | sed 's/^# \?//' | grep -v '^!'
            # Asking for help is not an error.
            exit 0
            ;;
        *) echo "Unknown flag: $1" >&2; exit 1 ;;
    esac
done

if [[ -z "$CONTRACT_ID" ]]; then
    echo "ERROR: CONTRACT_ID (or --contract) must be set." >&2
    exit 1
fi

if [[ -z "$SOURCE" ]]; then
    echo "ERROR: SOURCE (or --source) must be set to pay the transaction fee." >&2
    exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
    echo "ERROR: jq is required." >&2
    exit 1
fi

if ! [[ "$BATCH_SIZE" =~ ^[0-9]+$ ]]; then
    echo "ERROR: --batch-size must be a positive integer, got '$BATCH_SIZE'." >&2
    exit 1
fi

if [[ "$BATCH_SIZE" -gt "$CONTRACT_MAX_BATCH" || "$BATCH_SIZE" -lt 1 ]]; then
    echo "ERROR: batch size must be between 1 and ${CONTRACT_MAX_BATCH}" >&2
    echo "       (BatchConfig::default().max_batch_size in src/batch.rs)." >&2
    exit 1
fi

# `extend_registry_ttl` writes state, so `MAX_WRITE_BATCH` is the cap it would
# use if it were built on `BatchConfig::for_writes()`. It currently uses the
# looser default. Warn rather than fail: the contract is the authority on what
# it accepts, and a keeper that refuses to run because of a source-level
# inconsistency helps nobody — but an operator sizing batches should know.
if [[ -n "$MAX_WRITE_BATCH" && "$BATCH_SIZE" -gt "$MAX_WRITE_BATCH" ]]; then
    echo "NOTE: batch size ${BATCH_SIZE} exceeds MAX_WRITE_BATCH (${MAX_WRITE_BATCH})," >&2
    echo "      the cap derived for state-writing batch entry points." >&2
    echo "      extend_registry_ttl accepts it today, but large batches are" >&2
    echo "      likelier to exhaust the per-transaction resource budget." >&2
fi

echo "==> Starting TTL Keeper"
echo "    Contract:   $CONTRACT_ID"
echo "    Network:    $NETWORK"
echo "    Source:     $SOURCE"
echo "    Batch size: $BATCH_SIZE (contract cap: $CONTRACT_MAX_BATCH)"
echo "    Dry-run:    $DRY_RUN"
echo ""

cursor=0
max_iterations=100000
iteration=0
total_extended=0
total_processed=0
failed_batches=0

while :; do
    iteration=$((iteration + 1))
    if [[ "$iteration" -gt "$max_iterations" ]]; then
        echo "ERROR: exceeded ${max_iterations} pages; aborting." >&2
        exit 1
    fi

    # Using get_public_paginated because it's permissionless
    page="$("$STELLAR" contract invoke \
        --id "$CONTRACT_ID" \
        --source-account "$SOURCE" \
        --network "$NETWORK" \
        -- get_public_paginated --cursor "$cursor" --limit "$BATCH_SIZE")"
    
    # Extract usernames from the page
    usernames_json="$(jq -c '[.records[][0]]' <<<"$page")"
    count="$(jq 'length' <<<"$usernames_json")"
    
    if [[ "$count" -gt 0 ]]; then
        echo "Processing batch of $count records (cursor: $cursor)..."
        
        if [[ "$DRY_RUN" == true ]]; then
            echo "[DRY-RUN] Would extend TTL for: $usernames_json"
            total_extended=$((total_extended + count))
        else
            set +e
            output="$("$STELLAR" contract invoke \
                --id "$CONTRACT_ID" \
                --source-account "$SOURCE" \
                --network "$NETWORK" \
                --send=yes \
                -- extend_registry_ttl \
                --usernames "$usernames_json" 2>&1)"
            rc=$?
            set -e
            
            if [[ $rc -ne 0 ]]; then
                echo "ERROR: batch at cursor $cursor failed" >&2
                echo "$output" >&2
                # Keep walking — one bad page should not strand the records
                # after it — but remember, so the exit status is honest.
                failed_batches=$((failed_batches + 1))
            else
                # The contract returns how many records it actually extended,
                # which is not the same as how many were submitted: a record
                # removed between the page read and the write is skipped.
                # Counting the submitted total would over-report every run.
                extended="$(tr -dc '0-9' <<<"$output" | head -c 10)"
                if [[ -n "$extended" ]]; then
                    echo "  -> OK: extended $extended of $count records."
                    total_extended=$((total_extended + extended))
                else
                    echo "  -> OK, but could not parse the extended count from the response." >&2
                    echo "$output" >&2
                fi
            fi
        fi
        
        total_processed=$((total_processed + count))
    else
        # empty registry or end reached but has_more wasn't parsed properly
        echo "No records in page."
    fi

    has_more="$(jq -r '.has_more' <<<"$page")"
    next_cursor="$(jq -r '.next_cursor' <<<"$page")"

    if [[ "$has_more" != "true" || "$next_cursor" == "null" ]]; then
        break
    fi
    cursor="$next_cursor"
    
    # Optional sleep to avoid spamming RPC, as per guidelines "Do not spam RPC"
    sleep 1
done

echo ""
echo "==> Done"
echo "    Processed:      $total_processed"
if [[ "$DRY_RUN" == true ]]; then
    echo "    Would extend:   $total_extended"
else
    echo "    Extended:       $total_extended"
fi
echo "    Failed batches: $failed_batches"

# A keeper is normally run unattended on a timer. Exiting 0 after every batch
# failed would report a successful run while the records quietly expire, so the
# exit status has to carry the failures.
if [[ "$failed_batches" -gt 0 ]]; then
    echo "" >&2
    echo "ERROR: $failed_batches batch(es) failed; TTL was not extended for all records." >&2
    exit 1
fi
