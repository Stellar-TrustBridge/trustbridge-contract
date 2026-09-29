#!/usr/bin/env bash
# Storage key inventory linter (Issue #330).
#
# Every `const NAME: Symbol = ...` in src/storage.rs must be listed in
# docs/STORAGE_KEYS.md by its constant name (in backticks) and, for
# `symbol_short!` keys, by its symbol string. Fails closed: any Symbol
# constant the parser cannot classify is an error, not a skip.
set -euo pipefail

SRC="${1:-src/storage.rs}"
DOC="${2:-docs/STORAGE_KEYS.md}"
fail=0
count=0

while IFS= read -r line; do
  name=$(sed -E 's/.*const ([A-Z0-9_]+): *Symbol.*/\1/' <<<"$line")
  count=$((count + 1))
  if [[ "$line" =~ symbol_short!\(\"([A-Za-z0-9_]+)\"\) ]]; then
    sym="${BASH_REMATCH[1]}"
    if ! grep -qF "\`\"$sym\"\`" "$DOC"; then
      echo "MISSING: symbol \"$sym\" ($name) not documented in $DOC"; fail=1
    fi
  elif [[ "$line" =~ =\ *([A-Z0-9_]+)\; ]]; then
    : # alias of another constant; the target is checked on its own line
  else
    echo "UNKNOWN: cannot classify Symbol constant '$name': $line"; fail=1
  fi
  if ! grep -qF "\`$name\`" "$DOC"; then
    echo "MISSING: constant $name not documented in $DOC"; fail=1
  fi
done < <(grep -E '^\s*(pub(\([a-z]+\))? )?const [A-Z0-9_]+: *Symbol' "$SRC")

# Symbol::new keys cannot be inventoried statically; reject them.
if grep -nE 'Symbol::new\(' "$SRC"; then
  echo "UNKNOWN: Symbol::new in $SRC — use a documented const key"; fail=1
fi

if [[ $count -eq 0 ]]; then
  echo "ERROR: no Symbol constants found in $SRC (parser broken?)"; exit 1
fi

# Issue #468: CHUNK_CNT_KEY must have exactly one definition, persisted as chkcnt.
chunk_cnt_defs=$(grep -cE '^\s*(pub(\([a-z]+\))? )?const CHUNK_CNT_KEY:' "$SRC" || true)
if [[ "$chunk_cnt_defs" -ne 1 ]]; then
  echo "ERROR: expected exactly one CHUNK_CNT_KEY definition in $SRC, found $chunk_cnt_defs"; fail=1
fi
if grep -nE 'const [A-Z0-9_]+: *Symbol = symbol_short!\("chunkcnt"\)' "$SRC"; then
  echo "ERROR: obsolete chunkcnt Symbol constant in $SRC — persisted key is chkcnt (CHUNK_CNT_KEY)"; fail=1
fi
if ! grep -qE 'const CHUNK_CNT_KEY: Symbol = symbol_short!\("chkcnt"\)' "$SRC"; then
  echo "ERROR: CHUNK_CNT_KEY must be symbol_short!(\"chkcnt\")"; fail=1
fi
if grep -qE '^\| `"chunkcnt"` \|' "$DOC"; then
  echo "ERROR: $DOC still documents chunkcnt as a key row; canonical symbol is chkcnt"; fail=1
fi

if [[ $fail -ne 0 ]]; then
  echo "Storage key inventory check FAILED — update $DOC"; exit 1
fi
echo "Storage key inventory OK ($count constants documented)"
