#!/usr/bin/env bash
# Run a non-destructive export/validate round-trip against a disposable
# local or test network contract instance.
#
# Required: CONTRACT_ID, SOURCE, ADMIN_SOURCE
# Optional: NETWORK (default: testnet), PAGE_LIMIT (default: 1),
#           EXPECTED_COUNT (check the exported record count when set).

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

: "${CONTRACT_ID:?CONTRACT_ID must be set to the disposable contract ID}"
: "${SOURCE:?SOURCE must be set to the contract admin identity}"
: "${ADMIN_SOURCE:?ADMIN_SOURCE must be set to the contract admin identity}"

NETWORK="${NETWORK:-testnet}"
PAGE_LIMIT="${PAGE_LIMIT:-1}"
STELLAR="${STELLAR:-stellar}"
EXPECTED_COUNT="${EXPECTED_COUNT:-}"

if [[ ! "$PAGE_LIMIT" =~ ^[1-9][0-9]*$ ]]; then
  echo "ERROR: PAGE_LIMIT must be a positive integer." >&2
  exit 2
fi

export_file="$(mktemp "${TMPDIR:-/tmp}/trustbridge-registry-export.XXXXXX.json")"
trap 'rm -f "$export_file"' EXIT

echo "==> Running export/validate round-trip (${NETWORK}, page size ${PAGE_LIMIT})" >&2
CONTRACT_ID="$CONTRACT_ID" SOURCE="$SOURCE" NETWORK="$NETWORK" \
  PAGE_LIMIT="$PAGE_LIMIT" STELLAR="$STELLAR" OUTPUT_FILE="$export_file" \
  "$ROOT/scripts/export_registry.sh"

if [[ -n "$EXPECTED_COUNT" ]] && [[ "$(jq -r '.count' "$export_file")" != "$EXPECTED_COUNT" ]]; then
  echo "ERROR: expected ${EXPECTED_COUNT} record(s), export contains $(jq -r '.count' "$export_file")." >&2
  exit 1
fi

CONTRACT_ID="$CONTRACT_ID" NETWORK="$NETWORK" SOURCE="$SOURCE" \
  ADMIN_SOURCE="$ADMIN_SOURCE" PAGE_LIMIT="$PAGE_LIMIT" STELLAR="$STELLAR" \
  "$ROOT/scripts/validate_registry.sh" "$export_file"

echo "PASS: disaster-recovery export/validate round-trip completed without writes." >&2