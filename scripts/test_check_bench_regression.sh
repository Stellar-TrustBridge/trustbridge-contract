#!/usr/bin/env bash
# test_check_bench_regression.sh — self-test for check_bench_regression.sh
# (Issue #676). Verifies the regression gate passes when measurements match
# ci/bench-samples.csv, fails on a regression, and fails on a missing baseline.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CHECK="$ROOT/scripts/check_bench_regression.sh"
SAMPLES="$ROOT/ci/bench-samples.csv"
fail=0

expect() {
    local want="$1" name="$2"; shift 2
    "$@" >/dev/null 2>&1
    local got=$?
    if [ "$got" -eq "$want" ]; then
        echo "ok   $name"
    else
        echo "FAIL $name (exit $got, want $want)"; fail=1
    fi
}

baseline() { grep -v '^#' "$SAMPLES"; }
inflated() { baseline | awk -F',' 'NR==1 { print; next } { $3 = $3 * 2; print }' OFS=','; }

expect 0 "matching measurements pass" \
    bash -c "$(declare -f baseline); SAMPLES='$SAMPLES'; baseline | bash '$CHECK' --samples '$SAMPLES'"
expect 1 "doubled CPU cost fails as a regression" \
    bash -c "$(declare -f baseline inflated); SAMPLES='$SAMPLES'; inflated | bash '$CHECK' --samples '$SAMPLES'"
expect 2 "missing samples file is a setup error" \
    bash -c "echo | bash '$CHECK' --samples /nonexistent.csv"

exit "$fail"
