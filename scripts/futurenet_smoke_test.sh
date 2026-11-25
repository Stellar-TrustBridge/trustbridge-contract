#!/usr/bin/env bash
# Wave #39: Futurenet deploy smoke workflow.
#
# Deploys trustbridge-contract to Futurenet and exercises the read-only
# entry points to validate the threat model in docs/SECURITY.md before an
# audit: initialization gating, has_record lookups, and stats reads.
#
# Usage:
#   ADMIN=G... RPC_URL=https://rpc-futurenet.stellar.org ./scripts/futurenet_smoke_test.sh
#   DRY_RUN=true ./scripts/futurenet_smoke_test.sh
#
# This does not register real data; it is a deploy sanity check, not a
# functional test suite (see `cargo test` / tests/integration.rs for that).

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ADMIN="${ADMIN:-}"
SOURCE="${SOURCE:-default}"
RPC_URL="${RPC_URL:-https://rpc-futurenet.stellar.org}"
FRIENDBOT_URL="${FRIENDBOT_URL:-https://friendbot-futurenet.stellar.org}"
IDENTITY="${IDENTITY:-$SOURCE}"
DRY_RUN="${DRY_RUN:-false}"
STELLAR="${STELLAR:-stellar}"

if [[ "$DRY_RUN" == "true" ]]; then
  echo "DRY RUN: Futurenet RPC: ${RPC_URL}"
  echo "DRY RUN: Fund identity '${IDENTITY}' from ${FRIENDBOT_URL}/?addr=<G-address>"
  echo "DRY RUN: NETWORK=futurenet INIT=true ADMIN=<G-address> SOURCE=${SOURCE} RPC_URL=${RPC_URL} ./scripts/deploy.sh"
  echo "DRY RUN: stellar contract invoke --network futurenet --rpc-url ${RPC_URL} -- get_health_probes"
  echo "DRY RUN: stellar contract invoke --network futurenet --rpc-url ${RPC_URL} -- get_stats"
  echo "DRY RUN: stellar contract invoke --network futurenet --rpc-url ${RPC_URL} -- has_record --github_username smoke-test-user"
  exit 0
fi

if [[ -z "$ADMIN" ]]; then
  echo "ERROR: ADMIN must be set to a Futurenet G-address."
  exit 1
fi

# Friendbot throttles funding per account/IP and per time window. Match the
# wording used by the CLI/Horizon so operators get the workaround instead of
# an opaque deploy failure (see docs/FUTURENET_ONBOARDING.md).
FUNDING_ERROR_PATTERN='rate.?limit|too many requests|429|friendbot|fund|insufficient balance|insufficient fee|account not found'

deploy_log="$(mktemp)"
trap 'rm -f "$deploy_log"' EXIT

echo "==> Deploying to Futurenet with ADMIN=$ADMIN"
if ! NETWORK=futurenet INIT=true ADMIN="$ADMIN" "$ROOT/scripts/deploy.sh" 2>&1 | tee "$deploy_log"; then
  if grep -Eqi "$FUNDING_ERROR_PATTERN" "$deploy_log"; then
    echo "ERROR: Futurenet deploy failed at the faucet/funding step." >&2
    echo "Friendbot rate limits apply per account/IP and per time window." >&2
    echo "Reuse an already-funded identity, back off and retry with increasing" >&2
    echo "delays, or fall back to testnet. Details:" >&2
    echo "  docs/FUTURENET_ONBOARDING.md#faucet-rate-limits-and-workarounds" >&2
    echo "  docs/DEPLOYMENT.md#futurenet-smoke-test" >&2
  else
    echo "ERROR: Futurenet deploy failed for a reason other than funding." >&2
    echo "Review the deploy output above before retrying." >&2
  fi
  exit 1
fi

smoke_invoke() {
  local label="$1"
  shift
  echo "==> Smoke: $label"
  if ! "$@"; then
    echo "ERROR: smoke check failed: $label" >&2
    echo "See docs/DEPLOYMENT.md#futurenet-smoke-test for expected results." >&2
    exit 1
  fi
}
NETWORK=futurenet INIT=true ADMIN="$ADMIN" SOURCE="$SOURCE" RPC_URL="$RPC_URL" STELLAR="$STELLAR" "$ROOT/scripts/deploy.sh"

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

echo "==> Smoke: get_health_probes after init should report unpaused, total=0, verified=0"
"$STELLAR" contract invoke --id "$CONTRACT_ID" --source-account "$SOURCE" --network futurenet --rpc-url "$RPC_URL" -- get_health_probes

echo "==> Smoke: get_stats on a fresh deploy should be {total: 0, verified: 0, ever_verified: 0}"
smoke_invoke "get_stats on a fresh deploy should be {total: 0, verified: 0}" \
  stellar contract invoke --id "$CONTRACT_ID" --source-account "${SOURCE:-default}" \
  --network futurenet -- get_stats

smoke_invoke "has_record on an unregistered username should be false" \
  stellar contract invoke --id "$CONTRACT_ID" --source-account "${SOURCE:-default}" \
  --network futurenet -- has_record --github-username smoke-test-user
echo "==> Smoke: get_stats on a fresh deploy should be {total: 0, verified: 0}"
"$STELLAR" contract invoke --id "$CONTRACT_ID" --source-account "$SOURCE" --network futurenet --rpc-url "$RPC_URL" -- get_stats

echo "==> Smoke: has_record on an unregistered username should be false"
"$STELLAR" contract invoke --id "$CONTRACT_ID" --source-account "$SOURCE" --network futurenet --rpc-url "$RPC_URL" -- has_record --github_username smoke-test-user

echo "==> Futurenet smoke checks passed for ${CONTRACT_ID}"
