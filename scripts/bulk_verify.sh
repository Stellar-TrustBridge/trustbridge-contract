#!/usr/bin/env bash
# bulk_verify.sh — Maintainer bulk verify CLI
#
# Reads GitHub usernames (one per line) from a file and marks each one as verified.
# Continues on partial failure and summarises successes/failures.
# Includes pacing (configurable delay between calls) to avoid RPC throttling.
#
# Usage:
#   ./scripts/bulk_verify.sh --file usernames.txt \
#       --contract C... --source admin-identity --network testnet \
#       [--dry-run] [--pace-ms 500] [--audit-log audit.log] [--continue-on-error]
#
# Required env (or flags):
#   CONTRACT_ID  — deployed contract C-address
#   SOURCE       — Stellar CLI identity (must be admin or Verifier role)
#   NETWORK      — testnet | futurenet | mainnet (never defaults to mainnet)
#
# Auth:
#   SOURCE must be initialized as admin or hold Role::Verifier on the contract.
#   See docs/DEPLOYMENT.md for role setup instructions.
#
# Pacing:
#   RPC nodes apply per-IP rate limits. Use --pace-ms (default 500 ms) to insert
#   a sleep between calls. Increase to 1000–2000 ms for large batches (>50 usernames)
#   or when hitting HTTP 429 responses.
#
# Audit log format (one JSON-like line per username):
#   {"timestamp":"<ISO-8601>","username":"<u>","network":"<n>","result":"ok|error|dry-run","detail":"<msg>"}

set -euo pipefail

# ---------- defaults ----------
FILE=""
CONTRACT_ID="${CONTRACT_ID:-}"
SOURCE="${SOURCE:-default}"
NETWORK="${NETWORK:-}"
DRY_RUN=false
CONTINUE_ON_ERROR=false
PACE_MS="${PACE_MS:-500}"
AUDIT_LOG=""
STELLAR="${STELLAR:-stellar}"

usage() {
    grep '^#' "$0" | sed 's/^# \?//' | grep -v '^!'
    exit 1
}

# ---------- arg parse ----------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --file)              FILE="$2";              shift 2 ;;
        --contract)          CONTRACT_ID="$2";       shift 2 ;;
        --source)            SOURCE="$2";            shift 2 ;;
        --network)           NETWORK="$2";           shift 2 ;;
        --dry-run)           DRY_RUN=true;           shift ;;
        --continue-on-error) CONTINUE_ON_ERROR=true; shift ;;
        --pace-ms)           PACE_MS="$2";           shift 2 ;;
        --audit-log)         AUDIT_LOG="$2";         shift 2 ;;
        -h|--help)           usage ;;
        *) echo "Unknown flag: $1"; usage ;;
    esac
done

# ---------- validation ----------
[[ -z "$FILE" ]]        && echo "ERROR: --file is required." >&2 && exit 1
[[ -z "$CONTRACT_ID" ]] && echo "ERROR: --contract (or CONTRACT_ID env) is required." >&2 && exit 1
[[ -z "$NETWORK" ]]     && echo "ERROR: --network is required (testnet | futurenet | mainnet). Never defaults to mainnet." >&2 && exit 1
[[ ! -f "$FILE" ]]      && echo "ERROR: file not found: $FILE" >&2 && exit 1

# ---------- helpers ----------
log_audit() {
    local username="$1" result="$2" detail="$3"
    local ts; ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    local line="{\"timestamp\":\"$ts\",\"username\":\"$username\",\"network\":\"$NETWORK\",\"result\":\"$result\",\"detail\":\"$detail\"}"
    echo "$line"
    [[ -n "$AUDIT_LOG" ]] && echo "$line" >> "$AUDIT_LOG"
}

# ---------- main loop ----------
TOTAL=0; SUCCESS=0; FAILED=0; SKIPPED=0
PACE_S=$(echo "scale=3; $PACE_MS/1000" | bc 2>/dev/null || echo "0.5")

echo "=== bulk_verify.sh ==="
echo "  File:       $FILE"
echo "  Contract:   $CONTRACT_ID"
echo "  Network:    $NETWORK"
echo "  Source:     $SOURCE"
echo "  Dry-run:    $DRY_RUN"
echo "  Pace:       ${PACE_MS} ms between calls"
[[ -n "$AUDIT_LOG" ]] && echo "  Audit log:  $AUDIT_LOG"
echo ""

while IFS= read -r username || [[ -n "$username" ]]; do
    [[ -z "$username" || "$username" =~ ^# ]] && continue
    TOTAL=$((TOTAL + 1))

    if [[ "$DRY_RUN" = true ]]; then
        echo "[DRY-RUN] would verify: $username"
        log_audit "$username" "dry-run" "no transaction submitted"
        SKIPPED=$((SKIPPED + 1))
        continue
    fi

    echo "Verifying: $username ..."
    set +e
    output=$("$STELLAR" contract invoke \
        --id "$CONTRACT_ID" \
        --source-account "$SOURCE" \
        --network "$NETWORK" \
        --send=yes \
        -- verify \
        --github-username "$username" 2>&1)
    rc=$?
    set -e

    if [[ $rc -eq 0 ]]; then
        echo "  OK: $username"
        log_audit "$username" "ok" "verified"
        SUCCESS=$((SUCCESS + 1))
    else
        echo "  ERROR: $username — $output" >&2
        log_audit "$username" "error" "$output"
        FAILED=$((FAILED + 1))
        if [[ "$CONTINUE_ON_ERROR" = false ]]; then
            echo "Stopping batch on first error. Use --continue-on-error to process remaining usernames." >&2
            break
        fi
    fi

    # Pace to avoid RPC throttling
    [[ $PACE_MS -gt 0 ]] && sleep "$PACE_S"
done < "$FILE"

echo ""
echo "=== Summary ==="
echo "  Total:    $TOTAL"
echo "  Success:  $SUCCESS"
echo "  Failed:   $FAILED"
echo "  Dry-run:  $SKIPPED"
[[ -n "$AUDIT_LOG" ]] && echo "  Audit log written to: $AUDIT_LOG"

[[ $FAILED -gt 0 ]] && exit 1
exit 0
