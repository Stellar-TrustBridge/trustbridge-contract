//! Staged-WASM slot (Issue #300).
//!
//! Separates the "intent to upgrade" from the upgrade itself.  The admin
//! (or any holder of `Role::Upgrader`) may stage a WASM hash with
//! `stage_wasm(hash)` at any time.  The slot is publicly readable via
//! `get_staged`.  `upgrade` enforces that, **when a staged hash is present**,
//! the hash passed to `upgrade` must match it — preventing a last-second
//! swap to a different binary.  Clearing (`clear_staged`) is available to the
//! admin at any time.
//!
//! ## Relationship to the existing attestation slot
//!
//! `WasmAttestation` (Issue #198) is the admin's *time-bounded* on-chain
//! declaration, optionally required by `set_attestation_required`.  The staged
//! slot is a *permanent* public notice — no expiry, no required-flag — that
//! lives alongside it.  Both can be present simultaneously; `upgrade` checks
//! the staged slot first (fail-fast on mismatch), then proceeds to the
//! attestation check.
//!
//! ## Storage
//!
//! The staged hash is stored in instance storage under `STAGED_WASM_KEY`.
//! Absent = nothing staged; present = one `BytesN<32>`.

use soroban_sdk::{contracttype, symbol_short, Address, BytesN, Env, Symbol};

use crate::ContractError;

// ── Storage key ──────────────────────────────────────────────────────────────

/// Instance-storage key for the currently staged WASM hash.
///
/// Absent = nothing staged.  Present = `BytesN<32>` WASM hash.
pub const STAGED_WASM_KEY: Symbol = symbol_short!("stgwasm");

// ── Types ────────────────────────────────────────────────────────────────────

/// The currently staged WASM hash together with its provenance metadata.
///
/// Publicly readable via `get_staged`; no admin auth required to read.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct StagedWasm {
    /// Hash the staging operator intends to deploy next.
    pub wasm_hash: BytesN<32>,
    /// Address that called `stage_wasm`.
    pub staged_by: Address,
    /// Ledger timestamp `stage_wasm` was called.
    pub staged_at: u64,
}

// ── Storage helpers ───────────────────────────────────────────────────────────

/// Returns the currently staged WASM entry, or `None` if nothing is staged.
#[must_use]
pub fn get_staged_wasm(env: &Env) -> Option<StagedWasm> {
    env.storage().instance().get(&STAGED_WASM_KEY)
}

/// Writes a new staged WASM entry, replacing any existing one.
pub fn set_staged_wasm(env: &Env, entry: &StagedWasm) {
    env.storage().instance().set(&STAGED_WASM_KEY, entry);
}

/// Removes the staged WASM entry (if any).
pub fn clear_staged_wasm(env: &Env) {
    env.storage().instance().remove(&STAGED_WASM_KEY);
}

// ── Business logic ────────────────────────────────────────────────────────────

/// Validates that `wasm_hash` is consistent with the staged slot.
///
/// If nothing is staged, the check is a no-op — staging is advisory, not
/// mandatory (use `set_attestation_required` for a hard gate).  If a hash is
/// staged, the supplied hash must match; a mismatch fails with
/// [`ContractError::StagedWasmMismatch`].
///
/// # Errors
///
/// - [`ContractError::StagedWasmMismatch`] when a staged hash exists and
///   differs from `wasm_hash`.
pub fn require_staged_wasm_consistent(
    env: &Env,
    wasm_hash: &BytesN<32>,
) -> Result<(), ContractError> {
    if let Some(staged) = get_staged_wasm(env) {
        if staged.wasm_hash != *wasm_hash {
            return Err(ContractError::StagedWasmMismatch);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _},
        Address, BytesN, Env,
    };

    fn make_hash(env: &Env, byte: u8) -> BytesN<32> {
        BytesN::from_array(env, &[byte; 32])
    }

    /// Nothing staged → consistent with any hash.
    #[test]
    fn test_no_staged_allows_any_hash() {
        let env = Env::default();
        let hash = make_hash(&env, 0xAB);
        assert!(require_staged_wasm_consistent(&env, &hash).is_ok());
    }

    /// Staged hash matches → ok.
    #[test]
    fn test_matching_staged_hash_is_ok() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let hash = make_hash(&env, 0x01);
        env.ledger().set_timestamp(1_000);
        let entry = StagedWasm {
            wasm_hash: hash.clone(),
            staged_by: admin,
            staged_at: env.ledger().timestamp(),
        };
        set_staged_wasm(&env, &entry);
        assert!(require_staged_wasm_consistent(&env, &hash).is_ok());
    }

    /// Staged hash differs → StagedWasmMismatch.
    #[test]
    fn test_mismatched_staged_hash_is_rejected() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let staged_hash = make_hash(&env, 0x01);
        let other_hash = make_hash(&env, 0x02);
        env.ledger().set_timestamp(1_000);
        let entry = StagedWasm {
            wasm_hash: staged_hash,
            staged_by: admin,
            staged_at: env.ledger().timestamp(),
        };
        set_staged_wasm(&env, &entry);
        assert_eq!(
            require_staged_wasm_consistent(&env, &other_hash),
            Err(ContractError::StagedWasmMismatch)
        );
    }

    /// clear_staged_wasm removes the slot; subsequent check passes.
    #[test]
    fn test_clear_staged_wasm_removes_slot() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let hash = make_hash(&env, 0x03);
        env.ledger().set_timestamp(2_000);
        let entry = StagedWasm {
            wasm_hash: hash.clone(),
            staged_by: admin,
            staged_at: env.ledger().timestamp(),
        };
        set_staged_wasm(&env, &entry);
        assert!(get_staged_wasm(&env).is_some());
        clear_staged_wasm(&env);
        assert!(get_staged_wasm(&env).is_none());
        // After clearing, any hash is accepted.
        let other = make_hash(&env, 0xFF);
        assert!(require_staged_wasm_consistent(&env, &other).is_ok());
    }

    /// Re-staging overwrites the previous entry.
    #[test]
    fn test_restage_overwrites_previous() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let hash_a = make_hash(&env, 0x0A);
        let hash_b = make_hash(&env, 0x0B);
        env.ledger().set_timestamp(3_000);
        set_staged_wasm(
            &env,
            &StagedWasm {
                wasm_hash: hash_a,
                staged_by: admin.clone(),
                staged_at: env.ledger().timestamp(),
            },
        );
        env.ledger().set_timestamp(4_000);
        set_staged_wasm(
            &env,
            &StagedWasm {
                wasm_hash: hash_b.clone(),
                staged_by: admin,
                staged_at: env.ledger().timestamp(),
            },
        );
        let current = get_staged_wasm(&env).unwrap();
        assert_eq!(current.wasm_hash, hash_b);
        assert_eq!(current.staged_at, 4_000);
    }

    /// get_staged_wasm returns None when nothing has ever been staged.
    #[test]
    fn test_get_staged_wasm_returns_none_initially() {
        let env = Env::default();
        assert!(get_staged_wasm(&env).is_none());
    }

    /// The StagedWasm fields round-trip correctly through storage.
    #[test]
    fn test_staged_wasm_fields_round_trip() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let hash = make_hash(&env, 0x55);
        env.ledger().set_timestamp(9_999);
        let entry = StagedWasm {
            wasm_hash: hash.clone(),
            staged_by: admin.clone(),
            staged_at: 9_999,
        };
        set_staged_wasm(&env, &entry);
        let retrieved = get_staged_wasm(&env).unwrap();
        assert_eq!(retrieved.wasm_hash, hash);
        assert_eq!(retrieved.staged_by, admin);
        assert_eq!(retrieved.staged_at, 9_999);
    }
}
