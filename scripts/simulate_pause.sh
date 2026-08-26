#!/bin/bash
# scripts/simulate_pause.sh
# Simulates contract pause, verify blocked writes, unpause, and verify allowed writes.

set -e

CONTRACT_ID=$1
NETWORK=${2:-testnet}
SOURCE=${3:-default}

if [ -z "$CONTRACT_ID" ]; then
    echo "ERROR: Contract ID is required."
    echo "Usage: ./simulate_pause.sh <CONTRACT_ID> [NETWORK] [SOURCE]"
    exit 1
fi

STELLAR="stellar"
TEST_USER="pause-test-user"
TEST_ADDR="GB3D44A6UX73M7S7N2GCRMYWOBKCQMJKWYZS224CPAWTXU25NUXFLKHO"

echo "=== STEP 1: Pausing contract ==="
$STELLAR contract invoke \
  --id "$CONTRACT_ID" \
  --source-account "$SOURCE" \
  --network "$NETWORK" \
  --send=yes \
  -- pause

echo "=== STEP 2: Checking is_paused ==="
IS_PAUSED=$($STELLAR contract invoke \
  --id "$CONTRACT_ID" \
  --source-account "$SOURCE" \
  --network "$NETWORK" \
  -- is_paused)
echo "is_paused returned: $IS_PAUSED"

if [ "$IS_PAUSED" != "true" ]; then
    echo "ERROR: Contract should be paused!"
    exit 1
fi

echo "=== STEP 3: Attempting to register while paused (should fail) ==="
set +e
$STELLAR contract invoke \
  --id "$CONTRACT_ID" \
  --source-account "$SOURCE" \
  --network "$NETWORK" \
  --send=yes \
  -- register \
  --github-username "$TEST_USER" \
  --stellar-address "$TEST_ADDR" 2>pause_err.txt
EXIT_CODE=$?
set -e

if [ $EXIT_CODE -eq 0 ]; then
    echo "ERROR: Registration succeeded but should have failed while paused!"
    cat pause_err.txt
    exit 1
else
    echo "SUCCESS: Registration failed as expected while paused."
    echo "Error message captured:"
    cat pause_err.txt
fi

echo "=== STEP 4: Unpausing contract ==="
$STELLAR contract invoke \
  --id "$CONTRACT_ID" \
  --source-account "$SOURCE" \
  --network "$NETWORK" \
  --send=yes \
  -- unpause

echo "=== STEP 5: Registering after unpause (should succeed) ==="
$STELLAR contract invoke \
  --id "$CONTRACT_ID" \
  --source-account "$SOURCE" \
  --network "$NETWORK" \
  --send=yes \
  -- register \
  --github-username "$TEST_USER" \
  --stellar-address "$TEST_ADDR"

echo "=== STEP 6: Verifying registration ==="
REG_RECORD=$($STELLAR contract invoke \
  --id "$CONTRACT_ID" \
  --source-account "$SOURCE" \
  --network "$NETWORK" \
  -- get_address --github-username "$TEST_USER")

echo "get_address returned: $REG_RECORD"
echo "=== SIMULATION COMPLETED SUCCESSFULLY ==="
rm -f pause_err.txt
