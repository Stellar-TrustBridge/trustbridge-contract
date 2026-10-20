#!/usr/bin/env bash
# Offline checks for scripts/futurenet_smoke_test.sh (Issue #687).
#
# Exercises the dry-run happy path and the failure paths documented in
# docs/FUTURENET_ONBOARDING.md#troubleshooting-wasm-install-failures without
# contacting the network.
#
# Usage:  ./scripts/test_futurenet_smoke.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SMOKE="$ROOT/scripts/futurenet_smoke_test.sh"
FAILED=0

expect() {
  local name="$1" want_status="$2" want_text="$3"
  shift 3
  local out status=0
  out="$(env "$@" "$SMOKE" 2>&1)" || status=$?
  if [[ "$status" != "$want_status" ]] || ! grep -qF -- "$want_text" <<<"$out"; then
    printf 'FAIL %s: status=%s output:\n%s\n' "$name" "$status" "$out" >&2
    FAILED=1
  else
    printf 'ok   %s\n' "$name"
  fi
}

expect "dry run prints deploy plan" 0 "DRY RUN: Futurenet RPC" DRY_RUN=true
expect "missing ADMIN fails" 1 "ADMIN must be set" DRY_RUN=false ADMIN=
expect "missing stellar CLI points to troubleshooting" 1 "troubleshooting-wasm-install-failures" \
  DRY_RUN=false ADMIN=GTEST STELLAR=/nonexistent/stellar-cli

exit "$FAILED"
