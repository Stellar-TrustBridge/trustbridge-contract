#!/usr/bin/env bash
# Generate a verified-only payout allowlist CSV from on-chain registry state
# (Issue #285). Reads the unauthenticated get_public_paginated endpoint — no
# admin credentials required — filters to verified records, and writes one CSV
# row per contributor. Read-only; payment submission is out of scope.
#
# Usage:
#   CONTRACT_ID=C... NETWORK=testnet ./scripts/payout_allowlist.sh
#   CONTRACT_ID=C... NETWORK=testnet OUTPUT_FILE=allowlist.csv ./scripts/payout_allowlist.sh
#
# Environment variables:
#   CONTRACT_ID  — deployed contract ID (required)
#   SOURCE       — Stellar CLI identity to sign the read (default: "default";
#                  any funded identity works, no admin role needed)
#   NETWORK      — testnet | mainnet | futurenet (default: testnet)
#   OUTPUT_FILE  — CSV path (default: payout-allowlist-<network>.csv)
#   PAGE_LIMIT   — records per page (default: 100, the contract's MAX_PAGE_LIMIT)
#
# Extra flags are forwarded to payout_allowlist.py, e.g. --include-unverified.
#
# Output columns: github_username,payout_address,stellar_address,verified,registered_at

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

exec python3 "$ROOT/scripts/payout_allowlist.py" "$@"
