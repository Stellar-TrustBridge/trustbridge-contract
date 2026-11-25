#!/usr/bin/env bash
# Error-code consistency check (Issue #402).
set -euo pipefail
#
# Four places describe the ContractError discriminants, and before this check
# they had drifted apart:
#
#   1. the `ContractError` enum in `src/error.rs`   — what the contract returns
#   2. its `from_code` match arms                   — what off-chain decoders get
#   3. `abi/contract_error_codes.golden`            — the frozen ABI surface
#   4. the table in `docs/ABI.md`                   — what integrators read
#
# The doc table in `src/error.rs` had been four codes out of step with the enum
# from code 17 onward, and `docs/ABI.md` listed `NetworkMismatch` at code 21
# while the enum had `InvalidPauseReason` there. A client built from the docs
# would have decoded a bad pause-reason as a network mismatch.
#
# This script fails if any of the four disagree, or if a discriminant is used
# twice. Run it from CI and from `make check`.
#
# Usage:  ./scripts/check_error_codes.sh

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ERROR_RS="src/error.rs"
GOLDEN="abi/contract_error_codes.golden"
ABI_MD="docs/ABI.md"

fail() {
  printf 'error-codes: %s\n' "$1" >&2
  FAILED=1
}

FAILED=0
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# --- 1. The enum itself: `    VariantName = 12,` inside `pub enum ContractError`.
awk '
  /pub enum ContractError \{/ { inside = 1; next }
  inside && /^\}/             { inside = 0 }
  inside && /^[[:space:]]+[A-Za-z][A-Za-z0-9]* = [0-9]+,$/ {
    gsub(/[[:space:],]/, "")
    split($0, parts, "=")
    print parts[2] " " parts[1]
  }
' "$ERROR_RS" | sort -n > "$TMP/enum.txt"

if [[ ! -s "$TMP/enum.txt" ]]; then
  fail "could not parse any variants out of $ERROR_RS — has the enum moved?"
  exit 1
fi

# --- 2. Duplicate discriminants. The collision this whole script exists for.
DUPES="$(cut -d' ' -f1 "$TMP/enum.txt" | sort -n | uniq -d || true)"
if [[ -n "$DUPES" ]]; then
  fail "duplicate discriminants in $ERROR_RS: $(tr '\n' ' ' <<<"$DUPES")"
fi

# Duplicate *names* on different codes are just as bad — an off-chain decoder
# keyed by name would silently pick one.
NAME_DUPES="$(cut -d' ' -f2 "$TMP/enum.txt" | sort | uniq -d || true)"
if [[ -n "$NAME_DUPES" ]]; then
  fail "duplicate variant names in $ERROR_RS: $(tr '\n' ' ' <<<"$NAME_DUPES")"
fi

# --- 3. `from_code` must cover exactly the enum, no more and no less.
awk '
  /pub fn from_code/ { inside = 1; next }
  inside && /^[[:space:]]+\}$/ { inside = 0 }
  inside && /=> Some\(ContractError::/ {
    match($0, /^[[:space:]]*[0-9]+/)
    code = substr($0, RSTART, RLENGTH); gsub(/[[:space:]]/, "", code)
    match($0, /ContractError::[A-Za-z0-9]+/)
    name = substr($0, RSTART + 15, RLENGTH - 15)
    print code " " name
  }
' "$ERROR_RS" | sort -n > "$TMP/from_code.txt"

if ! diff -u "$TMP/enum.txt" "$TMP/from_code.txt" > "$TMP/from_code.diff"; then
  fail "the ContractError enum and from_code disagree (- enum, + from_code):"
  sed 's/^/  /' "$TMP/from_code.diff" >&2
fi

# --- 4. The golden file is the frozen ABI surface.
grep -E '^[0-9]+ [A-Za-z][A-Za-z0-9]*$' "$GOLDEN" | sort -n > "$TMP/golden.txt"

if ! diff -u "$TMP/enum.txt" "$TMP/golden.txt" > "$TMP/golden.diff"; then
  fail "$GOLDEN does not match the enum (- enum, + golden):"
  sed 's/^/  /' "$TMP/golden.diff" >&2
  printf 'error-codes: if you ADDED a variant, append it to %s. Never renumber.\n' "$GOLDEN" >&2
fi

# --- 5. The integrator-facing table in docs/ABI.md.
# Rows look like: | 30 | `NetworkMismatch` | description |
awk '
  /^### ContractError \(u32 discriminant\)/ { inside = 1; next }
  inside && /^#/                            { inside = 0 }
  inside && /^\| *[0-9]+ *\| *`[A-Za-z0-9]+` *\|/ {
    split($0, cells, "|")
    code = cells[2]; name = cells[3]
    gsub(/[[:space:]]/, "", code)
    gsub(/[[:space:]`]/, "", name)
    print code " " name
  }
' "$ABI_MD" | sort -n > "$TMP/abi.txt"

if ! diff -u "$TMP/enum.txt" "$TMP/abi.txt" > "$TMP/abi.diff"; then
  fail "the ContractError table in $ABI_MD does not match the enum (- enum, + docs):"
  sed 's/^/  /' "$TMP/abi.diff" >&2
fi

# --- 6. The doc-comment table on the enum, which is what drifted originally.
awk '
  /pub enum ContractError \{/ { exit }
  /^\/\/\/ \| *[0-9]+ *\| *`[A-Za-z0-9]+` *\|/ {
    sub(/^\/\/\/ /, "")
    split($0, cells, "|")
    code = cells[2]; name = cells[3]
    gsub(/[[:space:]]/, "", code)
    gsub(/[[:space:]`]/, "", name)
    print code " " name
  }
' "$ERROR_RS" | sort -n > "$TMP/doccomment.txt"

if ! diff -u "$TMP/enum.txt" "$TMP/doccomment.txt" > "$TMP/doc.diff"; then
  fail "the doc-comment table on ContractError does not match the enum (- enum, + doc):"
  sed 's/^/  /' "$TMP/doc.diff" >&2
fi

if (( FAILED )); then
  printf '\nerror-codes: FAILED — see above. Each discriminant must mean exactly one thing\n' >&2
  printf 'error-codes: in src/error.rs, from_code, %s, and %s.\n' "$GOLDEN" "$ABI_MD" >&2
  exit 1
fi

printf 'error-codes: OK — %s discriminants agree across enum, from_code, %s, and %s\n' \
  "$(wc -l < "$TMP/enum.txt" | tr -d ' ')" "$GOLDEN" "$ABI_MD"
