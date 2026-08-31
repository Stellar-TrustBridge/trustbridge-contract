# Homoglyph Extra Checks - Issue #299

## Summary

Comprehensive homoglyph and confusable character test corpus ensuring that ASCII-only username validation has no bypass paths. All non-ASCII characters — whether visually identical lookalikes, invisible marks, or bidirectional overrides — are rejected before reaching storage.

## Problem Statement

While `utils.rs` already rejects Unicode via ASCII-only validation, sophisticated attacks using homoglyphs, zero-width joiners (ZWJ), and bidirectional marks could slip through if any code path bypasses the validation. This test suite provides:

1. **Exhaustive corpus** of known confusable characters
2. **No silent accept** guarantee for lookalikes
3. **Bypass path detection** at the `register()` entry point
4. **Documented security guarantee** in SECURITY.md

## Test Coverage

### Homoglyph Corpus Tests (`tests/homoglyph_corpus.rs`)

#### 1. Cyrillic Lookalikes (23 characters)
- Small letters: а, е, о, р, с, х, у, і, ј (look like a, e, o, p, c, x, y, i, j)
- Capital letters: А, В, С, Е, Н, І, Ј, К, М, О, Р, Т, Х, У
- Test: `test_homoglyph_corpus_cyrillic_lookalikes_all_rejected`

#### 2. Greek Lookalikes (20 characters)
- Small letters: α, ο, ν, ρ, τ, υ, χ (look like a, o, v, p, t, u, x)
- Capital letters: Α, Β, Ε, Η, Ι, Κ, Μ, Ν, Ο, Ρ, Τ, Υ, Χ, Ζ
- Test: `test_homoglyph_corpus_greek_lookalikes_all_rejected`

#### 3. Latin Extended & Diacritics (16 characters)
- Accented variants: á, à, ã, å, é, è, í, ï, ó, õ, ñ, ú, ü, ç
- Test: `test_homoglyph_corpus_latin_extended_all_rejected`

#### 4. Zero-Width & Invisible Characters (8 characters)
- U+200B: Zero-width space
- U+200C: Zero-width non-joiner (ZWNJ)
- U+200D: Zero-width joiner (ZWJ)
- U+2060: Word joiner
- U+00AD: Soft hyphen
- U+2061–U+2063: Invisible operators
- Tests:
  - `test_invisible_characters_zero_width_joiners_rejected`
  - `test_invisible_characters_at_any_position_rejected`

#### 5. Bidirectional Override Marks (11 characters)
- U+200E, U+200F: LTR/RTL marks
- U+202A–U+202E: Embedding and override controls
- U+2066–U+2069: Directional isolates
- Tests:
  - `test_bidirectional_override_marks_rejected`
  - `test_bidirectional_complex_reversal_attack_rejected`

#### 6. Mixed-Script Confusables
- Combinations like "аlice" (Cyrillic а + ASCII lice)
- Multi-script attacks with characters from 2+ Unicode blocks
- Tests:
  - `test_mixed_script_confusables_rejected`
  - `test_mixed_script_every_byte_validated`

#### 7. Full-Width Latin (Japanese forms)
- U+FF21–U+FF5A: Full-width A-Z, a-z
- Example: "ａｌｉｃｅ" (full-width) vs "alice" (ASCII)
- Test: `test_fullwidth_latin_letters_rejected`

#### 8. Mathematical Alphanumeric Symbols
- U+1D400–U+1D7FF: Bold, italic, script, fraktur, monospace variants
- Example: "𝐚𝐥𝐢𝐜𝐞" (bold) vs "alice"
- Test: `test_mathematical_alphanumeric_symbols_rejected`

#### 9. Superscripts, Subscripts, Modifiers
- Modifier letters and super/subscript variants
- Test: `test_superscript_subscript_modifier_letters_rejected`

### Integration Tests

#### 10. Register Entry Point Bypass Detection
- Tests actual `register()` function, not just `is_valid_github_username`
- Attempts:
  - Cyrillic homoglyph registration
  - Zero-width joiner in username
  - Bidirectional override
- Test: `test_homoglyph_registration_blocked_at_register_entry_point`

### Positive Controls

#### 11. Valid ASCII Still Works
- Confirms ASCII-only policy doesn't break legitimate usernames
- Test: `test_valid_ascii_usernames_still_accepted_after_homoglyph_hardening`

#### 12. Coverage Completeness
- Meta-test documenting expected corpus size
- Test: `test_homoglyph_corpus_coverage_complete`

## Security Guarantee

**Documented in `docs/SECURITY.md` (Issue #299):**

> **We reject all non-ASCII.** Every codepoint above U+007F is blocked before
> it reaches storage, regardless of how it renders. The corpus tests validate
> this property against 78+ known confusable characters and ensure no bypass
> path exists at the `register()` entry point.

## Attack Vectors Covered

### Visual Confusion (Homoglyphs)
- **Threat**: "аlice" (Cyrillic) looks identical to "alice" (ASCII)
- **Defense**: Byte-wise validation rejects U+0430 (Cyrillic а)
- **Coverage**: 59 lookalike characters across Cyrillic, Greek, Latin-extended

### Invisible Tampering (Zero-Width Characters)
- **Threat**: "al\u{200D}ice" vs "alice" — same rendering, different keys
- **Defense**: All zero-width marks (U+200B–U+206F) are non-ASCII, rejected
- **Coverage**: 8 invisible control characters

### Text Direction Manipulation (Bidi)
- **Threat**: "alice\u{202E}bob" may render reversed in some contexts
- **Defense**: All bidi controls (U+200E–U+2069) are non-ASCII, rejected
- **Coverage**: 11 directional formatting marks

### Mixed-Encoding Attacks
- **Threat**: Combine lookalikes from multiple scripts to evade single-script checks
- **Defense**: Every byte validated independently — one non-ASCII = reject all
- **Coverage**: Position-based tests (prefix, infix, suffix, multiple)

### Width Variants
- **Threat**: Full-width "ａｌｉｃｅ" (CJK context) vs ASCII "alice"
- **Defense**: Full-width forms are 3-byte UTF-8, rejected by ASCII check
- **Coverage**: Full-width Latin A-Z, a-z

### Stylistic Variants
- **Threat**: Mathematical bold "𝐚𝐥𝐢𝐜𝐞" vs ASCII "alice"
- **Defense**: Math alphanumeric symbols are 4-byte UTF-8, rejected
- **Coverage**: 6 mathematical font variants

## How to Run

```bash
# Run all homoglyph corpus tests
cargo test homoglyph

# Run all Unicode rejection tests (includes existing + corpus)
cargo test unicode

# Run specific corpus test
cargo test test_homoglyph_corpus_cyrillic_lookalikes_all_rejected

# Run bypass detection test
cargo test test_homoglyph_registration_blocked_at_register_entry_point

# Run with verbose output
cargo test homoglyph -- --nocapture
```

## Implementation Details

### Validation Strategy

The defense is byte-level, not character-level:

```rust
// Every byte must be ASCII (< 0x80)
for &b in bytes.iter() {
    if !b.is_ascii() {
        return false;  // Reject entire username
    }
}
```

This works because:
- ASCII characters: 1 byte, value 0x00–0x7F
- All non-ASCII Unicode: 2–4 bytes, leading byte ≥ 0x80
- Leading byte check catches every multi-byte sequence

### No Normalization

The contract does **not** perform:
- Unicode normalization (NFC, NFD, NFKC, NFKD)
- Case folding beyond ASCII (IDNA/UTS46)
- Homoglyph substitution or "smart" fixes

Rationale: GitHub usernames are ASCII-only. Trying to "fix" non-ASCII input
would create a canonicalization attack surface. Reject and ask the user to
submit the correct ASCII form.

## Success Criteria (Issue #299)

✅ **Fuzz/table of homoglyph strings all fail `is_username_valid`**
   - 78+ corpus entries across 9 categories

✅ **SECURITY.md updated with guarantee**
   - "We reject all non-ASCII" documented
   - Corpus tests referenced
   - Run commands provided

✅ **No silent accept of lookalikes**
   - Every test includes failure message with codepoint
   - Integration test at `register()` catches bypass paths

✅ **If any accept path exists, tests close it**
   - Bypass detection test fails if validation is skipped
   - Mixed-script tests ensure every byte is checked

## Related Issues

- **Issue #70**: Original Unicode rejection policy implementation
- **Issue #69**: Wave #69 Unicode hardening
- **Issue #194**: Username case-folding (ASCII-only, no Unicode normalization)
- **Issue #299**: This work (extended homoglyph corpus)

## Corpus Growth

If a new attack vector is discovered:
1. Add it to `tests/homoglyph_corpus.rs` in the appropriate section
2. Include the Unicode codepoint and a visual example
3. Update `test_homoglyph_corpus_coverage_complete` expected count
4. Document it in SECURITY.md if it's a new category

## References

- [Unicode Security Guide](https://unicode.org/reports/tr36/)
- [Unicode Confusables](https://util.unicode.org/UnicodeJsps/confusables.jsp)
- [Invisible Characters](https://invisible-characters.com/)
- GitHub's actual username rules (ASCII alphanumerics + hyphen only)
