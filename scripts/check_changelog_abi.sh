#!/usr/bin/env bash
set -euo pipefail

base_revision="${1:-}"

if [[ -z "$base_revision" || "$base_revision" =~ ^0+$ ]]; then
  base_revision="HEAD^"
fi

if ! git rev-parse --verify "$base_revision^{commit}" >/dev/null 2>&1; then
  echo "ERROR: Cannot resolve ABI comparison base: $base_revision"
  exit 1
fi

abi_diff=$(git diff --unified=0 "$base_revision...HEAD" -- docs/ABI.md)
if [[ -z "$abi_diff" ]]; then
  echo "OK: ABI.md has no changes; changelog entry not required."
  exit 0
fi

signature_diff=$(printf '%s\n' "$abi_diff" | grep -E '^[+-][^+-].*###[[:space:]]+`[^`]+`[^`]*->[[:space:]]*' || true)
if [[ -z "$signature_diff" ]]; then
  echo "OK: ABI.md changes contain no public function signature changes."
  exit 0
fi

if [[ ! -f CHANGELOG.md ]]; then
  echo "ERROR: Public ABI signatures changed, but CHANGELOG.md is missing."
  exit 1
fi

changelog_diff=$(git diff --unified=0 "$base_revision...HEAD" -- CHANGELOG.md)
if printf '%s\n' "$changelog_diff" | grep -Eq '^\+[^+].*<!--[[:space:]]*changelog-check:[[:space:]]*skip[[:space:]]*-[[:space:]]+.+-->[[:space:]]*$'; then
  echo "OK: ABI signature change has an explicitly documented changelog-check skip."
  exit 0
fi

if ! printf '%s\n' "$changelog_diff" | grep -Eq '^\+[^+]*##[[:space:]]+\[?[0-9]+\.[0-9]+\.[0-9]+\]?'; then
  echo "ERROR: Public ABI signatures changed without a new versioned CHANGELOG.md entry."
  echo "Add a heading such as '## [1.2.0] - YYYY-MM-DD' and describe the ABI change."
  echo "For an intentional false positive, add '<!-- changelog-check: skip - reason -->' to the changelog diff."
  exit 1
fi

echo "OK: ABI signature change is documented by a versioned CHANGELOG.md entry."
