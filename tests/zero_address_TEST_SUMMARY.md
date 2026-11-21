# Zero-Address Test Suite - Issue #300

## Summary

This test suite validates that the zero-address guard (`is_zero_address`) remains effective against current Soroban SDK `mock_all_auths` behavior.

## Problem Statement

The zero-address guard exists because `mock_all_auths` in tests bypasses the normal `require_auth()` check that would reject the zero address on a live network (since no private key exists for it). Without the explicit guard, tests could register the zero address as a valid entry, and SDK updates could potentially reintroduce this vulnerability.

## Test Coverage

### Core Entry Points (4 guards, 4 tests)

1. **`register` with zero stellar_address**
   - Test: `test_zero_address_register_stellar_address_rejected_with_mock_all_auths`
   - Guard location: `src/lib.rs:1506`
   - Verifies: Primary registration path blocks zero address even with `mock_all_auths`

2. **`register` with zero fallback address**
   - Test: `test_zero_address_register_fallback_address_rejected_with_mock_all_auths`
   - Guard location: `src/lib.rs:1531`
   - Verifies: Fallback addresses list is also validated

3. **`register_sponsored` with zero stellar_address**
   - Test: `test_zero_address_register_sponsored_rejected_with_mock_all_auths`
   - Guard location: `src/lib.rs:1633`
   - Verifies: Sponsored registration has the same protection

4. **`request_address_rotation` with zero new_address**
   - Test: `test_zero_address_rotation_request_rejected_with_mock_all_auths`
   - Guard location: `src/lib.rs:3133`
   - Verifies: Address rotation cannot target zero address

### Edge Cases & Scenarios

5. **Re-registration to zero address**
   - Test: `test_zero_address_reregistration_rejected_with_mock_all_auths`
   - Verifies: Address update path (existing username → zero address) is blocked

6. **Mixed fallback list**
   - Test: `test_zero_address_in_mixed_fallback_list_rejected`
   - Verifies: Guard rejects list containing any zero address among valid ones

### Positive Controls

7. **Valid address registration succeeds**
   - Test: `test_valid_address_register_succeeds_with_mock_all_auths`
   - Verifies: Guard doesn't break normal operation, `mock_all_auths` works correctly

### Public API Validation

8. **`is_address_zero` helper**
   - Test: `test_is_address_zero_helper_identifies_zero_address`
   - Verifies: Public read function agrees with internal guard logic

### Error Code Stability

9. **ZeroAddress error code is 16**
   - Test: `test_zero_address_error_code_is_stable`
   - Verifies: Off-chain consumers relying on numeric code 16 remain compatible

10. **ZeroAddress is Fatal (not retryable)**
    - Test: `test_zero_address_error_is_fatal_not_retryable`
    - Verifies: Error classification is correct for retry logic

### Regression Detection

11. **Guard removal detector**
    - Test: `test_guard_removal_would_allow_zero_address_registration`
    - Verifies: If guard is removed, test fails with clear message

### Documentation Accuracy

12. **SECURITY.md / ABI.md accuracy**
    - Test: `test_security_md_zero_address_documentation_is_accurate`
    - Verifies: Documented strkey, error code, and behavior match implementation

## Documentation Updates

- **Fixed**: `docs/ABI.md` line 334 - corrected error code from 15 to 16

## How to Run

```bash
# Run all zero-address tests
cargo test zero

# Run individual test
cargo test test_zero_address_register_stellar_address_rejected_with_mock_all_auths

# Run with verbose output
cargo test zero -- --nocapture
```

## Success Criteria (Issue #300)

✅ Tests use current SDK mock auth APIs (`mock_all_auths`)  
✅ Tests still fail when trying to register zero address (guard required)  
✅ SECURITY.md/ABI.md one-liner is accurate (code 16, not 15)  
✅ Tests fail if someone removes the guard (regression detection)  
✅ All tests run in default CI (not wasm-test gated)  
✅ Test names match `cargo test zero` pattern  

## Zero Address Constant

```
GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF
```

This is the base32 (strkey) encoding of an all-zero 32-byte ed25519 public key with a valid checksum. No private key can exist for this address.

## Notes

- All 4 guard locations in `src/lib.rs` are tested
- Tests are intentionally verbose with clear failure messages
- Each test includes a docstring explaining its purpose
- The test file is standalone and doesn't depend on other test modules
