# Pagination API Parity Tests - Issue #302

## Summary

Comprehensive parity test suite ensuring all three pagination APIs (`get_registered_page`, `get_registered_paginated`, `get_public_paginated`) behave consistently across critical scenarios. Prevents indexers from encountering edge cases around removal, empty registries, and boundary conditions.

## Problem Statement

Before Issue #302:
- Three pagination APIs diverged in test coverage
- `get_registered_page` had less coverage than cursor-based variants
- Indexers picking the least-tested API could skip users (Issues #52, #92, #143)
- No clear guidance on which API to use when
- Edge cases (empty registry, middle removal, last page) were undertested

## Test Coverage

### Shared Parity Tests (`tests/pagination_parity.rs`)

**24 tests** across 8 scenarios × 3 APIs:

#### 1. Empty Registry (3 tests)
- `test_parity_empty_registry_get_registered_page`
- `test_parity_empty_registry_get_registered_paginated`
- `test_parity_empty_registry_get_public_paginated`

**Verifies:** All APIs return empty result on empty registry.

#### 2. Single Record (3 tests)
- `test_parity_single_record_get_registered_page`
- `test_parity_single_record_get_registered_paginated`
- `test_parity_single_record_get_public_paginated`

**Verifies:** All APIs correctly return a single record.

#### 3. Middle Removal — Issue #52 (3 tests)
- `test_parity_middle_removal_get_registered_page`
- `test_parity_middle_removal_get_registered_paginated`
- `test_parity_middle_removal_get_public_paginated`

**Verifies:** After removing "bob" from [alice, bob, carol], all APIs skip it
and return only [alice, carol].

#### 4. Last Page Detection — Issue #143 (3 tests)
- `test_parity_last_page_get_registered_page`
- `test_parity_last_page_get_registered_paginated`
- `test_parity_last_page_get_public_paginated`

**Verifies:** All APIs correctly signal exhaustion (empty page, has_more=false,
next_cursor=None).

#### 5. Multi-Page Consistency (3 tests)
- `test_parity_multi_page_get_registered_page`
- `test_parity_multi_page_get_registered_paginated`
- `test_parity_multi_page_get_public_paginated`

**Verifies:** Walking multiple pages collects all records exactly once, no
duplicates or skips.

#### 6. Authorization Differences (3 tests)
- `test_parity_auth_get_registered_page_requires_admin`
- `test_parity_auth_get_registered_paginated_requires_admin`
- `test_parity_auth_get_public_paginated_is_permissionless`

**Verifies:** Admin APIs require auth, public API does not.

#### 7. Pause Behavior — Issue #294 (3 tests)
- `test_parity_pause_get_registered_page_works_while_paused`
- `test_parity_pause_get_registered_paginated_works_while_paused`
- `test_parity_pause_get_public_paginated_works_while_paused`

**Verifies:** All pagination APIs work while paused (read-only).

#### 8. Return Type Differences (3 tests)
- `test_parity_return_type_get_registered_page`
- `test_parity_return_type_get_registered_paginated`
- `test_parity_return_type_get_public_paginated`

**Verifies:** 
- `get_registered_page`: Returns `Vec<(String, Address)>` (no verified field)
- Cursor-based APIs: Return `ExportPage` with full `ContributorRecord`

## API Comparison

| Feature | `get_registered_page` | `get_registered_paginated` | `get_public_paginated` |
|---------|----------------------|---------------------------|------------------------|
| **Auth** | Admin | Admin | None (permissionless) |
| **Pagination** | Offset-based | Cursor-based | Cursor-based |
| **Return Type** | `Vec<(String, Address)>` | `ExportPage` | `ExportPage` |
| **Verified Field** | ❌ No | ✅ Yes | ✅ Yes |
| **Merkle Root** | ❌ No | ✅ Yes | ✅ Yes |
| **Works While Paused** | ✅ Yes | ✅ Yes | ✅ Yes (Issue #294) |
| **Export Attestation** | ❌ No | ✅ Yes | ❌ No |
| **Cursor Invalidation** | N/A | On removal | On removal |
| **Use Case** | Legacy offset | Modern admin export | Public indexers |

## API Selection Guide (Added to ABI.md)

### Use `get_registered_paginated` when:
- ✅ You have admin credentials
- ✅ You need full `ContributorRecord` metadata (verified, registered_at, is_bot)
- ✅ You need cursor-based pagination
- ✅ You need merkle roots for integrity verification
- ✅ You need export attestation support

### Use `get_public_paginated` when:
- ✅ Building a public dashboard or indexer
- ✅ No admin credentials available
- ✅ Need verified flag per record (Issue #96)
- ✅ Need cursor-based pagination
- ✅ Must work during pause (Issue #294)

### Use `get_registered_page` when:
- ✅ Need simple offset-based pagination
- ✅ Only need username + address (no verified field)
- ✅ Legacy tooling that predates cursor pagination

**Migration path:** `get_registered_page` → `get_registered_paginated`

## Documentation Updates

### ABI.md

Added comprehensive **Pagination API Selection Guide** section:

**Includes:**
- Comparison table of all 3 APIs
- When to use each API (use cases)
- Return type differences
- Parity guarantees across all APIs
- Consumer loop examples (offset vs cursor)
- Migration guidance

**Key clarifications:**
- `get_registered_page` is legacy offset-based
- Cursor-based APIs (`get_registered_paginated`, `get_public_paginated`) are modern
- `get_public_paginated` works during pause (Issue #294)
- Cursors are interchangeable between admin and public variants
- All APIs guarantee middle removal skip (Issue #52)

## Parity Guarantees

All three APIs now guarantee:

✅ **Empty registry:** Returns empty result  
✅ **Single record:** Returns that record  
✅ **Middle removal (Issue #52):** Skips removed username  
✅ **Last page (Issue #143):** Correct exhaustion signal  
✅ **Multi-page:** Visits every record exactly once  
✅ **Pause (Issue #294):** Works while paused  
✅ **No duplicates:** Each username appears at most once  
✅ **No skips:** Every live username appears  

## How to Run

```bash
# Run all parity tests
cargo test parity

# Run specific scenario across all APIs
cargo test test_parity_empty_registry

# Run specific API tests
cargo test test_parity.*get_registered_page
cargo test test_parity.*get_registered_paginated
cargo test test_parity.*get_public_paginated

# Run with verbose output
cargo test parity -- --nocapture

# Check test count
cargo test parity | grep -c "test result: ok"
```

## Integration Examples

### Offset-Based (Legacy)

```rust
let mut offset = 0;
let limit = 50;

loop {
    let page = get_registered_page(offset, limit)?;
    
    if page.is_empty() {
        break;  // No explicit end marker
    }
    
    for (username, address) in page {
        process(username, address);
        // No verified field available
    }
    
    offset += limit;
}
```

### Cursor-Based (Modern)

```rust
let mut cursor = None;
let limit = 50;

loop {
    let page = get_registered_paginated(cursor, limit)?;
    
    for (username, record) in page.records {
        process(username, record.stellar_address, record.verified);
        // Full metadata available
    }
    
    if !page.has_more {
        break;  // Explicit end signal
    }
    cursor = page.next_cursor;
}
```

### Public Cursor-Based (No Auth)

```rust
// Same as cursor-based above, but uses get_public_paginated
// No admin credentials needed
let mut cursor = None;
let limit = 50;

loop {
    let page = get_public_paginated(cursor, limit)?;
    
    for (username, record) in page.records {
        // Same ExportPage shape as admin API
        process(username, record.stellar_address, record.verified);
    }
    
    if !page.has_more {
        break;
    }
    cursor = page.next_cursor;
}
```

## Success Criteria (Issue #302)

✅ **Shared examples:** Empty, one record, middle remove, last page — all tested  
✅ **Document when to use which API:** Complete selection guide in ABI.md  
✅ **Parity tests:** 24 tests covering all 3 APIs across 8 scenarios  
✅ **No bugs found:** All APIs behave consistently  
✅ **Tests run:** `cargo test registered_page && cargo test paginat`  

## Edge Cases Covered

### Middle Removal (Issue #52)
**Scenario:** [alice, bob, carol] → remove bob  
**Result:** All APIs return [alice, carol]  
**Tests:** 3 parity tests

### Last Page Detection (Issue #143)
**Scenario:** Request page beyond end  
**Offset API:** Returns empty Vec  
**Cursor APIs:** has_more=false, next_cursor=None  
**Tests:** 3 parity tests

### Empty Registry
**Scenario:** Zero registrations  
**Offset API:** Returns empty Vec  
**Cursor APIs:** Empty page with has_more=false  
**Tests:** 3 parity tests

### Pause Behavior (Issue #294)
**Scenario:** Contract paused  
**All APIs:** Continue to work (read-only)  
**Rationale:** Indexers must stay synchronized  
**Tests:** 3 parity tests

## Related Issues

- **Issue #52:** Paginated export skips removed records
- **Issue #92:** Lookup after peer removal
- **Issue #96:** Include verified flag in export
- **Issue #143:** Pagination boundary conditions
- **Issue #294:** Public reads available while paused
- **Issue #302:** This work (pagination parity)

## Notes

- **No API deletion:** All three APIs remain available (backward compatibility)
- **Deprecation path:** `get_registered_page` → `get_registered_paginated`
- **Cursors interchangeable:** Admin and public cursors work across both APIs
- **Return types differ:** Choose API based on metadata needs
- **Auth differs:** Public API is permissionless, admin APIs require auth
- **All work while paused:** Read-only operations during maintenance

## Future Considerations

If a fourth pagination API is added:
1. Add tests to `pagination_parity.rs` for all 8 scenarios
2. Update ABI.md selection guide
3. Document differences in comparison table
4. Ensure parity with existing APIs

If an API is deprecated:
1. Document migration path in ABI.md
2. Add deprecation timeline
3. Keep tests for backward compatibility period
4. Eventually mark as legacy in docs
