#!/usr/bin/env bash
# check_fixture_pairing.sh — require Rust fixture changes to ship with a TypeScript update.
#
# USAGE
#   scripts/check_fixture_pairing.sh [BASE_REF]
#
#   BASE_REF defaults to origin/main. Compares BASE_REF...HEAD and fails when the
#   Rust side of the differential fixtures changed but ts-differential-tests/ did
#   not (or the reverse for the TypeScript decoder), so cross-language drift is
#   caught in review instead of by downstream parsers.
#
# EXIT CODES
#   0  No fixture changes, or both sides changed together.
#   1  Only one side of the fixture pair changed.
#   2  Usage / setup error (BASE_REF not found).

set -euo pipefail

BASE_REF="${1:-origin/main}"

if ! git rev-parse --verify --quiet "$BASE_REF" >/dev/null; then
  echo "error: base ref '$BASE_REF' not found (fetch it first)" >&2
  exit 2
fi

changed="$(git diff --name-only "$BASE_REF"...HEAD)"

rust_side="$(grep -E '^(tests/testdata/bindings/|tests/generate_xdr_fixtures\.rs$|tests/bindings_golden\.rs$|abi/export_page\.layout\.golden$)' <<<"$changed" || true)"
ts_side="$(grep -E '^ts-differential-tests/' <<<"$changed" | grep -vE '/(README\.md|package-lock\.json)$' || true)"

if [[ -n "$rust_side" && -z "$ts_side" ]]; then
  echo "error: Rust fixtures changed without a ts-differential-tests/ update:" >&2
  sed 's/^/  /' <<<"$rust_side" >&2
  echo "Update the TypeScript decoder/tests in the same PR (see ts-differential-tests/README.md)." >&2
  exit 1
fi

if [[ -z "$rust_side" && -n "$ts_side" ]] && grep -qE '^ts-differential-tests/fixtures/' <<<"$ts_side"; then
  echo "error: TypeScript fixtures changed without a matching Rust golden update:" >&2
  sed 's/^/  /' <<<"$ts_side" >&2
  echo "Regenerate with UPDATE_GOLDEN=1 cargo test --test bindings_golden." >&2
  exit 1
fi

echo "fixture pairing OK"
