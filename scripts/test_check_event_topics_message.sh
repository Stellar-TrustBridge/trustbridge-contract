#!/usr/bin/env bash
# Unit test for the duplicate-topic failure message format produced by
# scripts/check_event_topics.sh (Issue #794).
#
# scripts/check_event_topics.sh is the source of truth for the check itself;
# this test only pins down the *message format* for a collision — that it
# names both colliding structs and their exact `file:line` locations — so a
# future edit to the script can't silently regress back to the old
# "duplicate event topics: foo_event" message with no location info.
#
# Usage: bash scripts/test_check_event_topics_message.sh

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

FAILED=0
fail() { printf 'test_check_event_topics_message: FAIL: %s\n' "$1" >&2; FAILED=1; }

# Build a minimal fixture repo layout so the script's relative paths resolve,
# with two structs whose derived snake_case names coincide: the leading-char
# case is dropped from the derivation (no underscore at position 1 whether
# upper- or lower-case), so "FooEvent" and "fooEvent" both derive to
# "foo_event" — a real collision shape, not a contrived one.
FIXTURE="$TMP/fixture"
mkdir -p "$FIXTURE/scripts" "$FIXTURE/src" "$FIXTURE/scripts/testdata"
cp "$ROOT/scripts/check_event_topics.sh" "$FIXTURE/scripts/check_event_topics.sh"
cp "$ROOT/scripts/event_indexer.sh" "$FIXTURE/scripts/event_indexer.sh"
cp "$ROOT/scripts/testdata/getEvents.sample.json" "$FIXTURE/scripts/testdata/getEvents.sample.json"

cat > "$FIXTURE/src/events.rs" <<'EOF'
pub struct FooEvent {
    pub x: u32,
}

pub struct fooEvent {
    pub y: u32,
}
EOF

OUT="$TMP/out.log"
cd "$FIXTURE"
if bash scripts/check_event_topics.sh > "$OUT" 2>&1; then
  fail "expected the script to fail on a deliberate topic collision, it exited 0"
fi

if ! grep -q 'topic "foo_event"' "$OUT"; then
  fail "message must name the colliding topic (\"foo_event\"); got:"
  sed 's/^/  /' "$OUT" >&2
fi

if ! grep -q 'FooEvent (src/events.rs:' "$OUT"; then
  fail "message must name struct FooEvent with its file:line location"
fi

if ! grep -q 'fooEvent (src/events.rs:' "$OUT"; then
  fail "message must name struct fooEvent with its file:line location"
fi

if ! grep -q 'rename one of the colliding structs' "$OUT"; then
  fail "message must suggest the fix (renaming one of the colliding structs)"
fi

if (( FAILED )); then
  echo "test_check_event_topics_message: FAILED — see above. Full output:" >&2
  sed 's/^/  /' "$OUT" >&2
  exit 1
fi

echo "test_check_event_topics_message: OK — collision message names both structs and their locations"
