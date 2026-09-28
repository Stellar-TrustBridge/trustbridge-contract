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
| `"evcount"` | `EVER_VCOUNT_KEY` | `u32` | Monotonic count of verifications ever granted (never decreases, Issue #229) |
| `"idx"` | `INDEX_KEY` | `Vec<String>` | Ordered list of all registered usernames used for admin export |
| `"orgidx"` | `ORG_INDEX_KEY` | `Vec<String>` | Ordered list of organization names |
| `"tmidx"` | `TEAM_INDEX_KEY` | `Vec<String>` | Ordered list of team keys |
| `"pause"` | `PAUSED_KEY` | `bool` | Pauses all state-mutating operations when set to `true` |
| `"pause_rsn"` | `PAUSE_RSN_KEY` | `u32` | Last recorded `PauseReason` discriminant (Issue #211) |
| `"cdown"` | `COOLDOWN_KEY` | `u64` | WASM upgrade timelock cooldown period in seconds |
| `"rotdelay"` | `ROT_DELAY_KEY` | `u64` | Address rotation delay in seconds (Issue #234) |
| `"emrg_ps"` | `EMERGENCY_PAUSE_KEY` | `bool` | Emergency pause flag set by guardian (Issue #196) |
| `"emerg_ts"` | `EMERGENCY_PAUSE_TS_KEY` | `u64` | Timestamp when emergency pause was activated |
| `"guardian"` | `GUARDIAN_KEY` | `Option<Address>` | Guardian address for emergency pause (Issue #196) |
| `"lastupg"` | `LAST_UPG_KEY` | `u64` | Ledger timestamp of the most recent WASM upgrade |
| `"ver"` | `VER_KEY` / `VERSION_KEY` | `(u32, u32, u32)` | Contract version tuple `(major, minor, patch)` set during `initialize` |
| `"network"` | `NETWORK_KEY` | `BytesN<32>` | Network ID (SHA-256 of passphrase) recorded at `initialize` (Issue #231) |
| `"roldelay"` | `ROLE_DELAY_KEY` | `u64` | Timelock delay for role grants in seconds (Issue #220) |
| `"role_idx"` | `ROLE_IDX_KEY` | `Vec<Address>` | Enumeration index of addresses holding roles (Issue #228) |
| `"chkcnt"` | `CHUNK_CNT_KEY` | `u32` | Number of chunks in the chunked username index |
| `"idx_gen"` | `INDEX_GEN_KEY` | `u32` | Index generation counter for cursor invalidation (Issue #215) |
| `"evt_ledger"` | `LAST_EVENT_LEDGER_KEY` | `u32` | Ledger sequence of most recent contract event (Issue #282) |
| `"prov"` | `PROV_KEY` | `Option<WasmProvenance>` | WASM provenance record (Wave #24) |
| `"attest"` | `ATTEST_KEY` | `Option<WasmAttestation>` | Pending upgrade attestation (Wave #24) |
| `"adt_log"` | `AUDIT_LOG_KEY` | `Vec<AuditLogEntry>` | Audit log entries (Issue #217) |
| `"adt_stat"` | `AUDIT_STATS_KEY` | `AuditStats` | Audit statistics aggregator |
| `"reserved"` | `RESERVED_KEY` | `Vec<String>` | Reserved username list (Issue #213) |
| `"adm_xfr"` | `ADMIN_TRANSFER_KEY` | `Option<AdminTransferProposal>` | Pending admin transfer proposal (Issue #195) |
| `"att_req"` | `ATTEST_REQUIRED_KEY` | `bool` | Whether WASM attestation is required before upgrade (Issue #198) |
| `"brm_thr"` | `BATCH_REMOVE_THRESHOLD_KEY` | `u32` | Dual-control batch_remove size threshold (Issue #219) |
| `"brm_pend"` | `PENDING_BATCH_REMOVE_KEY` | `Option<PendingBatchRemove>` | Pending large batch_remove proposal (Issue #219) |
| `"vfylimit"` | `VERIFY_LIMIT_KEY` | `u32` | Per-ledger verify/revoke rate limit (Issue #292) |
| `"vfyallow"` | `VERIFIER_ALLOWLIST_KEY` | `Vec<VerifierAllowEntry>` | Verifier allowlist with expiry (Issue #293) |

### Not used as direct instance keys (prefixes only)

| Key Symbol | Constant | Note |
|---|---|---|
| `"role"` | `ROLE_KEY` | Persistent key prefix for role assignments; not used as instance key |
| `"chunk"` | `CHUNK_KEY` | Persistent key prefix for chunked index; not used as instance key |
| `"lastact"` | `LAST_ACT_KEY` | Persistent key prefix for per-user cooldown; not used as instance key |

---

## Persistent Storage (per-entry, TTL-extended)

These keys use **tuples** as storage keys, where the first element is a `Symbol` discriminator and the second is a context value. Soroban serializes tuples so that different discriminator symbols can never produce the same storage key even if the context value overlaps.

### Key namespace mapping

| Discriminator Symbol | Constant | Context Type | Value Type | Purpose |
|---|---|---|---|---|
| `"reg"` | `REG_KEY` | `String` (GitHub username) | `ContributorRecord` | Per-user registration record (stellar_address, payout_address, registered_at, verified, is_bot) |
| `"role"` | `ROLE_KEY` | `Address` | `Role` (enum u32) | Role assignment per address (Admin, Upgrader, Verifier, Revoker) |
| `"chunk"` | `CHUNK_KEY` | `u32` (chunk index) | `Vec<String>` | Chunked slice of the username index for paginated reads |
| `"lastact"` | `LAST_ACT_KEY` | `String` (GitHub username) | `u64` (ledger timestamp) | Per-user cooldown tracking for rate-limiting |
| `"pendrot"` | `PENDING_ROT_KEY` | `String` (GitHub username) | `PendingRotation` | Pending address rotation with delay (Issue #234) |
| `"pend_rev"` | `PENDING_REVERIFY_KEY` | `String` (GitHub username) | `bool` | Flag indicating re-verification needed after address change (Issue #208) |
| `"chllng"` | `CHALLENGE_KEY` | `String` (GitHub username) | `ChallengeRecord` | Squatter challenge record (Issue #214) |
| `"pendrole"` | `PENDING_ROLE_KEY` | `Address` | `PendingRoleGrant` | Timelocked role grant awaiting activation (Issue #220) |
| `"role_exp"` | `ROLE_EXPIRY_KEY` | `Address` | `u64` (timestamp) | Expiry timestamp for role grant (Issue #221) |
| `"vfy_at"` | `VERIFIED_AT_KEY` | `String` (GitHub username) | `u64` (timestamp) | Ledger timestamp of last successful `verify()` (Issue #218) |
| `"vfyrate"` | `VERIFY_RATE_KEY` | `Address` | `(u32 ledger_seq, u32 units_spent)` | Per-verifier per-ledger rate limit counter (Issue #292) |

### TTL behavior

| Key namespace | TTL threshold | TTL bump | Notes |
|---|---|---|---|
| `"reg"` records | `LEDGERS_PER_DAY * 30` (~30 days) | `LEDGERS_PER_DAY * 90` (~90 days) | Extended on read (`get_record`, `extend_record_ttl`) and on write (`set_record`) |
| `"chunk"` records | `LEDGERS_PER_DAY * 30` (~30 days) | `LEDGERS_PER_DAY * 90` (~90 days) | Extended on read and on write |
| `"lastact"` records | `LEDGERS_PER_DAY * 30` (~30 days) | `LEDGERS_PER_DAY * 90` (~90 days) | Extended on write (`set_last_action`) |
| `"pendrot"` records | `LEDGERS_PER_DAY * 30` (~30 days) | `LEDGERS_PER_DAY * 90` (~90 days) | Extended on read and write |
| `"pend_rev"` records | `LEDGERS_PER_DAY * 30` (~30 days) | `LEDGERS_PER_DAY * 90` (~90 days) | Extended on write |
| `"chllng"` records | `LEDGERS_PER_DAY * 30` (~30 days) | `LEDGERS_PER_DAY * 90` (~90 days) | Extended on write |
| `"pendrole"` records | `LEDGERS_PER_DAY * 30` (~30 days) | `LEDGERS_PER_DAY * 90` (~90 days) | Extended on write |
| `"role_exp"` records | `LEDGERS_PER_DAY * 30` (~30 days) | `LEDGERS_PER_DAY * 90` (~90 days) | Extended on write |
| `"vfy_at"` records | `LEDGERS_PER_DAY * 30` (~30 days) | `LEDGERS_PER_DAY * 90` (~90 days) | Extended on write |
| `"vfyrate"` records | `LEDGERS_PER_DAY * 30` (~30 days) | `LEDGERS_PER_DAY * 90` (~90 days) | Extended on write |
| Instance keys | N/A | N/A | Instance storage entries do not have TTL |

---

## Collision Analysis

No overlapping key encodings were identified. The following properties prevent collisions:

1. **Instance vs. persistent partition**: Soroban separates instance storage from persistent storage. Keys in one partition cannot collide with keys in the other, even if the symbol value is identical.

2. **Tuple discriminator uniqueness**: The eleven persistent key namespaces (`"reg"`, `"role"`, `"chunk"`, `"lastact"`, `"pendrot"`, `"pend_rev"`, `"chllng"`, `"pendrole"`, `"role_exp"`, `"vfy_at"`, `"vfyrate"`) all use distinct `Symbol` discriminators. Even though multiple namespaces use `String` as context type, the discriminator is encoded first in the tuple serialization, making collisions impossible.

3. **Context type distinction**: Within the `"role"` namespace, the context is `Address` (32 bytes). Within the `"reg"` namespace, the context is `String`. These different types produce different serialized byte sequences for the same logical input.

4. **No duplicate constant definitions**: All constants in `src/storage.rs` are defined exactly once with unique symbol strings.

---

## Complete Key Inventory (linted)

Every `Symbol` constant in `src/storage.rs`, checked by
`scripts/check_storage_keys.sh` (`make storage-keys-check`, run in CI). The
linter fails if a constant or its symbol string is missing from this document,
or if it finds a `Symbol` constant it cannot classify (fail closed). Line
numbers are informational and are not linted.

| Key Symbol | Constant | `storage.rs` line | Purpose |
|---|---|---|---|
| `"vrfy_cfg"` | `VER_CFG_KEY` | 20 | — |
| `"reg"` | `REG_KEY` | 24 | — |
| `"admin"` | `ADMIN_KEY` | 30 | Storage key for the admin address |
| `"count"` | `COUNT_KEY` | 31 | — |
| `"vcount"` | `VCOUNT_KEY` | 32 | — |
| `"evcount"` | `EVER_VCOUNT_KEY` | 36 | Monotonic count of verifications ever granted (Issue #229) |
| `"idx"` | `INDEX_KEY` | 37 | — |
| `"orgidx"` | `ORG_INDEX_KEY` | 38 | — |
| `"tmidx"` | `TEAM_INDEX_KEY` | 39 | — |
| `"pause"` | `PAUSED_KEY` | 49 | — |
| `"pause_rsn"` | `PAUSE_RSN_KEY` | 51 | Last `PauseReason` recorded by `pause` / `unpause`. |
| `"cdown"` | `COOLDOWN_KEY` | 52 | — |
| `"rotdelay"` | `ROT_DELAY_KEY` | 55 | Seconds a requested address rotation must wait before it can execute (Issue #234) |
| `"pendrot"` | `PENDING_ROT_KEY` | 57 | Key prefix for a username's pending address rotation (Issue #234). |
| `"pend_rev"` | `PENDING_REVERIFY_KEY` | 59 | — |
| `"emrg_ps"` | `EMERGENCY_PAUSE_KEY` | 61 | — |
| `"emerg_ts"` | `EMERGENCY_PAUSE_TS_KEY` | 62 | — |
| `"guardian"` | `GUARDIAN_KEY` | 65 | Storage key for the designated guardian address (Issue #196) |
| `"lastupg"` | `LAST_UPG_KEY` | 66 | — |
| `"ver"` | `VER_KEY` | 67 | — |
| `"network"` | `NETWORK_KEY` | 74 | Key for the network id recorded at `initialize` (Issue #231) |
| `"role"` | `ROLE_KEY` | 76 | — |
| `"roldelay"` | `ROLE_DELAY_KEY` | 81 | Seconds a `set_role` grant must wait before it can be activated (Issue #220) |
| `"pendrole"` | `PENDING_ROLE_KEY` | 84 | Key prefix for a pending (timelocked) role grant, namespaced by address. |
| `"role_idx"` | `ROLE_IDX_KEY` | 89 | Key for the enumerable index of addresses that currently hold a role (Issue #228) |
| `"chunk"` | `CHUNK_KEY` | 98 | Key prefix for chunked username index entries. |
| `"chkcnt"` | `CHUNK_CNT_KEY` | 99 | — |
| `"idx_gen"` | `INDEX_GEN_KEY` | 105 | Monotonic counter bumped every time the flat index's existing positions shift — i.e |
| `"lastact"` | `LAST_ACT_KEY` | 106 | — |
| `"evt_ledger"` | `LAST_EVENT_LEDGER_KEY` | 111 | Ledger sequence containing the most recently emitted contract event (Issue #282) |
| `"prov"` | `PROV_KEY` | 113 | Key for the WASM provenance record (Wave #24). |
| `"attest"` | `ATTEST_KEY` | 115 | Key for the pending upgrade attestation (Wave #24). |
| `"adt_log"` | `AUDIT_LOG_KEY` | 117 | Key for audit log entries list. |
| `"adt_stat"` | `AUDIT_STATS_KEY` | 119 | Key for audit stats. |
| `"chllng"` | `CHALLENGE_KEY` | 121 | Key prefix for per-username challenge records (Issue #214). |
| `"pause_rsn"` | `PAUSE_RSN_KEY` / `PAUSE_REASON_KEY` | 127 | Key for the pause reason code (Issue #211). |
| `"reserved"` | `RESERVED_KEY` | 130 | Key for the reserved username set (Issue #213). |
| alias of `VER_KEY` | `VERSION_KEY` | 141 | Key for the version stored at `storage::get_version` / `set_version` |
| `"adm_xfr"` | `ADMIN_TRANSFER_KEY` | 144 | Key for a pending admin transfer proposal (Issue #195). |
| `"att_req"` | `ATTEST_REQUIRED_KEY` | 147 | Key for whether WASM attestation is required before upgrade (Issue #198). |
| `"role_exp"` | `ROLE_EXPIRY_KEY` | 154 | Key prefix for a role assignment's expiry timestamp (Issue #221) |
| `"vfy_at"` | `VERIFIED_AT_KEY` | 158 | Key prefix for the ledger timestamp a username was last `verify()`'d (Issue #218). |
| `"brm_thr"` | `BATCH_REMOVE_THRESHOLD_KEY` | 163 | Key for the `batch_remove` dual-control size threshold (Issue #219) |
| `"brm_pend"` | `PENDING_BATCH_REMOVE_KEY` | 167 | Key for the single pending large-`batch_remove` proposal (Issue #219) |
| `"vfylimit"` | `VERIFY_LIMIT_KEY` | 1481 | Instance key holding the configured per-ledger verify/revoke cap |
| `"vfyrate"` | `VERIFY_RATE_KEY` | 1485 | Persistent key prefix for the per-`(verifier, ledger)` spend counter |
| `"vfyallow"` | `VERIFIER_ALLOWLIST_KEY` | 1800 | Instance key for the verifier allowlist vector (Issue #293). |

---

## Update Procedure

When adding a new storage key:

1. Add a row to the appropriate table above **and** to the Complete Key Inventory.
   Run `make storage-keys-check` (CI fails otherwise).
2. If the key uses a new discriminator symbol, verify it does not match any existing persistent discriminator.
3. If the key uses a new context type, verify it cannot serialize to the same bytes as an existing context type for any valid input.
4. Update this document and link it from [ARCHITECTURE.md](ARCHITECTURE.md) and [ABI.md](ABI.md).