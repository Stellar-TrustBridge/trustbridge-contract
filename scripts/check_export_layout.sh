#!/usr/bin/env bash
# Snapshot check for the v1 registry export page layout (Issue #808).
#
# Verifies the JSON field order, types, and integer widths produced by
# scripts/export_registry.sh. This is a read-only compatibility check for
# dashboard/offline-attestor parsers: it never rewrites the file. The XDR
# byte-level layout it corresponds to is documented in
# docs/ABI.md#export-page-byte-layout-v1.
#
# Usage:
#   ./scripts/check_export_layout.sh registry-export-testnet.json
#   cat registry-export-testnet.json | ./scripts/check_export_layout.sh
#
# Exit status: 0 layout matches, 1 layout mismatch, 2 usage/tool error.

set -euo pipefail

if ! command -v jq >/dev/null 2>&1; then
  echo "ERROR: jq is required to check the export layout." >&2
  exit 2
fi

INPUT="${1:-/dev/stdin}"
if [[ ! -r "$INPUT" ]]; then
  echo "ERROR: cannot read export file: $INPUT" >&2
  echo "Usage: $0 <export.json>" >&2
  exit 2
fi

if ! jq empty "$INPUT" >/dev/null 2>&1; then
  echo "ERROR: $INPUT is not valid JSON." >&2
  exit 2
fi

errors=0
fail() {
  echo "LAYOUT_MISMATCH: $1" >&2
  errors=$((errors + 1))
}

if ! jq -e 'keys_unsorted == ["schema_version","contract_id","network","exported_at","count","records"]' \
    "$INPUT" >/dev/null; then
  fail "top-level fields must be ordered schema_version, contract_id, network, exported_at, count, records"
fi

if ! jq -e '
    (.schema_version | type == "number") and
    (.contract_id | type == "string") and
    (.network | type == "string") and
    (.exported_at | type == "string") and
    (.count | type == "number") and
    (.records | type == "array")
  ' "$INPUT" >/dev/null; then
  fail "top-level field types changed (expected number/string/string/string/number/array)"
fi

if ! jq -e '.schema_version == 1 and (.schema_version == floor)' "$INPUT" >/dev/null; then
  fail "schema_version must be the integer 1"
fi

if ! jq -e '.count == (.records | length)' "$INPUT" >/dev/null; then
  fail "count must equal the number of records"
fi

if ! jq -e 'all(.records[]; keys_unsorted == ["github_username","stellar_address","verified","registered_at"])' \
    "$INPUT" >/dev/null; then
  fail "record fields must be ordered github_username, stellar_address, verified, registered_at"
fi

if ! jq -e '
    all(.records[];
      (.github_username | type == "string") and
      (.stellar_address | type == "string") and
      (.verified | type == "boolean") and
      (.registered_at | type == "number") and
      (.registered_at == floor) and
      (.registered_at >= 0) and
      (.registered_at <= 4294967295)
    )
  ' "$INPUT" >/dev/null; then
  fail "record field types changed or registered_at no longer fits an unsigned 32-bit value"
fi

if [[ "$errors" -gt 0 ]]; then
  echo "FAIL: export page layout differs from the v1 snapshot (${errors} mismatch(es))." >&2
  echo "Expected layout: docs/ABI.md#export-page-byte-layout-v1" >&2
  exit 1
fi

echo "OK: export page layout matches the v1 snapshot."
