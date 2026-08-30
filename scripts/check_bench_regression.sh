#!/usr/bin/env bash
# check_bench_regression.sh — compare bench output against checked-in baselines.
#
# USAGE
#   scripts/check_bench_regression.sh [--samples FILE] [--threshold PCT] < bench-output.txt
#
#   Or pipe directly from cargo test:
#   cargo test bench ... --nocapture --test-threads=1 2>/dev/null | \
#       scripts/check_bench_regression.sh
#
# OPTIONS
#   --samples FILE      Path to baseline CSV (default: ci/bench-samples.csv)
#   --threshold PCT     Regression % that triggers a failure (default: 15)
#   --hard-cpu-cap N    Hard absolute CPU cap for register/verify (default: 25000000)
#   --hard-mem-cap N    Hard absolute memory cap for register/verify (default: 3000000)
#
# EXIT CODES
#   0  All operations are within threshold and hard caps.
#   1  One or more regressions or cap breaches detected.
#   2  Usage / setup error (missing baseline file, no bench lines parsed).
#
# OUTPUT FORMAT EXPECTED FROM CARGO TEST (--nocapture)
#   The script recognises CSV lines emitted by the test_bench_* and
#   test_report_register_budget_samples tests:
#
#     operation,input_label,cpu_instructions,memory_bytes
#     register,baseline,1050000,95000
#     usernames_match,10,320000,28000
#     get_all_registered,10,1800000,190000
#     verify,success,1350000,120000
#     verify,rejected_double_verify,900000,85000
#
#   Header lines (starting with "operation,") and comment lines (starting with
#   "#") are ignored.  Non-CSV lines from cargo/rustc noise are ignored.
#
# NOTES
#   * Run with --test-threads=1 for stable measurements.
#   * Baseline CPU/mem values in ci/bench-samples.csv reflect the Soroban SDK
#     test-host metering model at the time they were recorded; a soroban-sdk
#     version bump may shift all values — update baselines with
#     `make bench-update-samples` and commit the result.

set -euo pipefail

# ── defaults ────────────────────────────────────────────────────────────────
SAMPLES_FILE="ci/bench-samples.csv"
THRESHOLD_PCT=15
HARD_CPU_CAP=25000000
HARD_MEM_CAP=3000000

# ── argument parsing ─────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --samples)    SAMPLES_FILE="$2"; shift 2 ;;
        --threshold)  THRESHOLD_PCT="$2"; shift 2 ;;
        --hard-cpu-cap) HARD_CPU_CAP="$2"; shift 2 ;;
        --hard-mem-cap) HARD_MEM_CAP="$2"; shift 2 ;;
        -h|--help)
            sed -n '/^# /,/^[^#]/{ /^# /s/^# //p }' "$0"
            exit 0
            ;;
        *)
            echo "ERROR: unknown option: $1" >&2
            exit 2
            ;;
    esac
done

# ── validate inputs ───────────────────────────────────────────────────────────
if [[ ! -f "$SAMPLES_FILE" ]]; then
    echo "ERROR: baseline samples file not found: $SAMPLES_FILE" >&2
    echo "Run 'make bench-update-samples' to generate it, or pass --samples <path>." >&2
    exit 2
fi

# ── load baselines into associative arrays ────────────────────────────────────
# Key format: "operation:input_label"
declare -A BASELINE_CPU
declare -A BASELINE_MEM

while IFS=',' read -r op label cpu mem; do
    # Skip comment lines and the CSV header
    [[ "$op" =~ ^[[:space:]]*# ]] && continue
    [[ "$op" == "operation" ]]    && continue
    [[ -z "$op" ]]                && continue

    # Strip any trailing whitespace/carriage returns
    op="${op%%[[:space:]]}"
    label="${label%%[[:space:]]}"
    cpu="${cpu//[[:space:]]/}"
    mem="${mem//[[:space:]]/}"

    key="${op}:${label}"
    BASELINE_CPU["$key"]="$cpu"
    BASELINE_MEM["$key"]="$mem"
done < "$SAMPLES_FILE"

if [[ ${#BASELINE_CPU[@]} -eq 0 ]]; then
    echo "ERROR: no baselines loaded from $SAMPLES_FILE" >&2
    exit 2
fi

echo "Loaded ${#BASELINE_CPU[@]} baseline entries from $SAMPLES_FILE"
echo "Regression threshold : ${THRESHOLD_PCT}%"
echo "Hard CPU cap         : ${HARD_CPU_CAP}"
echo "Hard memory cap      : ${HARD_MEM_CAP}"
echo ""

# ── parse bench output from stdin ─────────────────────────────────────────────
# Accumulate all measured lines; we only report after reading everything so the
# summary is compact.
BENCH_INPUT=$(cat)

declare -A MEASURED_CPU
declare -A MEASURED_MEM

while IFS=',' read -r op label cpu mem; do
    [[ "$op" =~ ^[[:space:]]*# ]] && continue
    [[ "$op" == "operation" ]]    && continue
    [[ -z "$op" ]]                && continue
    # Must look like a data row: label non-empty, cpu and mem numeric
    [[ "$cpu" =~ ^[0-9]+$ ]]     || continue
    [[ "$mem" =~ ^[0-9]+$ ]]     || continue

    op="${op%%[[:space:]]}"
    label="${label%%[[:space:]]}"
    cpu="${cpu//[[:space:]]/}"
    mem="${mem//[[:space:]]/}"

    key="${op}:${label}"
    MEASURED_CPU["$key"]="$cpu"
    MEASURED_MEM["$key"]="$mem"
done <<< "$BENCH_INPUT"

if [[ ${#MEASURED_CPU[@]} -eq 0 ]]; then
    echo "ERROR: no benchmark CSV lines found in stdin." >&2
    echo "Ensure tests run with --nocapture and emit lines like:" >&2
    echo "  operation,input_label,cpu_instructions,memory_bytes" >&2
    exit 2
fi

echo "Parsed ${#MEASURED_CPU[@]} measured entries from bench output."
echo ""

# ── comparison ────────────────────────────────────────────────────────────────
FAILURES=0
WARNINGS=0

# Print a header for the results table
printf "%-45s %15s %15s %10s %15s %15s %10s\n" \
    "operation:label" "baseline_cpu" "measured_cpu" "cpu_chg%" \
    "baseline_mem" "measured_mem" "mem_chg%"
printf '%s\n' "$(printf '─%.0s' {1..130})"

for key in "${!MEASURED_CPU[@]}"; do
    measured_cpu="${MEASURED_CPU[$key]}"
    measured_mem="${MEASURED_MEM[$key]}"

    if [[ -z "${BASELINE_CPU[$key]+x}" ]]; then
        printf "%-45s  (no baseline — skipping regression check)\n" "$key"
        (( WARNINGS++ )) || true
        continue
    fi

    baseline_cpu="${BASELINE_CPU[$key]}"
    baseline_mem="${BASELINE_MEM[$key]}"

    # Compute percentage change (integer arithmetic; truncates toward zero).
    # Change = ((measured - baseline) * 100) / baseline
    cpu_chg=$(( (measured_cpu - baseline_cpu) * 100 / baseline_cpu ))
    mem_chg=$(( (measured_mem - baseline_mem) * 100 / baseline_mem ))

    cpu_status="OK"
    mem_status="OK"
    row_failed=0

    # Regression gate: fail if increase > THRESHOLD_PCT
    if (( cpu_chg > THRESHOLD_PCT )); then
        cpu_status="REGRESS"
        row_failed=1
    fi
    if (( mem_chg > THRESHOLD_PCT )); then
        mem_status="REGRESS"
        row_failed=1
    fi

    # Hard absolute cap for register/verify operations
    op="${key%%:*}"
    if [[ "$op" == "register" || "$op" == "verify" ]]; then
        if (( measured_cpu > HARD_CPU_CAP )); then
            cpu_status="OVER_CAP"
            row_failed=1
        fi
        if (( measured_mem > HARD_MEM_CAP )); then
            mem_status="OVER_CAP"
            row_failed=1
        fi
    fi

    printf "%-45s %15d %15d %+9d%% %15d %15d %+9d%%   cpu:%-10s mem:%s\n" \
        "$key" \
        "$baseline_cpu" "$measured_cpu" "$cpu_chg" \
        "$baseline_mem"  "$measured_mem"  "$mem_chg" \
        "$cpu_status" "$mem_status"

    if (( row_failed )); then
        (( FAILURES++ )) || true
    fi
done

echo ""

# ── summary ───────────────────────────────────────────────────────────────────
if (( WARNINGS > 0 )); then
    echo "WARNING: ${WARNINGS} operation(s) had no baseline entry — add them to ${SAMPLES_FILE}."
fi

if (( FAILURES > 0 )); then
    echo ""
    echo "FAIL: ${FAILURES} benchmark regression(s) or cap breach(es) detected."
    echo ""
    echo "If the cost increase is intentional (new feature, SDK bump, refactor):"
    echo "  1. Run: make bench-update-samples"
    echo "  2. Review the diff in ci/bench-samples.csv."
    echo "  3. Commit with a before/after cost table in the PR description."
    echo ""
    echo "See docs/BENCHMARK_BUDGETS.md for full guidance."
    exit 1
fi

echo "PASS: all ${#MEASURED_CPU[@]} benchmark sample(s) are within the ${THRESHOLD_PCT}% regression threshold."
exit 0
