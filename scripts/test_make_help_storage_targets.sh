#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

command -v make >/dev/null 2>&1 || {
  echo "FAIL: make is required to run 'make help'" >&2
  exit 1
}

help_output="$(make help)"
for target in \
  storage-footprint-check \
  storage-rent-check \
  storage-rent-estimator \
  storage-rent-estimator-golden
do
  if ! grep -qF -- "$target" <<<"$help_output"; then
    echo "FAIL: 'make help' does not document $target" >&2
    exit 1
  fi
done

printf '%s\n' "$help_output" | grep -E 'storage-(footprint|rent)' >/dev/null
if ! printf '%s\n' "$help_output" | grep -q 'storage-footprint-check'; then
  echo "FAIL: storage footprint help entry is missing" >&2
  exit 1
fi

echo "PASS: make help documents storage footprint/rent targets"
