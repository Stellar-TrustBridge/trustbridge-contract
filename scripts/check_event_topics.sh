#!/usr/bin/env bash
# Event-topic table consistency check (Issue #399).
#
# scripts/event_indexer.sh classifies events from a table of topic symbols.
# `#[contractevent]` derives the first topic as the struct name in snake_case,
# so every `pub struct FooEvent` in src/events.rs emits topic `foo_event`.
#
# If the two fall out of step the indexer silently labels a real event
# `unknown` — which is the correct runtime behaviour (never drop a stream you
# don't recognise) but a bad thing to discover from a dashboard gap weeks
# later. This check turns it into a CI failure instead.
#
# It also runs the indexer against the canned sample twice, asserting the
# second pass appends nothing — the resume-idempotency guarantee the indexer's
# header promises.
#
# Coverage beyond the indexer table — the docs/EVENT_INDEXING.md topic table
# and its Subgraph column (mapped entity or waiver in docs/subgraph/README.md) —
# is checked by scripts/check_subgraph_parity.py (`make subgraph-parity`,
# Issue #754). Run both when adding or renaming an event.
#
# Usage:  ./scripts/check_event_topics.sh

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

EVENTS_RS="src/events.rs"
INDEXER="scripts/event_indexer.sh"
SAMPLE="scripts/testdata/getEvents.sample.json"

FAILED=0
fail() { printf 'event-topics: %s\n' "$1" >&2; FAILED=1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# --- 1. Topic symbols the contract actually emits.
# `PascalCaseEvent` → `pascal_case_event`, matching the SDK's derivation.
# Keep the struct name and source line alongside the derived topic so a
# collision (step below) can be reported with enough detail to go fix it
# without re-deriving it by hand (Issue #794).
grep -noE '^pub struct [A-Za-z0-9]+Event' "$EVENTS_RS" \
  | sed 's/:pub struct /:/' \
  | awk -F: '{
      line = $1
      name = $2
      out = ""
      for (i = 1; i <= length(name); i++) {
        c = substr(name, i, 1)
        if (c ~ /[A-Z]/) {
          if (i > 1) out = out "_"
          out = out tolower(c)
        } else {
          out = out c
        }
      }
      print out "|" name "|" line
    }' \
  > "$TMP/from_rust_raw.txt"

cut -d'|' -f1 "$TMP/from_rust_raw.txt" > "$TMP/from_rust_topics_only.txt"

# Duplicate derived topics would collide on the event stream (Issue #643).
# `#[contractevent]` derives the topic from the struct name, so two event
# structs whose snake_case names match emit the same topic and the indexer
# would silently label one `unknown`.
DUPE_TOPICS="$(sort "$TMP/from_rust_topics_only.txt" | uniq -d)"
if [[ -n "$DUPE_TOPICS" ]]; then
  fail "duplicate event topics derived from $EVENTS_RS (two structs map to the same topic):"
  while IFS= read -r topic; do
    [[ -z "$topic" ]] && continue
    printf '  topic %-30s collides between:\n' "\"$topic\"" >&2
    # Print every colliding struct with its exact file:line so the fix is a
    # one-line jump, not another grep.
    awk -F'|' -v t="$topic" '$1 == t { printf "    - %s (%s:%s)\n", $2, "'"$EVENTS_RS"'", $3 }' \
      "$TMP/from_rust_raw.txt" >&2
  done <<<"$DUPE_TOPICS"
  printf 'event-topics: rename one of the colliding structs so their snake_case names differ.\n' >&2
fi

sort -u "$TMP/from_rust_topics_only.txt" > "$TMP/from_rust.txt"

if [[ ! -s "$TMP/from_rust.txt" ]]; then
  fail "no event structs found in $EVENTS_RS — has the file moved?"
  exit 1
fi

# --- 2. Symbols the indexer's table knows about.
awk '
  /^TOPIC_TABLE="\$\(cat <<.TOPICS./ { inside = 1; next }
  inside && /^TOPICS$/              { inside = 0 }
  inside && /^[a-z0-9_]+\|/         { split($0, p, "|"); print p[1] }
' "$INDEXER" | sort -u > "$TMP/from_table.txt"

if [[ ! -s "$TMP/from_table.txt" ]]; then
  fail "could not parse TOPIC_TABLE out of $INDEXER"
  exit 1
fi

# --- 3. Compare.
MISSING="$(comm -23 "$TMP/from_rust.txt" "$TMP/from_table.txt")"
if [[ -n "$MISSING" ]]; then
  fail "these events are emitted by $EVENTS_RS but absent from TOPIC_TABLE in $INDEXER:"
  sed 's/^/  /' <<<"$MISSING" >&2
  printf 'event-topics: add a `<symbol>|<event_kind>|<category>` row for each.\n' >&2
fi

STALE="$(comm -13 "$TMP/from_rust.txt" "$TMP/from_table.txt")"
if [[ -n "$STALE" ]]; then
  fail "TOPIC_TABLE in $INDEXER lists symbols no longer emitted by $EVENTS_RS:"
  sed 's/^/  /' <<<"$STALE" >&2
fi

# --- 4. Duplicate rows would make classification order-dependent.
DUPES="$(awk '
  /^TOPIC_TABLE="\$\(cat <<.TOPICS./ { inside = 1; next }
  inside && /^TOPICS$/              { inside = 0 }
  inside && /^[a-z0-9_]+\|/         { split($0, p, "|"); print p[1] }
' "$INDEXER" | sort | uniq -d)"
if [[ -n "$DUPES" ]]; then
  fail "duplicate TOPIC_TABLE rows: $(tr '\n' ' ' <<<"$DUPES")"
fi

# --- 5. The sample must exercise the classifier, not placeholder topics.
if ! command -v jq >/dev/null 2>&1; then
  printf 'event-topics: jq not found — skipping the indexer replay check\n' >&2
elif [[ ! -f "$SAMPLE" ]]; then
  fail "$SAMPLE is missing"
else
  DATA_DIR="$TMP/indexer"
  export DATA_DIR

  if ! MOCK_RESPONSE="$SAMPLE" CONTRACT_ID=C_MOCK ONESHOT=1 \
       "$INDEXER" > "$TMP/run1.log" 2>&1; then
    fail "the indexer failed on $SAMPLE:"
    sed 's/^/  /' "$TMP/run1.log" >&2
  else
    JSONL="$DATA_DIR/events-testnet.jsonl"
    FIRST_COUNT="$(wc -l < "$JSONL" | tr -d ' ')"

    if (( FIRST_COUNT == 0 )); then
      fail "$SAMPLE produced no events"
    fi

    # Every sample event but the deliberate unknown one must classify.
    UNCLASSIFIED="$(jq -r 'select(.event_kind == "unknown") | .topic_symbol // "<undecodable>"' "$JSONL")"
    UNCLASSIFIED_COUNT="$(grep -c . <<<"$UNCLASSIFIED" || true)"
    if (( UNCLASSIFIED_COUNT != 1 )); then
      fail "expected exactly one deliberately-unknown sample event, got $UNCLASSIFIED_COUNT:"
      sed 's/^/  /' <<<"$UNCLASSIFIED" >&2
    fi

    # And no event may have an undecodable topic — that would mean the sample
    # still carries placeholder base64 rather than real ScVal symbols.
    UNDECODABLE="$(jq -r 'select(.topic_symbol == null) | .id' "$JSONL")"
    if [[ -n "$UNDECODABLE" ]]; then
      fail "these sample events have undecodable topics (placeholder base64?):"
      sed 's/^/  /' <<<"$UNDECODABLE" >&2
    fi

    # --- 6. Resume idempotency: a second pass must append nothing.
    if ! MOCK_RESPONSE="$SAMPLE" CONTRACT_ID=C_MOCK ONESHOT=1 \
         "$INDEXER" > "$TMP/run2.log" 2>&1; then
      fail "the indexer failed on its second (resume) pass:"
      sed 's/^/  /' "$TMP/run2.log" >&2
    else
      SECOND_COUNT="$(wc -l < "$JSONL" | tr -d ' ')"
      if (( SECOND_COUNT != FIRST_COUNT )); then
        fail "resume is not idempotent: $FIRST_COUNT events became $SECOND_COUNT on re-run"
      fi

      UNIQUE="$(jq -r .id "$JSONL" | sort -u | wc -l | tr -d ' ')"
      if (( UNIQUE != SECOND_COUNT )); then
        fail "duplicate event ids in the log: $SECOND_COUNT lines, $UNIQUE unique ids"
      fi
    fi
  fi
fi

if (( FAILED )); then
  printf '\nevent-topics: FAILED — see above.\n' >&2
  exit 1
fi

printf 'event-topics: OK — %s event symbols classified, replay is idempotent\n' \
  "$(wc -l < "$TMP/from_rust.txt" | tr -d ' ')"
