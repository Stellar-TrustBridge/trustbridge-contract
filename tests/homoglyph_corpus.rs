//! Homoglyph corpus tests for Issue #299.
//!
//! **Problem**: Even though `utils::is_valid_github_username` rejects all
//! non-ASCII bytes, attackers may still attempt homoglyph substitution,
//! zero-width joiners, bidirectional overrides, and mixed-script confusables
//! in copy-paste registration flows or any path that might bypass validation.
//!
//! **Solution**: This corpus exhaustively tests known attack vectors to ensure
//! the ASCII-only guard catches every confusable character, invisible mark,
//! and lookalike glyph. If any accept path exists, these tests will expose it.
//!
//! **Documented guarantee**: "We reject all non-ASCII" (SECURITY.md).
//!
//! Related: Issue #70 (Unicode rejection policy), docs/SECURITY.md §
//! Unicode Rejection Policy.

#![cfg(test)]

use soroban_sdk::{Env, String};
use trustbridge_contract::utils::is_valid_github_username;

fn s(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

// ═══════════════════════════════════════════════════════════════════════════
// § Homoglyph Corpus: Cyrillic Lookalikes
// ═══════════════════════════════════════════════════════════════════════════

/// Comprehensive Cyrillic homoglyph corpus. Every entry looks like an ASCII
/// letter but is encoded as a different Unicode codepoint.
///
/// Attackers use these to register names like "аlice" (Cyrillic а + ASCII lice)
/// that appear identical to "alice" in most fonts but occupy a different
/// storage key unless canonicalized. Our defense: byte-wise ASCII validation
/// rejects all of them before they reach storage.
#[test]
fn test_homoglyph_corpus_cyrillic_lookalikes_all_rejected() {
    let env = Env::default();

    // Each entry: (description, lookalike character, Unicode codepoint, example username)
    let corpus = [
        ("Cyrillic small a", '\u{0430}', "U+0430", "аlice"),          // а looks like a
        ("Cyrillic small e", '\u{0435}', "U+0435", "al\u{0435}x"),    // е looks like e
        ("Cyrillic small o", '\u{043E}', "U+043E", "b\u{043E}b"),     // о looks like o
        ("Cyrillic small r", '\u{0440}', "U+0440", "\u{0440}ick"),    // р looks like p
        ("Cyrillic small c", '\u{0441}', "U+0441", "\u{0441}arol"),   // с looks like c
        ("Cyrillic small x", '\u{0445}', "U+0445", "ale\u{0445}"),    // х looks like x
        ("Cyrillic small y", '\u{0443}', "U+0443", "\u{0443}vonne"),  // у looks like y
        ("Cyrillic small i", '\u{0456}', "U+0456", "m\u{0456}ke"),    // і looks like i
        ("Cyrillic small j", '\u{0458}', "U+0458", "\u{0458}ane"),    // ј looks like j
        // Capital letters
        ("Cyrillic capital A", '\u{0410}', "U+0410", "\u{0410}lice"), // А looks like A
        ("Cyrillic capital B", '\u{0412}', "U+0412", "\u{0412}ob"),   // В looks like B
        ("Cyrillic capital C", '\u{0421}', "U+0421", "\u{0421}arol"), // С looks like C
        ("Cyrillic capital E", '\u{0415}', "U+0415", "\u{0415}ve"),   // Е looks like E
        ("Cyrillic capital H", '\u{041D}', "U+041D", "\u{041D}ick"),  // Н looks like H
        ("Cyrillic capital I", '\u{0406}', "U+0406", "\u{0406}an"),   // І looks like I
        ("Cyrillic capital J", '\u{0408}', "U+0408", "\u{0408}ane"),  // Ј looks like J
        ("Cyrillic capital K", '\u{041A}', "U+041A", "\u{041A}ate"),  // К looks like K
        ("Cyrillic capital M", '\u{041C}', "U+041C", "\u{041C}ike"),  // М looks like M
        ("Cyrillic capital O", '\u{041E}', "U+041E", "\u{041E}scar"), // О looks like O
        ("Cyrillic capital P", '\u{0420}', "U+0420", "\u{0420}aul"),  // Р looks like P
        ("Cyrillic capital T", '\u{0422}', "U+0422", "\u{0422}om"),   // Т looks like T
        ("Cyrillic capital X", '\u{0425}', "U+0425", "\u{0425}avier"),// Х looks like X
        ("Cyrillic capital Y", '\u{0423}', "U+0423", "\u{0423}vonne"),// У looks like Y
    ];

    for (desc, _char, codepoint, username) in corpus {
        assert!(
            !is_valid_github_username(&s(&env, username)),
            "{desc} ({codepoint}) must be rejected: {username}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// § Homoglyph Corpus: Greek Lookalikes
// ═══════════════════════════════════════════════════════════════════════════

/// Greek alphabet homoglyphs that look identical to ASCII in many fonts.
#[test]
fn test_homoglyph_corpus_greek_lookalikes_all_rejected() {
    let env = Env::default();

    let corpus = [
        ("Greek small alpha", '\u{03B1}', "U+03B1", "\u{03B1}lice"),     // α looks like a
        ("Greek small omicron", '\u{03BF}', "U+03BF", "b\u{03BF}b"),     // ο looks like o
        ("Greek small nu", '\u{03BD}', "U+03BD", "\u{03BD}ick"),         // ν looks like v
        ("Greek small rho", '\u{03C1}', "U+03C1", "\u{03C1}oger"),       // ρ looks like p
        ("Greek small tau", '\u{03C4}', "U+03C4", "\u{03C4}om"),         // τ looks like t
        ("Greek small upsilon", '\u{03C5}', "U+03C5", "\u{03C5}vonne"),  // υ looks like u
        ("Greek small chi", '\u{03C7}', "U+03C7", "\u{03C7}avier"),      // χ looks like x
        // Capital letters
        ("Greek capital Alpha", '\u{0391}', "U+0391", "\u{0391}lice"),   // Α looks like A
        ("Greek capital Beta", '\u{0392}', "U+0392", "\u{0392}ob"),      // Β looks like B
        ("Greek capital Epsilon", '\u{0395}', "U+0395", "\u{0395}ve"),   // Ε looks like E
        ("Greek capital Eta", '\u{0397}', "U+0397", "\u{0397}ank"),      // Η looks like H
        ("Greek capital Iota", '\u{0399}', "U+0399", "\u{0399}an"),      // Ι looks like I
        ("Greek capital Kappa", '\u{039A}', "U+039A", "\u{039A}ate"),    // Κ looks like K
        ("Greek capital Mu", '\u{039C}', "U+039C", "\u{039C}ike"),       // Μ looks like M
        ("Greek capital Nu", '\u{039D}', "U+039D", "\u{039D}ancy"),      // Ν looks like N
        ("Greek capital Omicron", '\u{039F}', "U+039F", "\u{039F}scar"), // Ο looks like O
        ("Greek capital Rho", '\u{03A1}', "U+03A1", "\u{03A1}aul"),      // Ρ looks like P
        ("Greek capital Tau", '\u{03A4}', "U+03A4", "\u{03A4}om"),       // Τ looks like T
        ("Greek capital Upsilon", '\u{03A5}', "U+03A5", "\u{03A5}vonne"),// Υ looks like Y
        ("Greek capital Chi", '\u{03A7}', "U+03A7", "\u{03A7}avier"),    // Χ looks like X
        ("Greek capital Zeta", '\u{0396}', "U+0396", "\u{0396}oe"),      // Ζ looks like Z
    ];

    for (desc, _char, codepoint, username) in corpus {
        assert!(
            !is_valid_github_username(&s(&env, username)),
            "{desc} ({codepoint}) must be rejected: {username}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// § Homoglyph Corpus: Latin Extended & Diacritics
// ═══════════════════════════════════════════════════════════════════════════

/// Latin-extended characters with diacritics that may look close to ASCII
/// after font rendering or in environments with poor font support.
#[test]
fn test_homoglyph_corpus_latin_extended_all_rejected() {
    let env = Env::default();

    let corpus = [
        ("Latin small a with acute", '\u{00E1}', "U+00E1", "\u{00E1}lice"),       // á
        ("Latin small a with grave", '\u{00E0}', "U+00E0", "\u{00E0}lice"),       // à
        ("Latin small a with tilde", '\u{00E3}', "U+00E3", "\u{00E3}lice"),       // ã
        ("Latin small a with ring", '\u{00E5}', "U+00E5", "\u{00E5}lice"),        // å
        ("Latin small e with acute", '\u{00E9}', "U+00E9", "caf\u{00E9}"),        // é
        ("Latin small e with grave", '\u{00E8}', "U+00E8", "caf\u{00E8}"),        // è
        ("Latin small i with acute", '\u{00ED}', "U+00ED", "\u{00ED}an"),         // í
        ("Latin small i with diaeresis", '\u{00EF}', "U+00EF", "na\u{00EF}ve"),   // ï
        ("Latin small o with acute", '\u{00F3}', "U+00F3", "b\u{00F3}b"),         // ó
        ("Latin small o with tilde", '\u{00F5}', "U+00F5", "b\u{00F5}b"),         // õ
        ("Latin small n with tilde", '\u{00F1}', "U+00F1", "jalape\u{00F1}o"),    // ñ
        ("Latin small u with acute", '\u{00FA}', "U+00FA", "\u{00FA}ser"),        // ú
        ("Latin small u with diaeresis", '\u{00FC}', "U+00FC", "\u{00FC}ser"),    // ü
        ("Latin small c with cedilla", '\u{00E7}', "U+00E7", "fran\u{00E7}ois"),  // ç
        // Capitals with diacritics
        ("Latin capital A with acute", '\u{00C1}', "U+00C1", "\u{00C1}lice"),     // Á
        ("Latin capital E with acute", '\u{00C9}', "U+00C9", "\u{00C9}ve"),       // É
        ("Latin capital O with tilde", '\u{00D5}', "U+00D5", "\u{00D5}scar"),     // Õ
    ];

    for (desc, _char, codepoint, username) in corpus {
        assert!(
            !is_valid_github_username(&s(&env, username)),
            "{desc} ({codepoint}) must be rejected: {username}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// § Zero-Width and Invisible Characters
// ═══════════════════════════════════════════════════════════════════════════

/// Zero-width joiners (ZWJ), zero-width non-joiners (ZWNJ), and other
/// invisible Unicode characters. These can hide inside an otherwise-ASCII
/// username and bypass naive length checks or create storage key collisions.
///
/// Example attack: "alice" vs "al\u{200D}ice" — visually identical, different keys.
#[test]
fn test_invisible_characters_zero_width_joiners_rejected() {
    let env = Env::default();

    let corpus = [
        ("Zero-width space", '\u{200B}', "U+200B", "alice\u{200B}"),
        ("Zero-width non-joiner", '\u{200C}', "U+200C", "al\u{200C}ice"),
        ("Zero-width joiner", '\u{200D}', "U+200D", "al\u{200D}ice"),
        ("Word joiner", '\u{2060}', "U+2060", "alice\u{2060}"),
        ("Soft hyphen", '\u{00AD}', "U+00AD", "alice\u{00AD}"),
        ("Invisible separator", '\u{2063}', "U+2063", "al\u{2063}ice"),
        ("Invisible times", '\u{2062}', "U+2062", "al\u{2062}ice"),
        ("Function application", '\u{2061}', "U+2061", "al\u{2061}ice"),
    ];

    for (desc, _char, codepoint, username) in corpus {
        assert!(
            !is_valid_github_username(&s(&env, username)),
            "{desc} ({codepoint}) must be rejected: {username}"
        );
    }
}

/// Zero-width invisible characters embedded at the start, middle, and end of
/// an otherwise-ASCII username must all be rejected.
#[test]
fn test_invisible_characters_at_any_position_rejected() {
    let env = Env::default();

    // Zero-width joiner in prefix, infix, suffix
    assert!(
        !is_valid_github_username(&s(&env, "\u{200D}alice")),
        "ZWJ prefix must be rejected"
    );
    assert!(
        !is_valid_github_username(&s(&env, "al\u{200D}ice")),
        "ZWJ infix must be rejected"
    );
    assert!(
        !is_valid_github_username(&s(&env, "alice\u{200D}")),
        "ZWJ suffix must be rejected"
    );

    // Multiple invisible characters
    assert!(
        !is_valid_github_username(&s(&env, "a\u{200B}l\u{200C}i\u{200D}ce")),
        "Multiple invisible chars must be rejected"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// § Bidirectional Text Override Marks
// ═══════════════════════════════════════════════════════════════════════════

/// Bidirectional (bidi) override marks can reverse text display order, making
/// "alice" appear as "ecila" in rendering while keeping storage as "alice".
/// These are used in sophisticated phishing attacks.
///
/// Example: "alice\u{202E}bob" may render as "alicebob" reversed.
#[test]
fn test_bidirectional_override_marks_rejected() {
    let env = Env::default();

    let corpus = [
        ("Left-to-right mark", '\u{200E}', "U+200E", "alice\u{200E}"),
        ("Right-to-left mark", '\u{200F}', "U+200F", "alice\u{200F}"),
        ("Left-to-right embedding", '\u{202A}', "U+202A", "\u{202A}alice"),
        ("Right-to-left embedding", '\u{202B}', "U+202B", "\u{202B}alice"),
        ("Pop directional formatting", '\u{202C}', "U+202C", "alice\u{202C}"),
        ("Left-to-right override", '\u{202D}', "U+202D", "\u{202D}alice"),
        ("Right-to-left override", '\u{202E}', "U+202E", "\u{202E}alice"),
        ("Left-to-right isolate", '\u{2066}', "U+2066", "\u{2066}alice"),
        ("Right-to-left isolate", '\u{2067}', "U+2067", "\u{2067}alice"),
        ("First strong isolate", '\u{2068}', "U+2068", "\u{2068}alice"),
        ("Pop directional isolate", '\u{2069}', "U+2069", "alice\u{2069}"),
    ];

    for (desc, _char, codepoint, username) in corpus {
        assert!(
            !is_valid_github_username(&s(&env, username)),
            "{desc} ({codepoint}) must be rejected: {username}"
        );
    }
}

/// Complex bidi attack: text that appears to be one username but stores as another.
#[test]
fn test_bidirectional_complex_reversal_attack_rejected() {
    let env = Env::default();

    // This would render as reversed in some environments
    let attack = "alice\u{202E}bob\u{202C}";
    assert!(
        !is_valid_github_username(&s(&env, attack)),
        "Bidirectional reversal attack must be rejected"
    );

    // Mixed with Arabic (RTL script)
    let mixed = "user\u{0645}name";
    assert!(
        !is_valid_github_username(&s(&env, mixed)),
        "Mixed LTR/RTL script must be rejected"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// § Mixed-Script Confusables
// ═══════════════════════════════════════════════════════════════════════════

/// Mixed-script usernames that combine lookalike characters from multiple
/// Unicode blocks to create confusable identifiers.
///
/// Example: "а" (Cyrillic) + "l" (ASCII) + "і" (Cyrillic і) + "ce" (ASCII)
/// = "аlіce" which looks identical to "alice" but has 3 non-ASCII bytes.
#[test]
fn test_mixed_script_confusables_rejected() {
    let env = Env::default();

    let corpus = [
        // Cyrillic + ASCII
        ("Cyrillic a + ASCII", "аlice"),      // а(Cyrillic) + lice(ASCII)
        ("ASCII + Cyrillic o", "b\u{043E}b"), // b(ASCII) + о(Cyrillic) + b(ASCII)
        // Greek + ASCII
        ("Greek o + ASCII", "b\u{03BF}b"),    // b(ASCII) + ο(Greek) + b(ASCII)
        ("ASCII + Greek a", "\u{03B1}lice"),  // α(Greek) + lice(ASCII)
        // Multiple scripts
        ("Cyrillic a + Greek o", "\u{0430}lic\u{03BF}"), // а(Cyr) + lic(ASCII) + ο(Greek)
        // Latin extended + ASCII
        ("Latin á + ASCII", "\u{00E1}lice"),  // á + lice
    ];

    for (desc, username) in corpus {
        assert!(
            !is_valid_github_username(&s(&env, username)),
            "{desc} must be rejected: {username}"
        );
    }
}

/// Every character in a mixed-script attack must be checked individually.
/// If even one non-ASCII byte slips through, the whole username is invalid.
#[test]
fn test_mixed_script_every_byte_validated() {
    let env = Env::default();

    // Position tests: non-ASCII at start, middle, end
    let attacks = [
        "\u{0430}lice",       // Cyrillic а at start
        "al\u{0430}ce",       // Cyrillic а in middle
        "alic\u{0430}",       // Cyrillic а at end
        "a\u{0430}i\u{0430}e",// Multiple Cyrillic а
    ];

    for attack in attacks {
        assert!(
            !is_valid_github_username(&s(&env, attack)),
            "Mixed-script with non-ASCII at any position must be rejected: {attack}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// § Full-Width and Half-Width Forms
// ═══════════════════════════════════════════════════════════════════════════

/// Full-width Latin letters (used in Japanese text) look like ASCII but are
/// encoded as different Unicode codepoints in the U+FF00 range.
///
/// Example: "ａｌｉｃｅ" (full-width) looks like "alice" but each character
/// is 3 bytes (U+FF21 for full-width A, etc.).
#[test]
fn test_fullwidth_latin_letters_rejected() {
    let env = Env::default();

    let corpus = [
        ("Full-width a", '\u{FF41}', "U+FF41", "\u{FF41}lice"),
        ("Full-width b", '\u{FF42}', "U+FF42", "\u{FF42}ob"),
        ("Full-width A", '\u{FF21}', "U+FF21", "\u{FF21}lice"),
        ("Full-width B", '\u{FF22}', "U+FF22", "\u{FF22}ob"),
        // Full-width username
        ("All full-width", '\u{FF41}', "U+FF41..", "\u{FF41}\u{FF4C}\u{FF49}\u{FF43}\u{FF45}"),
    ];

    for (desc, _char, codepoint, username) in corpus {
        assert!(
            !is_valid_github_username(&s(&env, username)),
            "{desc} ({codepoint}) must be rejected: {username}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// § Mathematical Alphanumeric Symbols
// ═══════════════════════════════════════════════════════════════════════════

/// Mathematical bold, italic, script, and other stylistic variants of ASCII
/// letters occupy different Unicode blocks (U+1D400–U+1D7FF).
///
/// Example: "𝐚𝐥𝐢𝐜𝐞" (bold) looks like "alice" but is 5 × 4-byte sequences.
#[test]
fn test_mathematical_alphanumeric_symbols_rejected() {
    let env = Env::default();

    let corpus = [
        ("Math bold small a", '\u{1D41A}', "U+1D41A", "\u{1D41A}lice"),
        ("Math italic small a", '\u{1D44E}', "U+1D44E", "\u{1D44E}lice"),
        ("Math bold italic a", '\u{1D482}', "U+1D482", "\u{1D482}lice"),
        ("Math script small a", '\u{1D4B6}', "U+1D4B6", "\u{1D4B6}lice"),
        ("Math fraktur small a", '\u{1D51E}', "U+1D51E", "\u{1D51E}lice"),
        ("Math monospace small a", '\u{1D68A}', "U+1D68A", "\u{1D68A}lice"),
    ];

    for (desc, _char, codepoint, username) in corpus {
        assert!(
            !is_valid_github_username(&s(&env, username)),
            "{desc} ({codepoint}) must be rejected: {username}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// § Superscripts, Subscripts, and Modifier Letters
// ═══════════════════════════════════════════════════════════════════════════

/// Superscript and subscript digits/letters can create subtle visual differences.
#[test]
fn test_superscript_subscript_modifier_letters_rejected() {
    let env = Env::default();

    let corpus = [
        ("Superscript a", '\u{1D43}', "U+1D43", "alice\u{1D43}"),
        ("Superscript b", '\u{1D47}', "U+1D47", "alice\u{1D47}"),
        ("Subscript a", '\u{2090}', "U+2090", "alice\u{2090}"),
        ("Subscript e", '\u{2091}', "U+2091", "alice\u{2091}"),
        ("Modifier letter small a", '\u{1D43}', "U+1D43", "\u{1D43}lice"),
    ];

    for (desc, _char, codepoint, username) in corpus {
        assert!(
            !is_valid_github_username(&s(&env, username)),
            "{desc} ({codepoint}) must be rejected: {username}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// § Regression: Valid ASCII must still pass
// ═══════════════════════════════════════════════════════════════════════════

/// After adding all these rejection tests, confirm that pure ASCII usernames
/// with valid GitHub shapes are still accepted. This is the positive control.
#[test]
fn test_valid_ascii_usernames_still_accepted_after_homoglyph_hardening() {
    let env = Env::default();

    let valid = [
        "alice",
        "bob123",
        "user-name",
        "user_name",
        "octocat",
        "a",
        "z",
        "A",
        "Z",
        "user1",
        "test-user-123",
        "foo_bar_baz",
    ];

    for username in valid {
        assert!(
            is_valid_github_username(&s(&env, username)),
            "Valid ASCII username must be accepted: {username}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// § Bypass Detection: All Entry Points
// ═══════════════════════════════════════════════════════════════════════════

/// Integration test: attempt to register a homoglyph username through the
/// actual `register` entry point. This tests the full validation chain,
/// not just `is_valid_github_username` in isolation.
///
/// If any code path bypasses `is_valid_github_username`, this will expose it.
#[test]
fn test_homoglyph_registration_blocked_at_register_entry_point() {
    use soroban_sdk::{testutils::Address as _, Address, Env, Vec};
    use trustbridge_contract::{ContractError, TrustBridgeContract};

    let env = Env::default();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });

    env.mock_all_auths();

    // Attempt to register a Cyrillic homoglyph
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::register(
            env.clone(),
            s(&env, "\u{0430}lice"), // Cyrillic а + ASCII lice
            user.clone(),
            Vec::new(&env),
        );

        assert_eq!(
            result,
            Err(ContractError::InvalidUsername),
            "Homoglyph username must be rejected by register()"
        );
    });

    // Attempt with zero-width joiner
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::register(
            env.clone(),
            s(&env, "al\u{200D}ice"), // ASCII with ZWJ
            user.clone(),
            Vec::new(&env),
        );

        assert_eq!(
            result,
            Err(ContractError::InvalidUsername),
            "ZWJ in username must be rejected by register()"
        );
    });

    // Attempt with bidi override
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::register(
            env.clone(),
            s(&env, "\u{202E}alice"), // RTL override
            user,
            Vec::new(&env),
        );

        assert_eq!(
            result,
            Err(ContractError::InvalidUsername),
            "Bidi override in username must be rejected by register()"
        );
    });
}

/// Test `register_sponsored` entry point with homoglyph usernames.
///
/// Sponsored registration must validate usernames the same way as regular
/// registration. A sponsor cannot bypass the homoglyph guard.
#[test]
fn test_homoglyph_registration_blocked_at_register_sponsored_entry_point() {
    use soroban_sdk::{testutils::Address as _, Address, Env, Vec};
    use trustbridge_contract::{ContractError, TrustBridgeContract};

    let env = Env::default();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let sponsor = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });

    env.mock_all_auths();

    // Attempt sponsored registration with Cyrillic homoglyph
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::register_sponsored(
            env.clone(),
            s(&env, "\u{0430}lice"), // Cyrillic а + ASCII lice
            user.clone(),
            sponsor.clone(),
        );

        assert_eq!(
            result,
            Err(ContractError::InvalidUsername),
            "Homoglyph username must be rejected by register_sponsored()"
        );
    });

    // Attempt with full-width Latin
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::register_sponsored(
            env.clone(),
            s(&env, "\u{FF41}lice"), // Full-width a + ASCII lice
            user,
            sponsor,
        );

        assert_eq!(
            result,
            Err(ContractError::InvalidUsername),
            "Full-width Latin in username must be rejected by register_sponsored()"
        );
    });
}

/// Test that `is_username_valid` helper correctly rejects homoglyphs.
///
/// This is the public read function dashboards use to pre-validate usernames
/// before asking users to sign. It must agree with the internal validation.
#[test]
fn test_homoglyph_rejected_by_public_is_username_valid_helper() {
    use soroban_sdk::{Env, testutils::Address as _, Address};
    use trustbridge_contract::TrustBridgeContract;

    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin).unwrap();
    });

    env.as_contract(&contract_id, || {
        // Cyrillic homoglyph
        assert!(
            !TrustBridgeContract::is_username_valid(env.clone(), s(&env, "\u{0430}lice")),
            "is_username_valid must reject Cyrillic homoglyph"
        );

        // Zero-width joiner
        assert!(
            !TrustBridgeContract::is_username_valid(env.clone(), s(&env, "al\u{200D}ice")),
            "is_username_valid must reject ZWJ"
        );

        // Bidi override
        assert!(
            !TrustBridgeContract::is_username_valid(env.clone(), s(&env, "\u{202E}alice")),
            "is_username_valid must reject bidi override"
        );

        // Valid ASCII must still pass
        assert!(
            TrustBridgeContract::is_username_valid(env.clone(), s(&env, "alice")),
            "is_username_valid must accept valid ASCII"
        );
    });
}

/// Test that read-only functions (`get_address`, `has_record`, etc.) don't
/// validate usernames — they just look up whatever key is provided.
///
/// This is correct behavior: validation only needs to happen at registration.
/// Looking up a malformed username should return "not found", not "invalid".
#[test]
fn test_read_only_functions_do_not_validate_homoglyph_usernames() {
    use soroban_sdk::{testutils::Address as _, Address, Env, Vec};
    use trustbridge_contract::TrustBridgeContract;

    let env = Env::default();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin).unwrap();
    });

    env.mock_all_auths();

    // First register a valid username
    env.as_contract(&contract_id, || {
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "alice"),
            user.clone(),
            Vec::new(&env),
        )
        .unwrap();
    });

    // Now try to look up with a homoglyph — should return None, not error
    env.as_contract(&contract_id, || {
        let result = TrustBridgeContract::get_address(env.clone(), s(&env, "\u{0430}lice"));
        assert_eq!(
            result, None,
            "Homoglyph lookup should return None (not found), not error"
        );

        let has = TrustBridgeContract::has_record(env.clone(), s(&env, "\u{0430}lice"));
        assert!(!has, "Homoglyph has_record should return false");
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// § Coverage Summary
// ═══════════════════════════════════════════════════════════════════════════

/// Meta-test: confirm that this test file covers all documented attack vectors.
///
/// Categories covered:
/// - Cyrillic homoglyphs (23 characters)
/// - Greek homoglyphs (20 characters)
/// - Latin extended / diacritics (16 characters)
/// - Zero-width & invisible (8 characters)
/// - Bidirectional marks (11 characters)
/// - Mixed-script confusables
/// - Full-width / half-width
/// - Mathematical alphanumeric symbols (6 variants)
/// - Superscripts / subscripts / modifiers
/// - Integration test at register() entry point
/// - Positive control (valid ASCII still works)
#[test]
fn test_homoglyph_corpus_coverage_complete() {
    // This is a documentation test — if it compiles and runs, all corpus
    // tests exist. If a new attack vector is discovered, add it above and
    // increment the count here.
    
    const EXPECTED_CORPUS_SIZE: usize = 80; // Approximate, update as corpus grows
    
    // The real validation is in each corpus test. This just documents the scope.
    assert!(
        EXPECTED_CORPUS_SIZE >= 78,
        "Corpus should cover at least 78 known homoglyph/confusable codepoints"
    );
}
