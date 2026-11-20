#!/usr/bin/env bash
# Golden-output test for scripts/storage_rent_estimator.py (Issues #290, #750).
#
# Runs the estimator with a fixed input and diffs stdout against the checked-in
# golden fixture, asserts the chunk-size mismatch warning fires when the inputs
# JSON disagrees with src/storage.rs, and asserts that invalid inputs fail with
# the stable error messages documented in docs/STORAGE_RENT_ESTIMATOR.md.
#
# Refresh the golden after an intentional change (see docs/CONTRIBUTING.md,
# "Storage rent estimator golden refresh"):
#
#   UPDATE_GOLDEN=1 bash scripts/test_storage_rent_estimator.sh   # or: make storage-rent-estimator-golden
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

GOLDEN="scripts/testdata/storage-rent-estimator.golden.txt"
# Fixed golden invocation — keep in sync with docs/CONTRIBUTING.md.
GOLDEN_ARGS=(--users 250 --roles 3 --lastact 100)

estimator() { python3 scripts/storage_rent_estimator.py "$@"; }

actual="$(estimator "${GOLDEN_ARGS[@]}")"

if [[ "${UPDATE_GOLDEN:-0}" == "1" ]]; then
  printf '%s\n' "$actual" > "$GOLDEN"
  echo "UPDATED: $GOLDEN — review with 'git diff $GOLDEN' and commit it with the inputs/doc changes."
  exit 0
fi

if ! diff -u "$GOLDEN" <(printf '%s\n' "$actual"); then
  echo "FAIL: estimator output differs from $GOLDEN." >&2
  echo "If the change is intentional, follow docs/CONTRIBUTING.md (Storage rent estimator golden refresh):" >&2
  echo "  make storage-rent-estimator-golden   # UPDATE_GOLDEN=1, then review the diff" >&2
  exit 1
fi

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

# Mismatch detection: feed a storage.rs stand-in with a different CHUNK_SIZE.
printf 'pub const CHUNK_SIZE: u32 = 99;\n' > "$tmpdir/storage.rs"
if ! estimator --users 10 --storage-rs "$tmpdir/storage.rs" 2>&1 >/dev/null | grep -q "chunk_size mismatch"; then
  echo "FAIL: expected a chunk_size mismatch warning" >&2
  exit 1
fi

# Failure paths: each must exit 2 with its documented stable message.
expect_error() {
  local want="$1"
  shift
  local out status=0
  out="$(estimator "$@" 2>&1 >/dev/null)" || status=$?
  if [[ "$status" -ne 2 ]] || ! grep -qF -- "$want" <<<"$out"; then
    echo "FAIL: expected exit 2 with '$want' for: $*" >&2
    echo "  got exit $status: $out" >&2
    exit 1
  fi
}

printf '{not json' > "$tmpdir/malformed.json"
printf '{"estimator_inputs_version": 2}' > "$tmpdir/v2.json"
printf '{"estimator_inputs_version": 1, "on_chain": {"layout": {}}}' > "$tmpdir/incomplete.json"

expect_error "inputs file not found:" --users 1 --inputs "$tmpdir/does-not-exist.json"
expect_error "inputs file is not valid JSON:" --users 1 --inputs "$tmpdir/malformed.json"
expect_error "unsupported estimator_inputs_version 2; expected 1" --users 1 --inputs "$tmpdir/v2.json"
expect_error "inputs JSON is missing required field: on_chain.layout.chunk_size" --users 1 --inputs "$tmpdir/incomplete.json"
expect_error "--users must be >= 0" --users -1

echo "PASS: storage_rent_estimator golden output, mismatch warning, and 5 invalid-input cases"
