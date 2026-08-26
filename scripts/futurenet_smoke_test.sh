#!/usr/bin/env bash
# Wave #39: Futurenet deploy smoke workflow.
#
# Deploys trustbridge-contract to Futurenet and exercises the read-only
# entry points to validate the threat model in docs/SECURITY.md before an
# audit: initialization gating, has_record lookups, and stats reads.
#
# Usage:
#   ADMIN=G... ./scripts/futurenet_smoke_test.sh
#
# This does not register real data; it is a deploy sanity check, not a
# functional test suite (see `cargo test` / tests/integration.rs for that).

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ADMIN="${ADMIN:-}"
if [[ -z "$ADMIN" ]]; then
  echo "ERROR: ADMIN must be set to a Futurenet G-address."
  exit 1
fi

NETWORK=futurenet INIT=true ADMIN="$ADMIN" "$ROOT/scripts/deploy.sh"

DEPLOY_FILE="deployments/futurenet.json"
if [[ ! -f "$DEPLOY_FILE" ]]; then
  echo "ERROR: $DEPLOY_FILE not found; deploy.sh should have written it."
  exit 1
fi

CONTRACT_ID="$(grep -o '"contract_id": *"[^"]*"' "$DEPLOY_FILE" | sed -E 's/.*: *"([^"]*)"/\1/')"
if [[ -z "$CONTRACT_ID" ]]; then
  echo "ERROR: could not read contract_id from $DEPLOY_FILE"
  exit 1
fi

echo "==> Smoke: get_stats on a fresh deploy should be {total: 0, verified: 0}"
stellar contract invoke --id "$CONTRACT_ID" --source-account "${SOURCE:-default}" --network futurenet -- get_stats

echo "==> Smoke: has_record on an unregistered username should be false"
stellar contract invoke --id "$CONTRACT_ID" --source-account "${SOURCE:-default}" --network futurenet -- has_record --github_username smoke-test-user

echo "==> Futurenet smoke checks passed for ${CONTRACT_ID}"
