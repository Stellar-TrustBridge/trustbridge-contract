#!/usr/bin/env bash
# Validate a registry export JSON file (see scripts/export_registry.sh) against
# the live on-chain registry (Issue #133).
#
# This is a validate-only tool: it never writes to the contract. It is meant
# for staging restores, migration dry-runs, and catching a stale export before
# anyone acts on it. Do not use its output to blindly replay writes to
# mainnet — review every reported mismatch first.
#
# Usage:
#   CONTRACT_ID=C... NETWORK=testnet ./scripts/validate_registry.sh registry-export-testnet.json
#
#   # Also check for on-chain registrations missing from the export file
#   # (requires the admin identity, since that read is admin-gated):
#   CONTRACT_ID=C... SOURCE=admin NETWORK=testnet ./scripts/validate_registry.sh export.json
#
# Environment variables:
#   CONTRACT_ID  — deployed contract ID (required)
#   NETWORK      — testnet | mainnet | futurenet (default: testnet)
#   SOURCE       — Stellar CLI identity used for the per-record checks (default: default).
#                  Those checks call get_address, which requires no auth, so
#                  any funded identity works here.
#   ADMIN_SOURCE — Stellar CLI identity of the contract admin (unset by default).
#                  When set, additionally detects on-chain records missing from
#                  the export via the admin-gated get_registered_paginated.
#                  Kept separate from SOURCE so a plain `make validate-registry`
#                  never attempts an admin-gated call with a non-admin identity.
#   PAGE_LIMIT   — records requested per page for the admin-side check (default: 100)
#   REPORT_FILE  — optional path for a stable JSON dry-run repair report whose
#                  field names/order are pinned in docs/ABI.md and
#                  docs/CONTRACT_HEALTH.md. The report is read-only: it never
#                  proposes or applies writes.
#
# Exit status: 0 if the export matches live state, 1 if any mismatch was
# found, 2 on usage/configuration errors.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

NETWORK="${NETWORK:-testnet}"
CONTRACT_ID="${CONTRACT_ID:-}"
SOURCE="${SOURCE:-default}"
ADMIN_SOURCE="${ADMIN_SOURCE:-}"
STELLAR="${STELLAR:-stellar}"
PAGE_LIMIT="${PAGE_LIMIT:-100}"
REPORT_FILE="${REPORT_FILE:-}"

EXPORT_FILE="${1:-}"

if [[ -z "$EXPORT_FILE" ]]; then
  echo "Usage: CONTRACT_ID=C... NETWORK=testnet $0 <export.json>" >&2
  exit 2
fi

if [[ -z "$CONTRACT_ID" ]]; then
  echo "ERROR: CONTRACT_ID must be set to the deployed contract ID (C...)." >&2
  exit 2
fi

if [[ ! -f "$EXPORT_FILE" ]]; then
  echo "ERROR: export file not found: $EXPORT_FILE" >&2
  exit 2
fi

if ! command -v "$STELLAR" >/dev/null 2>&1; then
  echo "ERROR: Stellar CLI ('$STELLAR') not found on PATH." >&2
  exit 2
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "ERROR: jq is required to read the export JSON." >&2
  exit 2
fi

if ! jq empty "$EXPORT_FILE" >/dev/null 2>&1; then
  echo "ERROR: $EXPORT_FILE is not valid JSON." >&2
  exit 2
fi

if ! jq -e 'has("records") and (.records | type == "array")' "$EXPORT_FILE" >/dev/null 2>&1; then
  echo "ERROR: $EXPORT_FILE does not match the export schema (expected a top-level \"records\" array)." >&2
  echo "See docs/DEPLOYMENT.md#registry-export--import for the schema." >&2
  exit 2
fi

echo "==> Validating $EXPORT_FILE against ${CONTRACT_ID} (${NETWORK})" >&2

mismatches=0
missing_onchain=0
address_mismatch=0
verified_mismatch=0
missing_from_export=0
record_count="$(jq '.records | length' "$EXPORT_FILE")"

for ((i = 0; i < record_count; i++)); do
  username="$(jq -r ".records[$i].github_username" "$EXPORT_FILE")"
  exp_address="$(jq -r ".records[$i].stellar_address" "$EXPORT_FILE")"
  exp_verified="$(jq -r ".records[$i].verified" "$EXPORT_FILE")"

  onchain="$("$STELLAR" contract invoke \
    --id "$CONTRACT_ID" \
    --source-account "$SOURCE" \
    --network "$NETWORK" \
    -- get_address --github-username "$username" 2>/dev/null || echo null)"

  if [[ "$onchain" == "null" ]]; then
    echo "MISSING_ONCHAIN    ${username}  (export has ${exp_address}, verified=${exp_verified})"
    mismatches=$((mismatches + 1))
    missing_onchain=$((missing_onchain + 1))
    continue
  fi

  onchain_address="$(jq -r '.stellar_address' <<<"$onchain")"
  onchain_verified="$(jq -r '.verified' <<<"$onchain")"

  if [[ "$onchain_address" != "$exp_address" ]]; then
    echo "ADDRESS_MISMATCH   ${username}  export=${exp_address} onchain=${onchain_address}"
    mismatches=$((mismatches + 1))
    address_mismatch=$((address_mismatch + 1))
  elif [[ "$onchain_verified" != "$exp_verified" ]]; then
    echo "VERIFIED_MISMATCH  ${username}  export=${exp_verified} onchain=${onchain_verified}"
    mismatches=$((mismatches + 1))
    verified_mismatch=$((verified_mismatch + 1))
  fi
done

if [[ -n "$ADMIN_SOURCE" ]]; then
  echo "==> Checking for on-chain registrations missing from the export (admin read)..." >&2

  onchain_usernames="$(mktemp)"
  trap 'rm -f "$onchain_usernames"' EXIT

  cursor=0
  while :; do
    page="$("$STELLAR" contract invoke \
      --id "$CONTRACT_ID" \
      --source-account "$ADMIN_SOURCE" \
      --network "$NETWORK" \
      -- get_registered_paginated --cursor "$cursor" --limit "$PAGE_LIMIT")"

    jq -r '.records[][0]' <<<"$page" >> "$onchain_usernames"

    has_more="$(jq -r '.has_more' <<<"$page")"
    next_cursor="$(jq -r '.next_cursor' <<<"$page")"
    [[ "$has_more" == "true" && "$next_cursor" != "null" ]] || break
    cursor="$next_cursor"
  done

  while IFS= read -r uname; do
    [[ -z "$uname" ]] && continue
    if ! jq -e --arg u "$uname" '.records[] | select(.github_username == $u)' "$EXPORT_FILE" >/dev/null; then
      echo "MISSING_FROM_EXPORT ${uname}"
      mismatches=$((mismatches + 1))
      missing_from_export=$((missing_from_export + 1))
    fi
  done < "$onchain_usernames"
else
  echo "==> ADMIN_SOURCE not set — skipping the on-chain-only diff." >&2
  echo "    Set ADMIN_SOURCE=<admin identity> to also detect registrations missing from the export." >&2
fi

if [[ -n "$REPORT_FILE" ]]; then
  # Pinned JSON shape (v1): field names and order are a compatibility surface
  # for dashboard parsers — see docs/ABI.md#dry-run-repair-report.
  if [[ -n "$ADMIN_SOURCE" ]]; then admin_diff_enabled=true; else admin_diff_enabled=false; fi
  export_verified_count="$(jq '[.records[] | select(.verified == true)] | length' "$EXPORT_FILE")"
  exported_at="$(jq -r '.exported_at // ""' "$EXPORT_FILE")"

  stats="$("$STELLAR" contract invoke \
    --id "$CONTRACT_ID" \
    --source-account "$SOURCE" \
    --network "$NETWORK" \
    -- get_stats 2>/dev/null || echo null)"

  if jq -e 'type == "object" and has("total") and has("verified")' <<<"$stats" >/dev/null 2>&1; then
    onchain_total="$(jq -r '.total' <<<"$stats")"
    onchain_verified="$(jq -r '.verified' <<<"$stats")"
    if [[ "$onchain_total" == "$record_count" && "$onchain_verified" == "$export_verified_count" ]]; then
      counter_parity=true
    else
      counter_parity=false
    fi
  else
    onchain_total=null
    onchain_verified=null
    counter_parity=null
  fi

  result=ok
  if [[ "$mismatches" -gt 0 || "$counter_parity" == "false" ]]; then
    result=mismatch
  fi

  jq -n \
    --argjson schema_version 1 \
    --arg tool "validate_registry" \
    --arg contract_id "$CONTRACT_ID" \
    --arg network "$NETWORK" \
    --arg generated_at "$(date -u +"%Y-%m-%dT%H:%M:%SZ")" \
    --arg exported_at "$exported_at" \
    --argjson checked_records "$record_count" \
    --argjson export_verified "$export_verified_count" \
    --argjson onchain_total "$onchain_total" \
    --argjson onchain_verified "$onchain_verified" \
    --argjson missing_onchain "$missing_onchain" \
    --argjson address_mismatch "$address_mismatch" \
    --argjson verified_mismatch "$verified_mismatch" \
    --argjson missing_from_export "$missing_from_export" \
    --argjson total_mismatches "$mismatches" \
    --argjson admin_diff_enabled "$admin_diff_enabled" \
    --argjson counter_parity "$counter_parity" \
    --arg result "$result" \
    '{
      schema_version: $schema_version,
      tool: $tool,
      contract_id: $contract_id,
      network: $network,
      generated_at: $generated_at,
      export: {
        exported_at: $exported_at,
        total: $checked_records,
        verified: $export_verified
      },
      counters: {
        onchain_total: $onchain_total,
        onchain_verified: $onchain_verified,
        parity: $counter_parity
      },
      mismatches: {
        missing_onchain: $missing_onchain,
        address_mismatch: $address_mismatch,
        verified_mismatch: $verified_mismatch,
        missing_from_export: $missing_from_export,
        total: $total_mismatches
      },
      admin_diff_enabled: $admin_diff_enabled,
      result: $result
    }' > "$REPORT_FILE"

  echo "==> Wrote dry-run repair report to ${REPORT_FILE}" >&2
fi

echo "" >&2
if [[ "$mismatches" -eq 0 ]]; then
  echo "OK: export matches live registry (${record_count} record(s) checked)." >&2
  exit 0
else
  echo "FOUND ${mismatches} mismatch(es). Review before using this export for a restore." >&2
  exit 1
fi
