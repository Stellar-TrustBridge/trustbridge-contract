#!/usr/bin/env bash
# Deploy trustbridge-contract to Stellar Testnet or Mainnet.
#
# Usage:
#   NETWORK=testnet ADMIN=G... ./scripts/deploy.sh
#   NETWORK=mainnet ADMIN=G... SOURCE=ops ./scripts/deploy.sh
#
# Environment variables:
#   NETWORK   — testnet | mainnet | futurenet (default: testnet)
#   ADMIN     — G-address of the contract admin (required)
#   SOURCE    — Stellar CLI identity name (default: default)
#   ALIAS     — Contract alias for Stellar CLI (default: trustbridge)
#   INIT      — Run initialize after deploy: true | false (default: true)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

NETWORK="${NETWORK:-testnet}"
ADMIN="${ADMIN:-}"
SOURCE="${SOURCE:-default}"
ALIAS="${ALIAS:-trustbridge}"
INIT="${INIT:-true}"
STELLAR="${STELLAR:-stellar}"

WASM_V1="target/wasm32v1-none/release/trustbridge-contract.wasm"
WASM_LEGACY="target/wasm32-unknown-unknown/release/trustbridge-contract.wasm"

if [[ -z "$ADMIN" ]]; then
  echo "ERROR: ADMIN must be set to the Stellar G-address of the contract admin."
  echo "Example: NETWORK=testnet ADMIN=GABC... ./scripts/deploy.sh"
  exit 1
fi

if [[ ! -f "$WASM_V1" && ! -f "$WASM_LEGACY" ]]; then
  echo "WASM not found. Building contract..."
  make build
fi

if [[ -f "$WASM_V1" ]]; then
  WASM="$WASM_V1"
else
  WASM="$WASM_LEGACY"
fi

echo "==> Deploying to ${NETWORK}"
echo "    Source account : ${SOURCE}"
echo "    Admin address  : ${ADMIN}"
echo "    WASM           : ${WASM}"

CONTRACT_ID="$("$STELLAR" contract deploy \
  --wasm "$WASM" \
  --source-account "$SOURCE" \
  --network "$NETWORK" \
  --alias "$ALIAS")"

echo ""
echo "==> Deployed contract ID: ${CONTRACT_ID}"

if [[ "$INIT" == "true" ]]; then
  echo "==> Initializing contract with admin ${ADMIN}..."
  "$STELLAR" contract invoke \
    --id "$CONTRACT_ID" \
    --source-account "$SOURCE" \
    --network "$NETWORK" \
    --send=yes \
    -- initialize --admin "$ADMIN"
  echo "==> Initialization complete."
fi

mkdir -p deployments
DEPLOY_FILE="deployments/${NETWORK}.json"
cat > "$DEPLOY_FILE" <<EOF
{
  "network": "${NETWORK}",
  "contract_id": "${CONTRACT_ID}",
  "admin": "${ADMIN}",
  "alias": "${ALIAS}",
  "wasm": "${WASM}",
  "deployed_at": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
}
EOF

echo ""
echo "Deployment record written to ${DEPLOY_FILE}"
echo ""
echo "Next steps:"
echo "  export CONTRACT_ID=${CONTRACT_ID}"
echo "  make invoke-register GITHUB_USER=octocat STELLAR_ADDR=G... CONTRACT_ID=\$CONTRACT_ID"
