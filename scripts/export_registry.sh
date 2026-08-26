#!/usr/bin/env bash
# Export the full contributor registry to a JSON file for backups, dashboard
# migrations, and audit snapshots (Issue #132).
#
# Usage:
#   CONTRACT_ID=C... SOURCE=admin NETWORK=testnet ./scripts/export_registry.sh
#   CONTRACT_ID=C... SOURCE=admin NETWORK=testnet OUTPUT_FILE=out.json ./scripts/export_registry.sh
#
# Environment variables:
#   CONTRACT_ID  — deployed contract ID (required)
#   SOURCE       — Stellar CLI identity of the contract admin (required).
#                  get_registered_paginated is admin-gated, so SOURCE must
#                  sign as the address that called `initialize`.
#   NETWORK      — testnet | mainnet | futurenet (default: testnet)
#   OUTPUT_FILE  — path to write the export JSON (default: registry-export-<network>.json)
#   PAGE_LIMIT   — records requested per page (default: 100, the contract's MAX_PAGE_LIMIT)
#
# Output schema (see docs/DEPLOYMENT.md#registry-export--import):
#   {
#     "schema_version": 1,
#     "contract_id": "C...",
#     "network": "testnet",
#     "exported_at": "2026-01-01T00:00:00Z",
#     "count": 2,
#     "records": [
#       { "github_username": "octocat", "stellar_address": "G...",
#         "verified": true, "registered_at": 1732800000 }
#     ]
#   }

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

NETWORK="${NETWORK:-testnet}"
SOURCE="${SOURCE:-}"
CONTRACT_ID="${CONTRACT_ID:-}"
OUTPUT_FILE="${OUTPUT_FILE:-registry-export-${NETWORK}.json}"
PAGE_LIMIT="${PAGE_LIMIT:-100}"
STELLAR="${STELLAR:-stellar}"

if [[ -z "$CONTRACT_ID" ]]; then
  echo "ERROR: CONTRACT_ID must be set to the deployed contract ID (C...)." >&2
  echo "Example: CONTRACT_ID=C... SOURCE=admin NETWORK=testnet ./scripts/export_registry.sh" >&2
  exit 1
fi

if [[ -z "$SOURCE" ]]; then
  echo "ERROR: SOURCE must be set to the Stellar CLI identity of the contract admin." >&2
  echo "get_registered_paginated is admin-gated; SOURCE must sign as the registered admin address." >&2
  exit 1
fi

if ! command -v "$STELLAR" >/dev/null 2>&1; then
  echo "ERROR: Stellar CLI ('$STELLAR') not found on PATH." >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "ERROR: jq is required to assemble the export JSON." >&2
  exit 1
fi

echo "==> Exporting registry from ${CONTRACT_ID} (${NETWORK})" >&2

tmp_records="$(mktemp)"
trap 'rm -f "$tmp_records"' EXIT

cursor=0
# Bounds the loop against a stalled cursor (e.g. an RPC hiccup echoing the same
# page) instead of spinning forever on untrusted network responses.
max_iterations=100000
iteration=0

while :; do
  iteration=$((iteration + 1))
  if [[ "$iteration" -gt "$max_iterations" ]]; then
    echo "ERROR: exceeded ${max_iterations} pages without exhausting the index; aborting." >&2
    exit 1
  fi

  page="$("$STELLAR" contract invoke \
    --id "$CONTRACT_ID" \
    --source-account "$SOURCE" \
    --network "$NETWORK" \
    -- get_registered_paginated --cursor "$cursor" --limit "$PAGE_LIMIT")"

  # get_registered_paginated returns:
  #   { "records": [[username, {stellar_address, registered_at, verified}], ...],
  #     "next_cursor": <u32> | null, "total": <u32>, "has_more": bool }
  jq -c '.records[] | {
      github_username: .[0],
      stellar_address: .[1].stellar_address,
      verified: .[1].verified,
      registered_at: .[1].registered_at
    }' <<<"$page" >> "$tmp_records"

  has_more="$(jq -r '.has_more' <<<"$page")"
  next_cursor="$(jq -r '.next_cursor' <<<"$page")"

  if [[ "$has_more" != "true" || "$next_cursor" == "null" ]]; then
    break
  fi
  cursor="$next_cursor"
done

jq -s \
  --arg contract_id "$CONTRACT_ID" \
  --arg network "$NETWORK" \
  --arg exported_at "$(date -u +"%Y-%m-%dT%H:%M:%SZ")" \
  '{
    schema_version: 1,
    contract_id: $contract_id,
    network: $network,
    exported_at: $exported_at,
    count: length,
    records: .
  }' "$tmp_records" > "$OUTPUT_FILE"

echo "==> Wrote $(jq '.count' "$OUTPUT_FILE") record(s) to ${OUTPUT_FILE}" >&2
