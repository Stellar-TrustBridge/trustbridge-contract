#!/usr/bin/env bash
# Offline regression test for scripts/futurenet_smoke_test.sh.
#
# Exercises:
#   1. Dry-run happy path (`DRY_RUN=true`) prints `get_health_probes` after init.
#   2. Mocked live happy path deploys, initializes, and invokes `get_health_probes`,
#      `get_stats`, and `has_record` in order.
#   3. Failure path 1: missing `ADMIN` when `DRY_RUN=false` fails non-zero.
#   4. Failure path 2: failing `get_health_probes` invocation aborts the smoke script.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TMP_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

# 1. Dry-run happy path
dry_out="$(DRY_RUN=true ./scripts/futurenet_smoke_test.sh)"
if ! grep -q "get_health_probes" <<<"$dry_out"; then
  echo "FAIL: dry-run output must include get_health_probes" >&2
  exit 1
fi

# 2. Failure path 1: missing ADMIN when DRY_RUN=false
if err_out="$(ADMIN="" DRY_RUN=false ./scripts/futurenet_smoke_test.sh 2>&1)"; then
  echo "FAIL: missing ADMIN should have failed non-zero" >&2
  exit 1
fi
if ! grep -q "ADMIN must be set" <<<"$err_out"; then
  echo "FAIL: unexpected error for missing ADMIN: $err_out" >&2
  exit 1
fi

# Build a temporary workspace copy so deploy.sh writes deployments/futurenet.json
# without dirtying the repo working tree.
WORK="$TMP_DIR/work"
mkdir -p "$WORK/scripts" "$WORK/target/wasm32v1-none/release"
cp scripts/deploy.sh scripts/futurenet_smoke_test.sh "$WORK/scripts/"
chmod +x "$WORK/scripts/deploy.sh" "$WORK/scripts/futurenet_smoke_test.sh"
touch "$WORK/target/wasm32v1-none/release/trustbridge-contract.wasm"

MOCK_LOG="$TMP_DIR/stellar-calls.log"
MOCK_STELLAR="$TMP_DIR/mock-stellar.sh"
cat > "$MOCK_STELLAR" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo "$*" >> "${MOCK_LOG:?}"
if [[ "$*" == *"contract deploy"* ]]; then
  echo "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2M2"
  exit 0
fi
if [[ -n "${FAIL_ON:-}" && "$*" == *"${FAIL_ON}"* ]]; then
  echo "Error(Contract, #2)" >&2
  exit 1
fi
if [[ "$*" == *"get_health_probes"* ]]; then
  echo '{"paused":false,"version":[1,1,0],"total":0,"verified":0,"cooldown_secs":0,"cooldown_remaining_secs":0,"attestation_present":false}'
elif [[ "$*" == *"get_stats"* ]]; then
  echo '{"total":0,"verified":0,"ever_verified":0}'
elif [[ "$*" == *"has_record"* ]]; then
  echo 'false'
fi
EOF
chmod +x "$MOCK_STELLAR"

# 3. Mocked live happy path
: > "$MOCK_LOG"
live_out="$(
  cd "$WORK" && \
  MOCK_LOG="$MOCK_LOG" ADMIN="GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF" \
  SOURCE="smoke-test" STELLAR="$MOCK_STELLAR" ./scripts/futurenet_smoke_test.sh
)"
if ! grep -q "Futurenet smoke checks passed" <<<"$live_out"; then
  echo "FAIL: expected smoke checks pass banner" >&2
  exit 1
fi
init_line="$(grep -n -- "-- initialize" "$MOCK_LOG" | cut -d: -f1 | head -n1)"
probe_line="$(grep -n -- "-- get_health_probes" "$MOCK_LOG" | cut -d: -f1 | head -n1)"
if [[ -z "$init_line" || -z "$probe_line" || "$probe_line" -le "$init_line" ]]; then
  echo "FAIL: get_health_probes must run after initialize (log: $(cat "$MOCK_LOG"))" >&2
  exit 1
fi

# 4. Failure path 2: failing get_health_probes aborts smoke
: > "$MOCK_LOG"
if fail_out="$(
  cd "$WORK" && \
  MOCK_LOG="$MOCK_LOG" FAIL_ON="get_health_probes" \
  ADMIN="GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF" \
  SOURCE="smoke-test" STELLAR="$MOCK_STELLAR" ./scripts/futurenet_smoke_test.sh 2>&1
)"; then
  echo "FAIL: failing get_health_probes must abort futurenet_smoke_test.sh" >&2
  exit 1
fi
if grep -q "Futurenet smoke checks passed" <<<"$fail_out"; then
  echo "FAIL: smoke script printed pass banner despite probe failure" >&2
  exit 1
fi

echo "PASS: futurenet_smoke_test.sh dry-run, live health probe ordering, and failure paths"
