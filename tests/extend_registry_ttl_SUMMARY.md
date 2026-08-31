# extend_registry_ttl Tests and Documentation - Issue #301

## Summary

Comprehensive test suite for `extend_registry_ttl` and complete documentation of BatchConfig bounds. The TTL keeper will call this function in production, so it now has exhaustive coverage for all paths, limits, and error conditions.

## Problem Statement

Before Issue #301:
- `extend_registry_ttl` had minimal test coverage (only one basic test)
- BatchConfig limits were not clearly documented in ABI.md
- Invalid batch size paths were undertested
- Interaction with pause state was unclear
- STORAGE_RENT.md incorrectly referenced MAX_PAGE_LIMIT (200) instead of actual limit (100)

## Test Coverage

### Test Suite (`tests/extend_registry_ttl.rs`)

**25 dedicated tests** across 9 categories:

#### 1. Happy Path Tests (6 tests)
- ✅ Single registered username
- ✅ Multiple registered usernames
- ✅ Mixed registered and unregistered (partial success)
- ✅ All unregistered usernames (returns 0)
- ✅ Duplicate usernames in list
- Tests: `test_extend_registry_ttl_single_registered_username`, etc.

#### 2. Batch Size Limits (4 tests)
- ✅ Empty list rejected with `InvalidBatchSize`
- ✅ At max limit (100 usernames) succeeds
- ✅ Over max limit (101) rejected with `InvalidBatchSize`
- ✅ Batch size 1 (minimum valid)
- Tests: `test_extend_registry_ttl_empty_list_rejected`, etc.

#### 3. Authorization & Permissionless Access (1 test)
- ✅ Any caller can extend TTL (permissionless by design)
- Test: `test_extend_registry_ttl_is_permissionless`

#### 4. Contract State Interaction (2 tests)
- ✅ Works while paused (TTL extension is read-like)
- ✅ Before initialize fails with `NotInitialized`
- Tests: `test_extend_registry_ttl_works_while_paused`, etc.

#### 5. TTL Behavior (2 tests)
- ✅ Idempotent (can extend same username multiple times)
- ✅ After removal returns 0 (not an error)
- Tests: `test_extend_registry_ttl_idempotent`, etc.

#### 6. Edge Cases (3 tests)
- ✅ Maximum-length username (39 chars)
- ✅ Single-character username
- ✅ Case-folded lookup (Alice → alice)
- Tests: `test_extend_registry_ttl_maximum_length_username`, etc.

#### 7. Error Code Validation (2 tests)
- ✅ InvalidBatchSize is error code 14
- ✅ InvalidBatchSize is Fatal (not retryable)
- Tests: `test_extend_registry_ttl_invalid_batch_size_error_code`, etc.

#### 8. Documentation Validation (2 tests)
- ✅ BatchConfig::default().max_batch_size is 100
- ✅ extend_registry_ttl uses default (not for_writes)
- Tests: `test_batch_config_default_max_is_100`, etc.

#### 9. Performance & Resource Tests (2 tests)
- ✅ Large batch (50 usernames)
- ✅ Varying username lengths
- Tests: `test_extend_registry_ttl_large_batch_50_usernames`, etc.

#### 10. Coverage Meta-test (1 test)
- ✅ Documents expected test count and categories
- Test: `test_extend_registry_ttl_coverage_complete`

## Documentation Updates

### ABI.md

Added complete `extend_registry_ttl` entry point specification:

**Key documentation points:**
- **Auth:** Permissionless by design
- **Batch limits:** 1–100 usernames (`BatchConfig::default().max_batch_size`)
- **Error codes:** `NotInitialized`, `InvalidBatchSize` (code 14)
- **Returns:** Count of successfully extended records (u32)
- **Partial success:** Unregistered usernames are skipped, not errors
- **Idempotent:** Safe to extend same username multiple times
- **Works while paused:** Unlike state-mutating functions
- **Keeper workflow:** Complete example with CLI invocation

### STORAGE_RENT.md

**Fixed incorrect batch size reference:**
- Before: "up to `MAX_PAGE_LIMIT` (200) records"
- After: "up to **100 records** (`BatchConfig::default().max_batch_size`)"

**Updated keeper implementation section:**
- Corrected batch size to 100
- Added link to ABI.md specification
- Clarified the batch grouping strategy

## BatchConfig Bounds

### extend_registry_ttl
- **Config:** `BatchConfig::default()`
- **Max batch size:** 100
- **Rationale:** TTL extension is cheap (no deserialization, events, or audit logs)

### Write Operations (batch_verify, batch_remove)
- **Config:** `BatchConfig::for_writes()`
- **Max batch size:** 25 (MAX_WRITE_BATCH)
- **Rationale:** Write batches are expensive (read, write, TTL, event, audit per entry)

### Why Different Limits?

The default 100 was a shape check, not a resource budget. Write operations pay:
- Persistent read
- Persistent write
- TTL extension
- Event publish
- Audit log append

TTL extension only pays:
- Persistent `.extend_ttl()` call (no deserialization)

Therefore, `extend_registry_ttl` safely uses the larger default batch size (100)
while write operations use the tighter resource-based cap (25).

## How to Run

```bash
# Run all extend_registry_ttl tests
cargo test extend_registry_ttl

# Run specific test
cargo test test_extend_registry_ttl_single_registered_username

# Run batch size tests
cargo test test_extend_registry_ttl.*batch.*size

# Run with verbose output
cargo test extend_registry_ttl -- --nocapture

# Check test count
cargo test extend_registry_ttl | grep -c "test result: ok"
```

## Keeper Integration

### Production Workflow

```bash
# 1. Read registry (admin or public endpoint)
stellar contract invoke --id $ID --source keeper \
  --network testnet \
  -- get_public_paginated --cursor 0 --limit 100

# 2. Identify cold records (TTL < 30 days)
# (Off-chain logic)

# 3. Batch up to 100 usernames and extend
stellar contract invoke --id $ID --source keeper \
  --network testnet --send=yes \
  -- extend_registry_ttl \
  --usernames '["alice","bob","carol",...,"user100"]'
```

### Return Value Interpretation

```rust
let extended = extend_registry_ttl(usernames)?;

if extended == usernames.len() {
    // All usernames were found and extended
} else {
    // Some usernames were not found (removed since list was built)
    // This is not an error — keeper list can lag behind removals
}
```

## Error Handling

### InvalidBatchSize (code 14)
- **Cause:** Empty list or > 100 usernames
- **Category:** Fatal (not retryable)
- **Fix:** Adjust batch size to 1–100

### NotInitialized (code 2)
- **Cause:** Contract not initialized
- **Category:** Fatal
- **Fix:** Call `initialize` first

### Partial Success (not an error)
- **Scenario:** Some usernames not registered
- **Behavior:** Returns count of successfully extended records
- **Handling:** Normal — keeper list can lag

## Performance Characteristics

### Cost per username
- **Storage operations:** 1 × persistent `.extend_ttl()`
- **Events:** None
- **Audit:** None
- **Budget impact:** Minimal (read-like operation)

### Batch efficiency
- **1 username:** 1 transaction
- **100 usernames:** Still 1 transaction
- **Savings:** 99× fewer transactions, signatures, and fees

### Resource limits
- **Instruction budget:** Ample headroom at 100 usernames
- **Memory:** No record deserialization (just key operations)
- **Footprint:** Minimal (no new state written)

## Success Criteria (Issue #301)

✅ **Dedicated tests:** 25 tests covering all paths  
✅ **Happy extend:** Multiple positive path tests  
✅ **Oversize:** Over-limit batch rejected with InvalidBatchSize  
✅ **Unauthorized:** Confirmed permissionless (intentional)  
✅ **Paused:** Works while paused (TTL is read-like)  
✅ **Empty:** Empty list rejected  
✅ **ABI bounds:** Complete specification with batch size limits  
✅ **Rent doc pointer:** STORAGE_RENT.md updated with correct limits  

## Related Files

- `src/lib.rs` — extend_registry_ttl implementation (line ~1732)
- `src/batch.rs` — BatchConfig default and for_writes
- `src/storage.rs` — extend_record_ttl (TTL extension logic)
- `docs/ABI.md` — API specification (new section added)
- `docs/STORAGE_RENT.md` — Keeper workflow and cost estimation
- `scripts/ttl_keeper.sh` — Production keeper script
- `tests/extend_registry_ttl.rs` — Dedicated test suite (new file)

## Future Considerations

### If batch size needs to increase:
1. Update `BatchConfig::default().max_batch_size`
2. Run performance benchmarks to confirm budget headroom
3. Update ABI.md and STORAGE_RENT.md documentation
4. Update test `test_extend_registry_ttl_at_max_batch_size`

### If TTL strategy changes:
1. Update `TTL_THRESHOLD` and `TTL_BUMP` in storage.rs
2. Update keeper cadence recommendations in STORAGE_RENT.md
3. Re-run cost estimation with new parameters

## Comparison to Other Batch Operations

| Function | Batch Size | Config | Reason |
|----------|-----------|--------|--------|
| `extend_registry_ttl` | 100 | `default()` | Read-like, cheap |
| `batch_verify` | 25 | `for_writes()` | Full write cost |
| `batch_remove` | 25 | `for_writes()` | Full write cost |
| `get_registered_paginated` | 100 (cap) | `MAX_PAGE_LIMIT` | Export pagination |
| `get_public_paginated` | 100 (cap) | `MAX_PAGE_LIMIT` | Public pagination |

## Notes

- **Permissionless by design:** Anyone can call, reduces operational SPOF
- **Partial success is normal:** Keeper list can lag behind removals
- **Works while paused:** Critical for keeper continuity
- **Idempotent:** Safe to call multiple times for same usernames
- **No events emitted:** Silent operation, just TTL extension
- **Error code 14:** InvalidBatchSize is the only non-initialization error
