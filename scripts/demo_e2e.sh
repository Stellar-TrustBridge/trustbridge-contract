#!/usr/bin/env bash
# Cross-repo E2E demo script for trustbridge-contract.
#
# Walks a happy path: register → verify → lookup → export.
# Intended for sibling TrustBridge repos (dashboard, indexer) to call from
# their own CI or local dev loops.
#
# Usage:
#   CONTRACT_ID=C... SOURCE=demo ADMIN=G... ./scripts/demo_e2e.sh
#
# Secrets are passed via env; nothing is committed.
# Defaults to testnet; does NOT hit mainnet.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CONTRACT_ID="${CONTRACT_ID:-}"
SOURCE="${SOURCE:-default}"
ADMIN="${ADMIN:-}"
NETWORK="${NETWORK:-testnet}"
STELLAR="${STELLAR:-stellar}"
GITHUB_USER="${E2E_GITHUB_USER:-e2e-demo-user}"
STELLAR_ADDR="${E2E_STELLAR_ADDR:-}"

if [[ -z "$CONTRACT_ID" ]]; then
  echo "ERROR: set CONTRACT_ID=<C...> for this script."
  exit 1
fi
if [[ -z "$ADMIN" ]]; then
  echo "ERROR: set ADMIN to the G-address of the contract admin."
  exit 1
fi
if [[ -z "$STELLAR_ADDR" ]]; then
  echo "ERROR: set E2E_STELLAR_ADDR=<G...> for the demo registration."
  exit 1
fi

export NETWORK SOURCE

step() {
  echo ""
  echo "==> [$STEP] $1"
  STEP=$((STEP + 1))
}

STEP=1

step "Pre-flight: verify contract is initialized"
$STELLAR contract invoke \
  --id "$CONTRACT_ID" \
  --source-account "$SOURCE" \
  --network "$NETWORK" \
  -- get_stats

step "Register: $GITHUB_USER → $STELLAR_ADDR"
$STELLAR contract invoke \
  --id "$CONTRACT_ID" \
  --source-account "$SOURCE" \
  --network "$NETWORK" \
  --send=yes \
  -- register \
  --github-username "$GITHUB_USER" \
  --stellar-address "$STELLAR_ADDR"

step "Lookup: confirm registration"
$STELLAR contract invoke \
  --id "$CONTRACT_ID" \
  --source-account "$SOURCE" \
  --network "$NETWORK" \
  -- get_address \
  --github-username "$GITHUB_USER"

step "Verify: mark contributor as verified (admin call)"
$STELLAR contract invoke \
  --id "$CONTRACT_ID" \
  --source-account "$SOURCE" \
  --network "$NETWORK" \
  --send=yes \
  -- verify \
  --caller "$ADMIN" \
  --github-username "$GITHUB_USER"

step "Export: paginated admin read (cursor=0, limit=10)"
$STELLAR contract invoke \
  --id "$CONTRACT_ID" \
  --source-account "$SOURCE" \
  --network "$NETWORK" \
  -- get_registered_paginated \
  --cursor 0 \
  --limit 10

step "Public export: unauthenticated dashboard read"
$STELLAR contract invoke \
  --id "$CONTRACT_ID" \
  --source-account "$SOURCE" \
  --network "$NETWORK" \
  -- get_public_paginated \
  --cursor 0 \
  --limit 10

echo ""
echo "==> E2E demo completed for ${GITHUB_USER} on ${NETWORK} (${CONTRACT_ID})"
echo "==> Next: remove the demo registration if desired:"
echo "     $STELLAR contract invoke --id $CONTRACT_ID --source-account $SOURCE --network $NETWORK --send=yes \\"
echo "       -- remove --caller $STELLAR_ADDR --github-username $GITHUB_USER"
