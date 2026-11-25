#!/usr/bin/env bash
# Unit test for wasm-hash.pin verification logic and WASM_HASH_STRICT modes (Issue #759).
#
# Validates:
#   1. Happy path: strict mode (WASM_HASH_STRICT=1) passes when actual hash matches pinned hash.
#   2. Happy path: permissive mode (WASM_HASH_STRICT=0) allows PLACEHOLDER with a warning.
#   3. Failure path 1: strict mode (WASM_HASH_STRICT=1) fails when pin contains PLACEHOLDER.
#   4. Failure path 2: hash mismatch fails with error naming expected and actual hashes.
#   5. Failure path 3: missing WASM artifact fails with error requesting 'make build'.
#   6. Pin rotation: wasm-hash-update rewrites PLACEHOLDER/old hash and enables strict verification.
#
# Usage: bash scripts/test_wasm_hash_pin.sh

set -euo pipefail

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

FAILED=0
fail() { printf 'test_wasm_hash_pin: FAIL: %s\n' "$1" >&2; FAILED=1; }

# Mock WASM artifact and pin verification logic directly mirroring Makefile
MOCK_WASM="$TMP/mock_contract.wasm"
printf "mock-soroban-wasm-bytecode-v1\n" > "$MOCK_WASM"

ACTUAL_HASH=$(sha256sum "$MOCK_WASM" | awk '{print $1}')

run_check() {
  local WASM_FILE="$1"
  local PIN_FILE="$2"
  local STRICT="${3:-0}"

  if [ ! -f "$WASM_FILE" ]; then
    echo "ERROR: No WASM artifact found. Run 'make build' first."
    return 1
  fi

  local ACTUAL
  ACTUAL=$(sha256sum "$WASM_FILE" | awk '{print $1}')
  echo "WASM SHA-256: $ACTUAL"

  local PINNED
  PINNED=$(grep -v '^#' "$PIN_FILE" 2>/dev/null | grep -v '^$' | tr -d '[:space:]' || true)

  if [ "$PINNED" = "PLACEHOLDER" ]; then
    echo "WARNING: wasm-hash.pin contains PLACEHOLDER — run 'make wasm-hash-update' to pin."
    if [ "$STRICT" = "1" ]; then
      echo "ERROR: WASM_HASH_STRICT=1 requires a real pinned hash."
      return 1
    fi
  elif [ "$ACTUAL" != "$PINNED" ]; then
    echo "ERROR: WASM hash mismatch! Expected: $PINNED  Actual: $ACTUAL"
    echo "Run 'make wasm-hash-update' if this change is intentional."
    return 1
  else
    echo "OK: WASM hash matches pin."
  fi
  return 0
}

# ── Test 1: Happy Path — Strict Mode (WASM_HASH_STRICT=1) with Matching Pin ──
PIN_1="$TMP/pin_matching.pin"
printf "# Pinned release hash\n%s\n" "$ACTUAL_HASH" > "$PIN_1"
OUT_1="$TMP/out_1.log"
if ! run_check "$MOCK_WASM" "$PIN_1" "1" > "$OUT_1" 2>&1; then
  fail "Happy path strict mode failed unexpectedly"
fi
if ! grep -q "OK: WASM hash matches pin." "$OUT_1"; then
  fail "Happy path output missing 'OK: WASM hash matches pin.'"
fi

# ── Test 2: Happy Path — Permissive Mode (WASM_HASH_STRICT=0) with PLACEHOLDER
PIN_2="$TMP/pin_placeholder.pin"
printf "# Placeholder\nPLACEHOLDER\n" > "$PIN_2"
OUT_2="$TMP/out_2.log"
if ! run_check "$MOCK_WASM" "$PIN_2" "0" > "$OUT_2" 2>&1; then
  fail "Permissive mode with PLACEHOLDER failed unexpectedly"
fi
if ! grep -q "WARNING: wasm-hash.pin contains PLACEHOLDER" "$OUT_2"; then
  fail "Permissive mode output missing PLACEHOLDER warning"
fi

# ── Test 3: Failure Path 1 — Strict Mode (WASM_HASH_STRICT=1) Rejects PLACEHOLDER
OUT_3="$TMP/out_3.log"
STATUS_3=0
run_check "$MOCK_WASM" "$PIN_2" "1" > "$OUT_3" 2>&1 || STATUS_3=$?
if [ "$STATUS_3" -eq 0 ]; then
  fail "Strict mode must fail when pin is PLACEHOLDER"
fi
if ! grep -q "ERROR: WASM_HASH_STRICT=1 requires a real pinned hash." "$OUT_3"; then
  fail "Strict mode missing error message 'ERROR: WASM_HASH_STRICT=1 requires a real pinned hash.'"
fi

# ── Test 4: Failure Path 2 — Hash Mismatch (Actual != Pinned) ────────────────
PIN_4="$TMP/pin_mismatch.pin"
printf "# Stale hash\n0000000000000000000000000000000000000000000000000000000000000000\n" > "$PIN_4"
OUT_4="$TMP/out_4.log"
STATUS_4=0
run_check "$MOCK_WASM" "$PIN_4" "1" > "$OUT_4" 2>&1 || STATUS_4=$?
if [ "$STATUS_4" -eq 0 ]; then
  fail "Strict mode must fail on hash mismatch"
fi
if ! grep -q "ERROR: WASM hash mismatch! Expected:" "$OUT_4"; then
  fail "Mismatch output missing 'ERROR: WASM hash mismatch! Expected:'"
fi
if ! grep -q "Run 'make wasm-hash-update' if this change is intentional." "$OUT_4"; then
  fail "Mismatch output missing actionable update command guidance"
fi

# ── Test 5: Failure Path 3 — Missing WASM Artifact ───────────────────────────
OUT_5="$TMP/out_5.log"
STATUS_5=0
run_check "$TMP/nonexistent.wasm" "$PIN_1" "1" > "$OUT_5" 2>&1 || STATUS_5=$?
if [ "$STATUS_5" -eq 0 ]; then
  fail "Verification must fail when WASM artifact is missing"
fi
if ! grep -q "ERROR: No WASM artifact found. Run 'make build' first." "$OUT_5"; then
  fail "Missing artifact output missing 'ERROR: No WASM artifact found.'"
fi

# ── Test 6: Pin Rotation Workflow (wasm-hash-update simulation) ──────────────
PIN_ROT="$TMP/pin_rotation.pin"
printf "PLACEHOLDER\n" > "$PIN_ROT"
# Rotate pin using sed matching Makefile wasm-hash-update
sed -i "s/^PLACEHOLDER$/$ACTUAL_HASH/" "$PIN_ROT"
sed -i "s/^[a-f0-9]\{64\}$/$ACTUAL_HASH/" "$PIN_ROT"

OUT_6="$TMP/out_6.log"
if ! run_check "$MOCK_WASM" "$PIN_ROT" "1" > "$OUT_6" 2>&1; then
  fail "Strict mode check failed after pin rotation"
fi
if ! grep -q "OK: WASM hash matches pin." "$OUT_6"; then
  fail "Rotated pin verification did not report match"
fi

if (( FAILED )); then
  echo "test_wasm_hash_pin: FAILED — see above." >&2
  exit 1
fi

echo "test_wasm_hash_pin: OK — all happy path and failure path cases passed."
