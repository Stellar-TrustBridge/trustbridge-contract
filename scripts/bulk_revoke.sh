#!/usr/bin/env bash
# bulk_revoke.sh — Maintainer bulk revoke_verification CLI
#
# Reads GitHub usernames (one per line) from a file and revokes each one.
# Supports dry-run mode, confirmation prompts, and per-line audit logging.
#
# Usage:
#   ./scripts/bulk_revoke.sh --file usernames.txt \
#       --contract C... --source admin-identity --network testnet \
#       [--dry-run] [--yes] [--audit-log audit.log] [--continue-on-error]
#
# Required env (or flags):
#   CONTRACT_ID  — deployed contract C-address
#   SOURCE       — Stellar CLI identity (must be admin or Verifier)
#   NETWORK      — testnet | mainnet | futurenet (never defaults to mainnet)
#
# Safety rules:
#   - NETWORK never defaults to mainnet; explicit --network mainnet required.
#   - Dry-run prints what would run without submitting any transaction.
#   - Without --yes, prompts for confirmation before sending to mainnet.
#   - Failures do not abort the batch unless --continue-on-error is omitted.
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
CONFIRM=false
CONTINUE_ON_ERROR=false
AUDIT_LOG=""
STELLAR="${STELLAR:-stellar}"

usage() {
    grep '^#' "$0" | sed 's/^# \?//' | grep -v '^!'
    exit 1
}

# ---------- arg parse ----------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --file)          FILE="$2";          shift 2 ;;
        --contract)      CONTRACT_ID="$2";   shift 2 ;;
        --source)        SOURCE="$2";        shift 2 ;;
        --network)       NETWORK="$2";       shift 2 ;;
        --dry-run)       DRY_RUN=true;       shift ;;
        --yes)           CONFIRM=true;       shift ;;
        --continue-on-error) CONTINUE_ON_ERROR=true; shift ;;
        --audit-log)     AUDIT_LOG="$2";     shift 2 ;;
        -h|--help)       usage ;;
        *) echo "Unknown flag: $1"; usage ;;
    esac
done

# ---------- validation ----------
[[ -z "$FILE" ]]        && echo "ERROR: --file is required." >&2 && exit 1
[[ -z "$CONTRACT_ID" ]] && echo "ERROR: --contract (or CONTRACT_ID env) is required." >&2 && exit 1
[[ -z "$NETWORK" ]]     && echo "ERROR: --network is required (testnet | futurenet | mainnet). Never defaults to mainnet." >&2 && exit 1
[[ ! -f "$FILE" ]]      && echo "ERROR: file not found: $FILE" >&2 && exit 1

# Explicit mainnet guard: require --yes confirmation
if [[ "$NETWORK" = "mainnet" && "$DRY_RUN" = false && "$CONFIRM" = false ]]; then
    echo "WARNING: You are about to bulk-revoke on MAINNET."
    read -r -p "Type 'yes' to confirm mainnet bulk revoke: " ans
    [[ "$ans" != "yes" ]] && echo "Aborted." && exit 1
fi

CALLER=$("$STELLAR" keys address "$SOURCE" 2>/dev/null || echo "")
[[ -z "$CALLER" ]] && echo "ERROR: could not resolve address for identity '$SOURCE'." >&2 && exit 1

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

echo "=== bulk_revoke.sh ==="
echo "  File:       $FILE"
echo "  Contract:   $CONTRACT_ID"
echo "  Network:    $NETWORK"
echo "  Source:     $SOURCE ($CALLER)"
echo "  Dry-run:    $DRY_RUN"
[[ -n "$AUDIT_LOG" ]] && echo "  Audit log:  $AUDIT_LOG"
echo ""

while IFS= read -r username || [[ -n "$username" ]]; do
    # skip blank lines and comments
    [[ -z "$username" || "$username" =~ ^# ]] && continue
    TOTAL=$((TOTAL + 1))

    if [[ "$DRY_RUN" = true ]]; then
        echo "[DRY-RUN] would revoke: $username"
        log_audit "$username" "dry-run" "no transaction submitted"
        SKIPPED=$((SKIPPED + 1))
        continue
    fi

    echo "Revoking: $username ..."
    set +e
    output=$("$STELLAR" contract invoke \
        --id "$CONTRACT_ID" \
        --source-account "$SOURCE" \
        --network "$NETWORK" \
        --send=yes \
        -- revoke_verification \
        --caller "$CALLER" \
        --github-username "$username" 2>&1)
    rc=$?
    set -e

    if [[ $rc -eq 0 ]]; then
        echo "  OK: $username"
        log_audit "$username" "ok" "revoked"
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
