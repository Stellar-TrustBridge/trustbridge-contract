# Storage Key Audit

This document inventories every persistent and instance storage key used by the trustbridge-contract registry, proving that key namespaces cannot collide across records, indexes, and counters.

It should be updated whenever a new storage key is introduced (e.g. new enum variant, new feature state).

Related docs: [ARCHITECTURE](ARCHITECTURE.md) · [ABI](ABI.md) · [SECURITY](SECURITY.md)

---

## Instance Storage (per contract instance)

These keys live in Soroban's **instance** storage partition. Each key is a `Symbol` mapped to a single value.

| Key Symbol | Constant | Type | Purpose |
|---|---|---|---|
| `"admin"` | `ADMIN_KEY` | `Address` | Contract administrator address set once during `initialize` |
| `"count"` | `COUNT_KEY` | `u32` | Total number of active registrations (incremented on register, decremented on remove) |
| `"vcount"` | `VCOUNT_KEY` | `u32` | Count of verified registrations (incremented on verify, decremented on revoke or address-change re-reg) |
| `"idx"` | `INDEX_KEY` | `Vec<String>` | Ordered list of all registered usernames used for admin export |
| `"pause"` | `PAUSED_KEY` | `bool` | Pauses all state-mutating operations when set to `true` |
| `"cdown"` | `COOLDOWN_KEY` | `u64` | WASM upgrade timelock cooldown period in seconds |
| `"lastupg"` | `LAST_UPG_KEY` | `u64` | Ledger timestamp of the most recent upgrade |
| `"ver"` | `VER_KEY` / `VERSION_KEY` | `(u32, u32, u32)` | Contract version tuple `(major, minor, patch)` set during `initialize` |
| `"chkcnt"` | `CHUNK_CNT_KEY` | `u32` | Number of chunks in the chunked username index |

### Reserved / future instance keys

| Key Symbol | Constant | Reserved For |
|---|---|---|
| `"role"` | `ROLE_KEY` | Persistent role map key prefix (see below); not used directly as an instance key |
| `"chunk"` | `CHUNK_KEY` | Persistent chunk index key prefix (see below); not used directly as an instance key |
| `"lastact"` | `LAST_ACT_KEY` | Per-user cooldown tracking key prefix (see below); not used directly as an instance key |

---

## Persistent Storage (per-entry, TTL-extended)

These keys use **tuples** as storage keys, where the first element is a `Symbol` discriminator and the second is a context value. Soroban serializes tuples so that different discriminator symbols can never produce the same storage key even if the context value overlaps.

### Key namespace mapping

| Discriminator Symbol | Constant | Context Type | Value Type | Purpose |
|---|---|---|---|---|
| `"reg"` | `REG_KEY` | `String` (GitHub username) | `ContributorRecord` | Per-user registration record (stellar_address, registered_at, verified) |
| `"role"` | `ROLE_KEY` | `Address` | `Role` (enum u32) | Role assignment per address (Admin, Upgrader, Verifier) |
| `"chunk"` | `CHUNK_KEY` | `u32` (chunk index) | `Vec<String>` | Chunked slice of the username index for paginated reads |
| `"lastact"` | `LAST_ACT_KEY` | `String` (GitHub username) | `u64` (ledger timestamp) | Per-user cooldown tracking for rate-limiting |
| `"lastact"` | `LAST_ACT_KEY` | `String` (GitHub username) | `u64` | Also used for the non-per-chunk cooldown getter (see note below) |

### TTL behavior

| Key namespace | TTL threshold | TTL bump | Notes |
|---|---|---|---|
| `"reg"` records | 100 000 ledgers (~3 days) | 1 000 000 ledgers (~60 days) | Extended on read (`get_record`, `extend_record_ttl`) and on write (`set_record`) |
| `"chunk"` records | 100 000 ledgers (~3 days) | 1 000 000 ledgers (~60 days) | Extended on read and on write |
| `"lastact"` records | 100 000 ledgers (~3 days) | 1 000 000 ledgers (~60 days) | Extended on write (`set_last_action`) |
| Instance keys | N/A | N/A | Instance storage entries do not have TTL |

---

## Collision Analysis

No overlapping key encodings were identified. The following properties prevent collisions:

1. **Instance vs. persistent partition**: Soroban separates instance storage from persistent storage. Keys in one partition cannot collide with keys in the other, even if the symbol value is identical.

2. **Tuple discriminator uniqueness**: The four persistent key namespaces (`"reg"`, `"role"`, `"chunk"`, `"lastact"`) all use distinct `Symbol` discriminators. Even though `"chunk"` and `"lastact"` share the same context type (`String`) for part of their key, the discriminator is encoded first in the tuple serialization, making collisions impossible.

3. **Context type distinction**: Within the `"role"` namespace, the context is `Address` (32 bytes). Within the `"reg"` namespace, the context is `String`. These different types produce different serialized byte sequences for the same logical input.

4. **Duplicate constant definitions**: `src/storage.rs` contains duplicate definitions for `CHUNK_KEY`, `CHUNK_CNT_KEY`, and `LAST_ACT_KEY` using different symbol strings on the second definition. The later definitions shadow the earlier ones at the Rust constant level. The runtime values in use are:
   - `CHUNK_KEY` = `symbol_short!("chunk")` (both definitions agree)
   - `CHUNK_CNT_KEY` = `symbol_short!("chkcnt")` (the second definition at line 43, differing from the first `symbol_short!("chunkcnt")` at line 18)
   - `LAST_ACT_KEY` = `symbol_short!("lastact")` (both definitions agree)

   The `CHUNK_CNT_KEY` discrepancy (`"chunkcnt"` vs `"chkcnt"`) is non-colliding but should be resolved — only the second definition (`"chkcnt"`) takes effect at runtime.

---

## Update Procedure

When adding a new storage key:

1. Add a row to the appropriate table above.
2. If the key uses a new discriminator symbol, verify it does not match any existing persistent discriminator.
3. If the key uses a new context type, verify it cannot serialize to the same bytes as an existing context type for any valid input.
4. Update this document and link it from [ARCHITECTURE.md](ARCHITECTURE.md) and [ABI.md](ABI.md).