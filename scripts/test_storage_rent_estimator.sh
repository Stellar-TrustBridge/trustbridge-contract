#!/usr/bin/env bash
# Golden-output test for scripts/storage_rent_estimator.py (Issue #290).
#
# Runs the estimator with a fixed input and diffs stdout against the checked-in
# golden fixture. Also asserts the chunk-size mismatch warning fires when the
# inputs JSON disagrees with src/storage.rs.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

GOLDEN="scripts/testdata/storage-rent-estimator.golden.txt"

actual="$(python3 scripts/storage_rent_estimator.py --users 250 --roles 3 --lastact 100)"
if ! diff -u "$GOLDEN" <(printf '%s\n' "$actual"); then
  echo "FAIL: estimator output does not match $GOLDEN" >&2
  exit 1
fi

# Mismatch detection: feed a storage.rs stand-in with a different CHUNK_SIZE.
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
printf 'pub const CHUNK_SIZE: u32 = 99;\n' > "$tmp"
if python3 scripts/storage_rent_estimator.py --users 10 --storage-rs "$tmp" 2>&1 >/dev/null \
  | grep -q "chunk_size mismatch"; then
  :
else
  echo "FAIL: expected a chunk_size mismatch warning" >&2
  exit 1
fi

echo "PASS: storage_rent_estimator golden output and mismatch warning"
