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
#   --batch-size — number of records to extend per transaction (default: 100, max: 100)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

NETWORK="${NETWORK:-testnet}"
SOURCE="${SOURCE:-}"
CONTRACT_ID="${CONTRACT_ID:-}"
BATCH_SIZE=100
DRY_RUN=false
STELLAR="${STELLAR:-stellar}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)    DRY_RUN=true; shift ;;
        --batch-size) BATCH_SIZE="$2"; shift 2 ;;
        --contract)   CONTRACT_ID="$2"; shift 2 ;;
        --source)     SOURCE="$2"; shift 2 ;;
        --network)    NETWORK="$2"; shift 2 ;;
        -h|--help)
            grep '^#' "$0" | sed 's/^# \?//' | grep -v '^!'
            exit 1
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

if [[ "$BATCH_SIZE" -gt 100 || "$BATCH_SIZE" -lt 1 ]]; then
    echo "ERROR: batch size must be between 1 and 100." >&2
    exit 1
fi

echo "==> Starting TTL Keeper"
echo "    Contract:   $CONTRACT_ID"
echo "    Network:    $NETWORK"
echo "    Source:     $SOURCE"
echo "    Batch size: $BATCH_SIZE"
echo "    Dry-run:    $DRY_RUN"
echo ""

cursor=0
max_iterations=100000
iteration=0
total_extended=0
total_processed=0

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
                echo "ERROR: batch failed" >&2
                echo "$output" >&2
                # Do not stop entirely, but record error.
            else
                # On success, it returns the number of records actually extended
                # e.g., output might be `10` or `100`.
                echo "  -> OK: Extended TTL for records."
                total_extended=$((total_extended + count))
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
echo "==> Done!"
echo "    Processed: $total_processed"
echo "    Extended:  $total_extended (attempted)"
