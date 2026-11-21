#!/usr/bin/env bash
# bulk_verify.sh — Maintainer bulk verify CLI (Issue #286)
#
# Reads GitHub usernames (one per line) from a file and marks each one as verified.
# Calls the on-chain `batch_verify` in pages of up to MAX_WRITE_BATCH (25) when
# the deployed contract exposes it, and falls back to per-username `verify` on
# older deployments. Continues on partial failure and summarises the outcome.
# Includes pacing (configurable delay between calls) to avoid RPC throttling.
#
# Usage:
#   ./scripts/bulk_verify.sh --file usernames.txt \
#       --contract C... --source admin-identity --network testnet \
#       [--dry-run] [--pace-ms 500] [--audit-log audit.log] [--continue-on-error] \
#       [--batch-size 25] [--no-batch]
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
# Batching:
#   --batch-size N  usernames per batch_verify call (default 25, the contract's
#                   MAX_WRITE_BATCH; clamped to that ceiling).
#   --no-batch      skip batch_verify entirely and call verify once per username.
#   batch_verify is idempotent at the contract: an already-verified or unknown
#   username is counted in the returned BatchSummary as failed and skipped, and
#   the batch does not abort. A batch whose success_rate < 100 is treated as a
#   partial success here, not a hard error, unless --continue-on-error is unset
#   and the whole call failed.
#
# Pacing:
#   RPC nodes apply per-IP rate limits. Use --pace-ms (default 500 ms) to insert
#   a sleep between calls (per batch, or per username in fallback mode). Increase
#   to 1000–2000 ms for large runs or when hitting HTTP 429 responses.
#
# Audit log format (one JSON-like line per batch or username):
#   {"timestamp":"<ISO-8601>","scope":"batch|username","target":"<u|list>","network":"<n>","result":"ok|partial|error|dry-run","detail":"<msg>"}

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
BATCH_SIZE="${BATCH_SIZE:-25}"
MAX_WRITE_BATCH=25
NO_BATCH=false

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
        --batch-size)        BATCH_SIZE="$2";        shift 2 ;;
        --no-batch)          NO_BATCH=true;          shift ;;
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
[[ ! "$BATCH_SIZE" =~ ^[0-9]+$ ]] && echo "ERROR: --batch-size must be a positive integer." >&2 && exit 1
(( BATCH_SIZE < 1 )) && BATCH_SIZE=1
(( BATCH_SIZE > MAX_WRITE_BATCH )) && BATCH_SIZE=$MAX_WRITE_BATCH

# ---------- helpers ----------
log_audit() {
    local scope="$1" target="$2" result="$3" detail="$4"
    local ts; ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    local esc_detail=${detail//\\/\\\\}; esc_detail=${esc_detail//\"/\\\"}
    local line="{\"timestamp\":\"$ts\",\"scope\":\"$scope\",\"target\":\"$target\",\"network\":\"$NETWORK\",\"result\":\"$result\",\"detail\":\"$esc_detail\"}"
    echo "$line"
    [[ -n "$AUDIT_LOG" ]] && echo "$line" >> "$AUDIT_LOG"
}

invoke() {
    # Run `stellar contract invoke -- <args...>`, capturing combined output in
    # the global `output` and the return code in `rc`.
    set +e
    output=$("$STELLAR" contract invoke \
        --id "$CONTRACT_ID" \
        --source-account "$SOURCE" \
        --network "$NETWORK" \
        --send=yes \
        -- "$@" 2>&1)
    rc=$?
    set -e
}

# `true` once the deployed contract is shown not to expose `batch_verify`.
BATCH_UNSUPPORTED=false
looks_like_missing_fn() {
    grep -qiE 'unrecognized subcommand|unexpected argument|no such|not found|MissingValue.*batch_verify|unknown (function|method)' <<<"$1"
}

pace() { [[ ${PACE_MS:-0} -gt 0 ]] && sleep "$PACE_S"; }

# ---------- collect usernames ----------
USERNAMES=()
while IFS= read -r username || [[ -n "$username" ]]; do
    username="${username//$'\r'/}"
    [[ -z "$username" || "$username" =~ ^[[:space:]]*# ]] && continue
    USERNAMES+=("$username")
done < "$FILE"

TOTAL=${#USERNAMES[@]}
SUCCESS=0; FAILED=0; SKIPPED=0; BATCHES=0
PACE_S=$(echo "scale=3; $PACE_MS/1000" | bc 2>/dev/null || echo "0.5")

MODE="batch (size $BATCH_SIZE)"
[[ "$NO_BATCH" = true ]] && MODE="per-username (--no-batch)"

echo "=== bulk_verify.sh ==="
echo "  File:       $FILE"
echo "  Contract:   $CONTRACT_ID"
echo "  Network:    $NETWORK"
echo "  Source:     $SOURCE"
echo "  Usernames:  $TOTAL"
echo "  Mode:       $MODE"
echo "  Dry-run:    $DRY_RUN"
echo "  Pace:       ${PACE_MS} ms between calls"
[[ -n "$AUDIT_LOG" ]] && echo "  Audit log:  $AUDIT_LOG"
echo ""

verify_single() {
    local u="$1"
    if [[ "$DRY_RUN" = true ]]; then
        echo "[DRY-RUN] would verify: $u"
        log_audit "username" "$u" "dry-run" "no transaction submitted"
        SKIPPED=$((SKIPPED + 1))
        return 0
    fi
    echo "Verifying: $u ..."
    invoke verify --caller "$SOURCE" --github-username "$u"
    if [[ $rc -eq 0 ]] || grep -qi 'AlreadyVerified' <<<"$output"; then
        echo "  OK: $u"
        log_audit "username" "$u" "ok" "verified"
        SUCCESS=$((SUCCESS + 1))
        return 0
    fi
    echo "  ERROR: $u — $output" >&2
    log_audit "username" "$u" "error" "$output"
    FAILED=$((FAILED + 1))
    return 1
}

# Verify a batch. Returns non-zero only on a hard failure (whole call errored
# and it was not a missing-function fallback).
verify_batch() {
    local -a batch=("$@")
    local joined; joined=$(IFS=,; echo "${batch[*]}")
    local json="["; local u
    for u in "${batch[@]}"; do json+="\"$u\","; done
    json="${json%,}]"

    if [[ "$DRY_RUN" = true ]]; then
        echo "[DRY-RUN] would batch_verify ${#batch[@]}: $joined"
        log_audit "batch" "$joined" "dry-run" "no transaction submitted"
        SKIPPED=$((SKIPPED + ${#batch[@]}))
        return 0
    fi

    echo "batch_verify ${#batch[@]}: $joined ..."
    invoke batch_verify --caller "$SOURCE" --usernames "$json"

    if [[ $rc -ne 0 ]] && looks_like_missing_fn "$output"; then
        echo "  note: contract has no batch_verify — falling back to per-username verify" >&2
        BATCH_UNSUPPORTED=true
        local ok=0
        for u in "${batch[@]}"; do
            verify_single "$u" || { [[ "$CONTINUE_ON_ERROR" = false ]] && return 1; }
            pace
        done
        return 0
    fi

    BATCHES=$((BATCHES + 1))
    if [[ $rc -ne 0 ]]; then
        echo "  ERROR: batch failed — $output" >&2
        log_audit "batch" "$joined" "error" "$output"
        FAILED=$((FAILED + ${#batch[@]}))
        return 1
    fi

    local ok
    ok=$(grep -o '"successful"[^0-9]*[0-9]\+' <<<"$output" | grep -o '[0-9]\+$' | head -1 || true)
    [[ -z "$ok" ]] && ok=${#batch[@]}   # older CLI printing: assume full success on rc 0
    local miss=$(( ${#batch[@]} - ok ))
    SUCCESS=$((SUCCESS + ok))
    FAILED=$((FAILED + miss))
    if (( miss > 0 )); then
        echo "  PARTIAL: $ok/${#batch[@]} verified (already-verified / unknown usernames skipped)"
        log_audit "batch" "$joined" "partial" "successful=$ok of ${#batch[@]}"
    else
        echo "  OK: $ok/${#batch[@]} verified"
        log_audit "batch" "$joined" "ok" "successful=$ok"
    fi
    return 0
}

# ---------- main loop ----------
i=0
while (( i < TOTAL )); do
    if [[ "$NO_BATCH" = true || "$BATCH_UNSUPPORTED" = true ]]; then
        verify_single "${USERNAMES[$i]}" || {
            if [[ "$CONTINUE_ON_ERROR" = false ]]; then
                echo "Stopping on first error. Use --continue-on-error to process the rest." >&2
                break
            fi
        }
        i=$((i + 1))
    else
        chunk=("${USERNAMES[@]:i:BATCH_SIZE}")
        verify_batch "${chunk[@]}" || {
            if [[ "$CONTINUE_ON_ERROR" = false ]]; then
                echo "Stopping on first error. Use --continue-on-error to process the rest." >&2
                break
            fi
        }
        i=$((i + ${#chunk[@]}))
    fi
    pace
done

echo ""
echo "=== Summary ==="
echo "  Usernames: $TOTAL"
echo "  Verified:  $SUCCESS"
echo "  Failed:    $FAILED"
echo "  Dry-run:   $SKIPPED"
echo "  Batches:   $BATCHES"
[[ "$BATCH_UNSUPPORTED" = true ]] && echo "  Note: fell back to per-username verify (no batch_verify on contract)"
[[ -n "$AUDIT_LOG" ]] && echo "  Audit log written to: $AUDIT_LOG"

[[ $FAILED -gt 0 ]] && exit 1
exit 0
