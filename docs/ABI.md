# Contract ABI Reference

Complete interface reference for **trustbridge-contract**.

Related docs: [README](../README.md) · [ARCHITECTURE](ARCHITECTURE.md) · [DEPLOYMENT](DEPLOYMENT.md)

---

## Types

### ContributorRecord

```rust
struct ContributorRecord {
    stellar_address: Address,
    payout_address: Address,     // separate payout recipient from identity
    registered_at: u32,  // u32 saves 4 bytes vs u64; sufficient until ~2106
    verified: bool,
    is_bot: bool,
}
```

### Stats

```rust
struct Stats {
    total: u32,
    verified: u32,
}
```

### ExportPage

Returned by `get_registered_paginated` and `get_public_paginated`. Unlike
`get_all_registered`, each entry is a full `ContributorRecord` — so `verified`
is available directly, with no second call needed (Issue #96).

```rust
struct ExportPage {
    records: Vec<(String, ContributorRecord)>,
    next_cursor: Option<u32>,
    total: u32,
    has_more: bool,
}
```

### BatchSummary

Returned by `batch_verify`. `success_rate` is an integer percentage.

```rust
struct BatchSummary {
    total: u32,
    successful: u32,
    failed: u32,
    success_rate: u32,
}
```

### Role (u32 discriminant)

```rust
enum Role {
    Admin = 1,
    Upgrader = 2,
    /// May call `verify` only.
    Verifier = 3,
    /// May call `revoke_verification` only (Issue #212 — Verifier/Revoker split).
    Revoker = 4,
}
```

**Role matrix (Issue #212):**

| Role | `verify` | `revoke_verification` | `upgrade` | `set_role` |
|------|----------|-----------------------|-----------|------------|
| Admin | ✅ | ✅ | ✅ | ✅ |
| Verifier | ✅ | ❌ | ❌ | ❌ |
| Revoker | ❌ | ✅ | ❌ | ❌ |
| Upgrader | ❌ | ❌ | ✅ | ❌ |

### HealthSnapshot

Returned by `get_health` (Issue #210).

```rust
struct HealthSnapshot {
    paused: bool,
    version: Vec<u32>,          // [major, minor, patch]
    total: u32,
    verified: u32,
    cooldown_secs: u64,
    cooldown_remaining_secs: u64,
    attestation_present: bool,
}
```

### ChallengeRecord

Returned by `get_challenge` (Issue #214).

```rust
struct ChallengeRecord {
    challenged_by: Address,
    started_at: u64,
    resolve_after: u64,
}
```

### RevokeReason (u32 discriminant)

| Code | Name |
|------|------|
| 1 | `IdentityFraud` |
| 2 | `CompromisedKey` |
| 3 | `Regulatory` |
| 4 | `DuplicateRegistration` |
| 5 | `OperatorError` |
| 6 | `GdprErasure` |
| 99 | `Other` |

### ContractError (u32 discriminant)

| Code | Name | Description |
|------|------|-------------|
| 1 | `AlreadyInitialized` | Contract already has an admin |
| 2 | `NotInitialized` | Contract not yet initialized |
| 3 | `NotAuthorized` | Caller lacks permission |
| 4 | `NotRegistered` | Username not in registry |
| 5 | `AlreadyVerified` | Username already verified |
| 6 | `InvalidEntityType` | Unknown entity type value |
| 7 | `OrgNameRequired` | Team registration requires org_name |
| 6 | `NotVerified` | Cannot revoke verification because the username is not verified |
| 7 | `Paused` | Contract is paused for maintenance or emergency |
| 8 | `CooldownActive` | Upgrade cooldown period has not elapsed |
| 9 | `InvalidVersion` | Target version is not higher than current version |
| 10 | `InvalidRole` | Invalid or unauthorized role assignment |
| 11 | `InvalidUsername` | Username is empty, over `max_username_len`, or contains disallowed characters |
| 12 | `AttestationExpired` | Upgrade attestation's `expires_at` has passed |
| 13 | `UnattestedWasm` | `upgrade` hash does not match the live attestation |
| 14 | `InvalidBatchSize` | Batch call supplied zero items or more than the configured max |
| 15 | `InvalidReasonCode` | `revoke_verification` reason_code is not a known `RevokeReason` value |
| 16 | `ZeroAddress` | Supplied Stellar address is the well-known zero/burn address |
| 17 | `ChallengeAlreadyActive` | `start_challenge` called while a challenge is already open |
| 18 | `NoChallengeActive` | `cancel_challenge` or `complete_challenge` called with no active challenge |
| 19 | `ChallengeNotResolvable` | `complete_challenge` called before the delay has elapsed |
| 20 | `ChallengeActive` | `register` attempted while a challenge is active on the username |
| 21 | `NetworkMismatch` | Instance state was initialized on a different network than the one executing (Issue #231) |

`ContractError::from_code(u32)` maps every code in this table back to the typed
variant and returns `None` for any unrecognized code. Every code round-trips
through `from_code(variant.code()) == Some(variant)` — verified by the unit
tests in `src/lib.rs` (`test_error_from_code_is_inverse_of_code`).

---

### RoleHolder (Issue #228)

| Field | Type | Notes |
|-------|------|-------|
| `address` | `Address` | Address holding the role |
| `role` | `Role` | Role held, as the `Role` discriminant |

### EventDomain (Issue #226)

Attached as the `domain` field of **every** event this contract emits.

| Field | Type | Notes |
|-------|------|-------|
| `contract_id` | `Address` | Emitting contract instance |
| `network_id` | `BytesN<32>` | SHA-256 of the network passphrase |
| `contract_version` | `(u32, u32, u32)` | Contract version at emit time |
| `domain_version` | `u32` | Envelope schema version — currently `1` |

Indexers should scope deduplication to `(contract_id, network_id)`. See
[EVENT_INDEXING.md](./EVENT_INDEXING.md#event-domain-separation-issue-226).

This is an additive change to all 12 events: no field was renamed, retyped, or
removed, and topic assignments are unchanged. Consumers that decode events
positionally must be updated; consumers that ignore unknown fields need no
change.

## Batch size limits (Issue #227)

`batch_verify` and `batch_remove` are capped at **`MAX_WRITE_BATCH` = 25**
entries, not the generic `BatchConfig::default().max_batch_size` of 100.

The 100 was a shape check that was never derived from what a batch costs. Each
accepted entry pays a persistent read, a persistent write, a TTL extension, an
event publish, and an audit-log append; the worst case is a full batch of
39-character usernames that all need writing. Soroban exposes no host function
for querying remaining instruction budget, so a contract cannot check its
headroom mid-loop — a batch that overruns simply traps, with no partial success
to fall back on. The cap therefore has to be conservative and measured rather
than checked at runtime.

Passing more than 25 entries returns `InvalidBatchSize` (code 14) **before any
state is written**.

### Fail-before-write

`batch_verify` runs in two phases. Phase 1 resolves every entry and collects
only those that will actually change, writing nothing; phase 2 applies them.
A rejected batch therefore touches no state at all.

Both entry points now write each counter **once per batch** instead of a
read-modify-write per entry — 2 storage operations rather than 2N, and `count`
and `vcount` move exactly once, with no intermediate state in which one has been
advanced for some entries but not others.

Duplicate usernames within a single batch are collapsed in phase 1, so a
repeated entry is counted once against `vcount`.

**Raising `MAX_WRITE_BATCH` requires re-running `test_bench_batch_verify_max`
and `test_bench_batch_remove_max`, not just editing the constant.**

## Functions

### `initialize(admin: Address) -> Result<(), ContractError>`

One-time setup. Stores the admin address and zeroes counters.

| | |
|---|---|
| **Auth** | None (protect at deployment time) |
| **Mutates** | Yes |
| **Errors** | `AlreadyInitialized` |

**Admin immutability (Issue #97) and rotation (Issue #195).** The admin address
set here cannot be overwritten by `initialize` again (a second call always fails
with `AlreadyInitialized`). Admin rotation is now supported via the two-step
`propose_admin_transfer` → `execute_admin_transfer` flow — see
[SECURITY.md § Admin Key Management](SECURITY.md#admin-key-management).

```bash
stellar contract invoke --id $ID --source deployer --network testnet --send=yes \
  -- initialize --admin G...
```

---

### `register(github_username: String, stellar_address: Address, fallback_addresses: Vec<Address>) -> Result<(), ContractError>`

Register or update a GitHub username mapping. `entity_type` distinguishes personal users (0), orgs (1), and teams (2). Teams require `org_name`.

| | |
|---|---|
| **Auth** | `stellar_address` must sign; if the username is already registered to a *different* address, that address must sign too; each fallback address must also sign |
| **Mutates** | Yes |
| **Errors** | `NotInitialized`, `Paused`, `InvalidUsername`, `ZeroAddress`, `FallbackListFull` |
| **Events** | `RegisteredEvent` |

**Zero-address rejection:**

`stellar_address` must not be the well-known zero/burn address
(`GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF`, the strkey
encoding of an all-zero ed25519 public key), or the call fails with
`ZeroAddress` (code 15), checked before `require_auth`. On a live network
`require_auth` would already reject this address — no private key exists for
it — but `mock_all_auths` in tests and local sandboxes bypasses that check, so
the explicit guard is what actually stops a mistaken zero-address registration
in those environments, and gives dashboard/indexer consumers a typed error
instead of an opaque auth failure. Use `is_address_zero` to pre-check.

**Username validation:**

`github_username` must be a well-formed GitHub handle or the call fails with
`InvalidUsername` (code 11) before any authentication or storage write:

| Rule | Accepted | Rejected |
|---|---|---|
| Length 1–39 characters | `a`, `octocat` | `""`, 40+ characters |
| ASCII only — no Unicode | `user123`, `bob-smith` | `café`, `аlice` (Cyrillic a), `user😀`, `中user` |
| ASCII alphanumerics, `-`, `_` | `user_123`, `bob-smith` | `a@invalid`, `dot.name`, `has space` |
| First and last character alphanumeric | `alice`, `7` | `-invalid`, `invalid-`, `_leading`, `trailing_` |
| No consecutive hyphens | `foo-bar-baz` | `foo--bar` |

**Case folding (Issue #194).** GitHub logins are case-insensitive, so every
persistent storage key built from `github_username` uses its ASCII-lowercased
canonical form — `Alice`, `ALICE`, and `alice` all resolve to one record.
Registering a case variant of an existing login updates that record (subject
to the same `old.stellar_address.require_auth()` protection a same-case
re-registration requires) rather than creating a second, independent entry.
Paginated exports and `get_all_registered` report the canonical (lowercased)
username; domain events report the raw string as submitted. See
[SECURITY.md § Username Case-Folding](SECURITY.md#5-username-case-folding-issue-194).

Two deliberate choices:

- **Underscores are accepted** even though GitHub itself rejects them. Records
  written before validation existed must stay removable, and `remove` looks a
  username up by exact key — a name that cannot be expressed could never be
  cleaned up.
- **Validation applies to `register` only.** Lookups, `remove`, `verify` and
  `revoke_verification` accept any username, for the same reason.

Checks run *before* `require_auth`, so a malformed username is rejected at the
cheapest point and the caller is not charged for an auth check on an invocation
that can never succeed. It is also what stops an unbounded key from reaching
persistent storage.

Behavior:

- New username → increment `count`, append to `idx`
- Existing username → update record; reset `verified` if address changed
- Existing username pointed at a new address → the **currently registered
  address must also authorize the call**. Without its signature the invocation
  fails at auth, so a username cannot be taken over by whoever calls `register`
  next.
- Cold-start registration from an initialized empty registry must expose the
  new record through both `get_address` and admin `get_all_registered`; this is
  covered by the Wave #50 regression test.
- If a verified username is updated to a new Stellar address, verification is
  cleared until the admin verifies the updated address. The Wave #49 regression
  test covers re-verification against the new address.
- Index-length invariant: every successful `register` increments `COUNT_KEY`
  **and** appends to `INDEX_KEY` atomically. A re-registration of an existing
  username does neither. See [SECURITY.md](SECURITY.md#index-length-invariant).

**Copy-pasteable examples**

```bash
# Register personal account
stellar contract invoke --id $ID --source deployer --network testnet --send=yes \
  -- register --github-username octocat --stellar-address G... --entity-type 0

# Register org
stellar contract invoke --id $ID --source deployer --network testnet --send=yes \
  -- register --github-username my-org --stellar-address G... --entity-type 1 --org-name my-org

# Register team
stellar contract invoke --id $ID --source deployer --network testnet --send=yes \
  -- register --github-username my-team --stellar-address G... --entity-type 2 --org-name my-org
# New registration (registrant signs with the Stellar address being registered)
stellar contract invoke --id $CONTRACT_ID \
  --source-account deployer \
  --network testnet \
  --send=yes \
  -- register \
  --github-username octocat \
  --stellar-address G...
```

```bash
# Re-point an existing username to a new address
# BOTH the old and new addresses must sign this call
stellar contract invoke --id $CONTRACT_ID \
  --source-account deployer \
  --network testnet \
  --send=yes \
  -- register \
  --github-username octocat \
  --stellar-address G...
```

**Common failure modes**

| Failure | Cause | Fix |
|---|---|---|
| `NotInitialized` (code 2) | Contract not yet initialized | Run `make invoke-init` first |
| `Paused` (code 7) | Contract is paused | Wait for unpause or contact admin |
| `InvalidUsername` (code 11) | Username empty, >39 chars, or contains disallowed characters | Use 1–39 ASCII alphanumerics, hyphens, underscores |
| `NotAuthorized` (code 3) | `stellar_address` did not sign, or old address did not sign on transfer | Ensure the correct source account is used |


---

### `register_sponsored(github_username: String, stellar_address: Address, sponsor: Address) -> Result<(), ContractError>`

Register or update a GitHub username mapping sponsored by a maintainer/account.

| | |
|---|---|
| **Auth** | Both `stellar_address` and `sponsor` must sign; if already registered to a different address, that old address must sign too (double-auth protection) |
| **Mutates** | Yes |
| **Errors** | `NotInitialized`, `Paused`, `InvalidUsername` |
| **Events** | `RegisteredEvent` (carrying the sponsor) |

```bash
stellar contract invoke --id $ID --source sponsor_key --network testnet --send=yes \
  -- register_sponsored --github-username octocat --stellar-address G... --sponsor G...
```

---

### `max_username_len() -> u32`

Returns the maximum accepted username length (currently `39`). Clients should
read this instead of hardcoding the limit, so relaxing the guard does not
require a client release.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |

---

### `is_username_valid(github_username: String) -> bool`

Reports whether a username would pass the `register` guard, so a dashboard can
validate input before asking the user to sign.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |

---

### `is_address_zero(address: Address) -> bool`

Reports whether `address` is the well-known zero/burn address that `register`
rejects, so a dashboard or indexer consumer can validate a Stellar address
before asking a user to sign — mirroring `is_username_valid`.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |

---

### `usernames_match(a: String, b: String) -> bool`

Case-insensitive username equality, matching GitHub's own semantics. Off-chain
verification workflows use this to match a registration against a GitHub
identity without depending on the stored casing.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |

Cost is linear in the number of comparisons and allocation-free — the
comparison runs on a fixed stack buffer. `make bench-username` records the
metered CPU/memory cost across 10–200 comparisons.

---

### `get_address(github_username: String) -> Option<ContributorRecord>`

Read-only lookup. Returns `null`/`None` if not registered.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |

```bash
stellar contract invoke --id $ID --source deployer --network testnet \
  -- get_address --github-username octocat
```

---

### `remove(caller: Address, github_username: String) -> Result<(), ContractError>`

Remove a registration.

| | |
|---|---|
| **Auth** | `caller` must sign; must be admin or registrant — see **Auth model** below |
| **Mutates** | Yes |
| **Errors** | `NotInitialized`, `Paused`, `NotRegistered`, `NotAuthorized` |
| **Events** | `RemovedEvent` |

```bash
# Self-removal (registrant signs)
stellar contract invoke --id $ID --source registrant --network testnet --send=yes \
  -- remove --caller G... --github-username octocat

# Admin removal
stellar contract invoke --id $ID --source admin --network testnet --send=yes \
  -- remove --caller G... --github-username octocat
```

**Auth model** (Issue #74 / Wave #75):

The authorization check is isolated in `TrustBridgeContract::require_remove_auth`
so it can be read, tested, and changed independently of the mutation logic:

```
caller == admin                    → allowed
caller == record.stellar_address   → allowed (registrant self-removal)
anything else                      → NotAuthorized
```

`caller.require_auth()` runs first — the host verifies the signature before the
policy check. `NotRegistered` is returned before auth when the username does not
exist in storage, so a caller cannot probe username existence by observing whether
they get `NotRegistered` or `NotAuthorized`.

**Error precedence:**

| Condition | Error |
|-----------|-------|
| Contract not initialized | `NotInitialized` |
| Contract paused | `Paused` |
| Username not registered | `NotRegistered` |
| Caller is not admin or registrant | `NotAuthorized` |

**Test coverage** (`src/lib.rs`, Issue #74 / Wave #75):

| Test | Path |
|------|------|
| `test_registrant_can_remove_own_record` | Success — registrant |
| `test_admin_can_remove_any_record` | Success — admin |
| `test_admin_can_remove_record_registered_by_another_user` | Success — admin removes different user's record |
| `test_third_party_cannot_remove` | Failure — `NotAuthorized`, record and count unchanged |
| `test_unknown_address_cannot_remove` | Failure — `NotAuthorized` for fresh address with no role |
| `test_remove_unregistered_username_fails` | Failure — `NotRegistered` |
| `test_remove_already_removed_username_fails` | Failure — `NotRegistered` on double removal |
| `test_remove_blocked_while_paused` | Failure — `Paused` |
| `test_remove_unverified_record_does_not_decrement_verified_count` | Invariant — verified count unchanged |
| `test_remove_verified_record_decrements_verified_count` | Invariant — verified count decremented |
| `test_readding_removed_user_increments_count` | Invariant — re-add starts fresh and unverified |

Stats invariant: partial removal decrements `total` only for the removed record
and decrements `verified` only when that removed record was verified. Removing
an unverified record while another verified record remains must leave
`verified` unchanged; this is covered by the Wave #46 regression test and
`test_remove_unverified_record_does_not_decrement_verified_count`.

Index-length invariant: after every `remove`, `get_stats().total` must equal
the length of the flat username index. Both are updated atomically in the same
transaction. See the **Index-Length Invariant** section in
[SECURITY.md](SECURITY.md#index-length-invariant) and the test suite in
`tests/integration.rs` (Issue #59 / Wave #60).

Empty-registry invariant (Issue #92): removing the **last** registered
contributor returns the registry to a clean empty state — `get_stats()`
reports `{total: 0, verified: 0}`, the username index is empty
(`get_all_registered`, `get_registered_page`, and the paginated export paths
all return zero records with `has_more: false`), and every lookup
(`get_address`, `has_record`) reports absence. No stale index entry or
non-zero counter survives. A subsequent registration on the now-empty
registry proceeds exactly as it would on a never-used one. Covered by
`test_remove_last_user_returns_registry_to_empty_state` in `src/lib.rs`.

---

### `batch_remove(caller: Address, usernames: Vec<String>) -> Result<BatchSummary, ContractError>`

Removes multiple registrations in a single invocation, collecting per-entry errors rather than aborting on the first failure.

This is the batched form of `remove`, intended for admin workflows that need to clean up many stale or disputed registrations efficiently. Doing that as N separate invocations costs N transactions, N signatures, and N rounds of ledger overhead — this is one.

| | |
|---|---|
| **Auth** | Admin |
| **Mutates** | Yes |
| **Errors** | `NotInitialized`, `Paused`, `InvalidBatchSize`, `NotAuthorized` |
| **Events** | One `RemovedEvent` per successfully removed contributor |

**Partial success is the point.** A username that cannot be removed (e.g., not registered) does not abort the batch; it is counted as a failure in the returned `BatchSummary` and the rest proceed. A cleanup of 100 contributors must not be lost wholesale because one entry was already removed.

Unlike the single `remove` function which allows registrants to self-remove, `batch_remove` is strictly admin-only.

| Outcome | Counted as | Notes |
|---------|------------|-------|
| Registered, caller is admin | `successful` | Record removed, `RemovedEvent` published |
| Not registered | `failed` | Skipped, batch continues |
| Caller not authorized | `failed` | The entire call fails with `NotAuthorized` if the caller is not the admin |

```bash
stellar contract invoke --id $ID --source admin --network testnet --send=yes \
  -- batch_remove --caller G... --usernames '["octocat","alice"]'
```

---

### `get_all_registered() -> Result<Vec<(String, Address)>, ContractError>`

Export the full registry. Admin-only.

| | |
|---|---|
| **Auth** | Admin |
| **Mutates** | No |
| **Errors** | `NotInitialized` |

```bash
stellar contract invoke --id $ID --source admin --network testnet \
  -- get_all_registered
```

For a repeatable JSON export (backups, dashboard migrations, audit
snapshots) that pages through `get_registered_paginated` instead of this
single-call export, see [Registry Export & Import](DEPLOYMENT.md#registry-export--import).

---

### `verify(caller: Address, github_username: String) -> Result<(), ContractError>`

Bulk export must not exceed Soroban resource limits on large registries.
`get_all_registered` remains available for small dumps; production sync jobs
should page.

### Limit constants (`src/storage.rs`)

| Constant | Value | Notes |
|----------|------:|-------|
| `DEFAULT_PAGE_LIMIT` | `20` | Applied when `limit == 0` |
| `MAX_PAGE_LIMIT` | `100` | Enforced by **clamping** (`limit.min(MAX_PAGE_LIMIT)`); over-limit does not error |

Justification: a single invoke that materializes more than ~100 registry
reads trips the ledger-entry footprint ceiling (see [Cost and
benchmarks](#cost-and-benchmarks)). Capping the page keeps each call inside
that budget while still allowing full export via a cursor loop.

### `ExportPage`

| Field | Type | Semantics |
|-------|------|-----------|
| `records` | `Vec<(String, ContributorRecord)>` | Current page |
| `next_cursor` | `Option<u32>` | Next zero-based index offset, or `None` when done |
| `total` | `u32` | Live registration count |
| `has_more` | `bool` | `true` when another page exists |
| `merkle_root` | `BytesN<32>` | Merkle root over `records`, in page order (Issue #216) |

### Merkle root over an export page (Issue #216)

`merkle_root` lets a treasury or dashboard prove a specific
`(github_username, stellar_address, verified)` triple was present in a given
export page without republishing the whole registry, and lets anyone detect
an edited off-chain CSV copy of that page (its recomputed root would not
match the one the contract returned).

**Leaf encoding:**

```text
leaf = SHA256("trustbridge/export-leaf/v1:" || username_bytes || 0x00 || address_strkey_bytes || verified_byte)
```

- `username_bytes` — raw bytes of the entry's `github_username` exactly as it
  appears in `records` (its canonical, lowercased form — see
  [SECURITY.md § Username Case-Folding](SECURITY.md#5-username-case-folding-issue-194)).
- `0x00` — fixed separator between the username and address byte strings.
- `address_strkey_bytes` — raw bytes of `record.stellar_address`'s Stellar
  strkey (the `G...` string), not its internal binary encoding.
- `verified_byte` — `0x01` if `record.verified`, else `0x00`. The verified
  flag is part of the leaf, so a proof attests to verification status at
  export time, not just membership.

`merkle_leaf_hash(github_username, stellar_address, verified) -> BytesN<32>`
is a read-only, no-auth contract call exposing this exact computation, so
off-chain tooling can check its own reimplementation against the on-chain one
before trusting proofs built from it.

**Tree construction:** a standard bottom-up binary tree,
`node = SHA256("trustbridge/export-node/v1:" || left || right)`, over the
page's leaves in page order. When a level has an odd number of nodes, the
last one is promoted to the next level **unchanged** — never duplicated or
re-hashed with itself. An empty page's root is 32 zero bytes; a one-record
page's root is that single leaf's hash unchanged (every level promotes it).

**Scope:** one root per page, not a historic accumulator over the registry's
lifetime, and no zero-knowledge proof — a verifier needs the leaf's plaintext
fields and a sibling-hash path, like any standard Merkle proof. See
`src/merkle.rs` for the reference implementation and
`tests/merkle_export.rs` for a worked inclusion proof (built independently of
the contract's own tree code) that verifies for a member and fails for a
non-member.

### `get_registered_paginated(cursor: u32, limit: u32) -> Result<ExportPage, ContractError>`

| | |
|---|---|
| **Auth** | Admin (`admin.require_auth()`) — unchanged by Issue #143 |
| **Mutates** | No |
| **Errors** | `NotInitialized` |
| **Limit** | `0` → `DEFAULT_PAGE_LIMIT`; `> MAX_PAGE_LIMIT` → clamped to `MAX_PAGE_LIMIT` |

```bash
stellar contract invoke --id $ID --source admin --network testnet \
  -- get_registered_paginated --cursor 0 --limit 100
```

### `get_public_paginated(cursor: u32, limit: u32) -> Result<ExportPage, ContractError>`

Same page shape and limit clamping; no admin auth; requires not paused.

```bash
stellar contract invoke --id $ID --source deployer --network testnet \
  -- get_public_paginated --cursor 0 --limit 100
```

### Consumer loop

```text
cursor ← 0
repeat:
  page ← get_registered_paginated(cursor, limit)   # or get_public_paginated
  process(page.records)
  if page.has_more is false OR page.next_cursor is None:
    stop
  cursor ← page.next_cursor
```

Exhaustion is when `has_more == false` / `next_cursor == None` (including an
empty page when `cursor >= total`).

Boundary tests: `test_paginated_export_at_max_page_limit`,
`test_paginated_export_over_max_page_limit_clamps` in `src/lib.rs`.

---

### `config_verification(caller: Address, attestation: Symbol, expires_in: u64, threshold: u32) -> Result<(), ContractError>`

Stores the verification configuration parameters (attestation symbol, expiration window, and threshold). Can only be called once by the contract admin.

| | |
|---|---|
| **Auth** | Admin |
| **Mutates** | Yes |
| **Errors** | `NotInitialized`, `Paused`, `NotAuthorized`, `AlreadyInitialized` |

```bash
stellar contract invoke --id $ID --source admin --network testnet --send=yes \
  -- config_verification --caller G... --attestation github_att --expires-in 3600 --threshold 2
```

---

### `verify(caller: Address, github_username: String) -> Result<(), ContractError>`

Admin-gated page of `(github_username, stellar_address)` pairs, for large
registries where `get_all_registered` would exceed the per-invocation
footprint limit. Same shape as `get_all_registered` — no `verified` flag; use
`get_registered_paginated` for that.

| | |
|---|---|
| **Auth** | Admin |
| **Mutates** | No |
| **Errors** | `NotInitialized` |

---

### `get_registered_paginated(cursor: u32, limit: u32) -> Result<ExportPage, ContractError>`

Admin-gated paginated export. Each entry is a full `ContributorRecord`, so the
`verified` bit travels with every row — no second call or cross-reference
against `get_address` is needed to know verification status (Issue #96).

| | |
|---|---|
| **Auth** | Admin |
| **Mutates** | No |
| **Errors** | `NotInitialized` |

`limit = 0` defaults to `DEFAULT_PAGE_LIMIT` (20); any value above
`MAX_PAGE_LIMIT` (100) is clamped down to it. `next_cursor` is `Some` while
`has_more` is true; pass it back as `cursor` to continue.

```bash
stellar contract invoke --id $ID --source admin --network testnet \
  -- get_registered_paginated --cursor 0 --limit 50
```

---

### `get_public_paginated(cursor: u32, limit: u32) -> Result<ExportPage, ContractError>`

Same `ExportPage` shape as `get_registered_paginated` — including the
`verified` flag per record — but callable by anyone. This is the
unauthenticated read path for dashboards and indexers that need
username + address + verified without an admin key (Issue #96).

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |
| **Errors** | `NotInitialized`, `Paused` |

```bash
stellar contract invoke --id $ID --source deployer --network testnet \
  -- get_public_paginated --cursor 0 --limit 50
```

---

### `verify(caller: Address, github_username: String) -> Result<(), ContractError>`

Mark a contributor as verified after off-chain GitHub identity confirmation.

Admin authorization is authoritative. The `caller` argument must be the admin or an address granted `Role::Verifier` via `set_role`. A registrant **cannot** self-verify.

| | |
|---|---|
| **Auth** | Admin **or** any address assigned `Role::Verifier` |
| **Caller arg** | `caller: Address` — must be the admin or a `Verifier`-role holder |
| **Mutates** | Yes |
| **Errors** | `NotInitialized`, `NotRegistered`, `AlreadyVerified`, `NotAuthorized` |
| **Events** | `VerifiedEvent` |

The `caller` argument is required so the contract can validate which identity
signed the transaction. Both the admin and any address granted `Role::Verifier`
via `set_role` may call this function. An address without either role returns
`NotAuthorized`. Calling `verify` on a `github_username` that has never been
registered returns `NotRegistered` rather than creating a record (Issue #57);
`revoke_verification` guards the same way (see tests in `src/lib.rs` and
`tests/integration.rs`).

```bash
# Admin calling verify (authoritative example)
stellar contract invoke --id $ID --source admin --network testnet --send=yes \
  -- verify --caller G... --github-username octocat
```

---

### `batch_verify(caller: Address, usernames: Vec<String>) -> Result<BatchSummary, ContractError>`

Verify many contributors in a single invocation — the batched form of `verify`,
for the dashboard-sync workflow where an off-chain job confirms a page of GitHub
identities at once. Doing that as N separate invocations costs N transactions,
N signatures and N rounds of ledger overhead; this is one.

| | |
|---|---|
| **Auth** | Admin **or** any address assigned `Role::Verifier` |
| **Mutates** | Yes |
| **Errors** | `NotInitialized`, `Paused`, `InvalidBatchSize`, `NotAuthorized` |
| **Events** | One `VerifiedEvent` per newly verified contributor |
| **Since** | 1.1.0 — gate on `Version::supports_batch_verify` |

**Partial success is the point.** A username that cannot be verified does not
abort the batch; it is counted as a failure and the rest proceed. A sync of 100
contributors must not be lost wholesale because one entry was removed or already
verified since the off-chain job built its list.

| Outcome | Counted as | Notes |
|---|---|---|
| Registered and unverified | `successful` | Record updated, `VerifiedEvent` published |
| Not registered | `failed` | Skipped, batch continues |
| Already verified | `failed` | Skipped — idempotent, so re-runs are safe |

Inspect the returned `BatchSummary`: a `success_rate` below 100 means some
entries need attention, **not** that the batch failed. The errors listed above
are the only conditions that abort the whole call, and all of them invalidate
every entry rather than a single one.

`verified` is incremented per newly verified item, and `vcount` stays consistent with successful items.

```bash
stellar contract invoke --id $ID --source admin --network testnet --send=yes \
  -- batch_verify --caller G... --usernames '["octocat","alice","bob-smith"]'
```

---

### `revoke_verification(caller: Address, github_username: String, reason_code: u32) -> Result<(), ContractError>`

Revoke verification for a registered contributor.

Admin authorization is authoritative. The `caller` argument must be the admin or an address granted `Role::Verifier` via `set_role`. A registrant **cannot** revoke another's verification.

| | |
|---|---|
| **Auth** | Admin **or** any address assigned `Role::Verifier` |
| **Caller arg** | `caller: Address` — must be the admin or a `Verifier`-role holder |
| **Mutates** | Yes |
| **Errors** | `NotInitialized`, `Paused`, `InvalidReasonCode`, `NotRegistered`, `NotVerified`, `NotAuthorized` |
| **Events** | `VerificationRevokedEvent` |

Like `verify`, the `caller` argument enables on-chain role enforcement. Only the contract admin or a `Verifier`-role holder may revoke verification. An `Upgrader`-role holder or an address with no role returns `NotAuthorized`.

**Mainnet incident response:** prefer this method over `remove` when the goal
is to stop trust quickly. Operator runbook (detect → revoke → notify → audit
export): [ADMIN_RUNBOOK.md](ADMIN_RUNBOOK.md#mainnet-incident-emergency-verification-revoke).
Threat-model notes: [SECURITY.md](SECURITY.md#mainnet-verification-revoke-incidents).

```bash
# Admin revoking verification (authoritative example)
stellar contract invoke --id $ID --source admin --network testnet --send=yes \
  -- revoke_verification --caller G... --github-username octocat --reason-code 1
```

`reason_code` is emitted in `VerificationRevokedEvent` and must be one of the
codes documented in [`RevokeReason`](#revokereason-u32-discriminant): `1`–`6`
or `99`.

**Verify → revoke → verify cycle (Issue #95).** `verify` and
`revoke_verification` can be applied to the same username repeatedly without
corrupting counters or storage: revoking decrements `verified`, and a
subsequent `verify` succeeds and publishes a fresh `VerifiedEvent`. Only the
`verified` flag and the `verified` counter change across the cycle — the
record's `stellar_address` and `registered_at` are untouched. A `verify` call
with no intervening `revoke_verification` still fails `AlreadyVerified`, even
after the record has already been through one full cycle. Covered by
`test_issue_95_verify_revoke_verify_cycle` and
`test_issue_95_double_verify_without_revoke_fails_mid_cycle` in `src/lib.rs`.

---

### `set_bot_status(caller: Address, github_username: String, is_bot: bool) -> Result<(), ContractError>`

Sets the bot-account status flag on a contributor record.

| | |
|---|---|
| **Auth** | `caller` must sign; must be admin or registrant |
| **Mutates** | Yes |
| **Errors** | `NotInitialized`, `NotRegistered`, `NotAuthorized` |

```bash
# Registrant setting own bot flag
stellar contract invoke --id $ID --source registrant --network testnet --send=yes \
  -- set_bot_status --caller G... --github-username octocat --is_bot true

# Admin setting bot flag
stellar contract invoke --id $ID --source admin --network testnet --send=yes \
  -- set_bot_status --caller G... --github-username octocat --is_bot true
```

---

### `get_verified_count() -> u32`

Returns the number of **currently** verified registrations. This figure drops
when a verification is revoked. For "how many were ever verified", read
[`get_ever_verified_count`](#get_ever_verified_count---u32) instead.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |

**Parity invariant (Issue #90):** `get_verified_count()` always equals
`get_stats().verified`, and both always equal the number of stored records
with `verified == true`. This holds after every path that touches
verification state — `register` (including an address-change re-register),
`verify`, `revoke_verification`, and `remove` — including on an empty
registry and across repeated verify/revoke cycles. See
[REGISTRY_INVARIANTS.md#verification](REGISTRY_INVARIANTS.md#verification)
and `test_verified_count_parity_across_all_mutation_paths` in `src/lib.rs`.

```bash
stellar contract invoke --id $ID --source deployer --network testnet \
  -- get_verified_count
```

---

### `get_ever_verified_count() -> u32`

Returns how many verifications have ever been granted, including any since
revoked (Issue #229). Monotonic: it never decreases.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |

`get_verified_count()` answers "who is verified right now" and moves in both
directions; this answers "how many did we ever verify" and only climbs. A
contributor verified, revoked, and verified again counts twice here — the
counter records verification events, not distinct contributors.

Instances deployed before this counter existed have no stored value and report
the live verified count instead of zero, which is the tightest lower bound the
contract can still prove after the fact.

```bash
stellar contract invoke --id $ID --source deployer --network testnet \
  -- get_ever_verified_count
```

---

### `get_stats() -> Stats`

Returns `{ total, verified, ever_verified }` registration counts.

`verified` is the live count and falls on revoke; `ever_verified` is the
monotonic historical count described above. `total` is the number of
registered contributors.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |

```bash
stellar contract invoke --id $ID --source deployer --network testnet \
  -- get_stats
```

---

### `pause() -> Result<(), ContractError>`

Pauses all state-mutating contract operations. Admin-only.

---

### `unpause() -> Result<(), ContractError>`

Unpauses state-mutating contract operations. Admin-only.

---

### `is_paused() -> bool`

Returns true if contract mutations are currently paused.

---

### `set_role(target: Address, role: Role) -> Result<(), ContractError>`

Assigns an administrative or operational role (`Admin`, `Upgrader`, `Verifier`). Admin-only.

---

### `remove_role(target: Address) -> Result<(), ContractError>`

Revokes a role from an address. Admin-only.

---

### `get_role(address: Address) -> Option<Role>`

Queries assigned role for an address.

---

### `set_cooldown(cooldown_seconds: u64) -> Result<(), ContractError>`

Configures the WASM upgrade timelock cooldown period in seconds. Admin-only.

---

### `get_cooldown() -> u64`

Returns the current WASM upgrade timelock cooldown period in seconds.

---

### `get_version() -> (u32, u32, u32)`

Returns contract version tuple `(major, minor, patch)`.

---

### `upgrade(new_wasm_hash: BytesN<32>) -> Result<(), ContractError>`

Upgrades the executable WASM bytecode of the contract. Subject to admin
authentication and the upgrade timelock cooldown.

| | |
|---|---|
| **Auth** | Admin |
| **Mutates** | Yes |
| **Errors** | `NotInitialized`, `Paused`, `CooldownActive`, `UnattestedWasm`, `AttestationExpired` |
| **Events** | `UpgradedEvent` |

Records a `WasmProvenance` entry for the new hash: what it replaced, who
authorised it, when, at what version, and whether it had been attested. The
record is written *before* the executable is swapped — afterwards the code
answering the question is the new binary, and what it replaced would be lost.

---

### `attest_upgrade(wasm_hash: BytesN<32>, expires_at: u64) -> Result<(), ContractError>`

Declare in advance the WASM hash you intend to deploy. Admin-only.

| | |
|---|---|
| **Auth** | Admin |
| **Mutates** | Yes |
| **Errors** | `NotInitialized`, `AttestationExpired` (if `expires_at` is not in the future) |
| **Events** | `UpgradeAttestedEvent` |

**Optional two-step upgrade.** While an attestation is live, `upgrade` accepts
only the hash it names — so a compromised admin key cannot swap in a different
binary at the moment of the upgrade without first publishing that intent
on-chain, ahead of time, where watchers can see it.

| Situation | `upgrade` behaviour |
|---|---|
| No attestation published | Proceeds as before — attestation is opt-in |
| Attestation matches, unexpired | Proceeds; attestation is consumed; `attested: true` in provenance |
| Attestation expired | Fails `AttestationExpired`; the stale record is cleared so a retry is not blocked |
| Hash does not match | Fails `UnattestedWasm`; the attestation is **left in place**, since a mismatch may be an attacker substituting a binary and clearing it would let a second attempt through unchecked |

Attestations are **single-use** — one upgrade, not a standing permission for
that hash. `expires_at` is mandatory for the same reason: an attestation that
never lapsed would be a standing authorisation, which is worse than none.

Publishing a new attestation replaces any existing one.

```bash
stellar contract invoke --id $ID --source admin --network testnet --send=yes \
  -- attest_upgrade --wasm-hash <hex> --expires-at 1893456000
```

---

### `clear_attestation() -> Result<(), ContractError>`

Withdraw a pending attestation. Admin-only. The escape hatch for one published
in error — without it the admin would have to wait out the expiry before
upgrading to any other hash.

| | |
|---|---|
| **Auth** | Admin |
| **Mutates** | Yes |
| **Errors** | `NotInitialized` |
| **Events** | `AttestationClearedEvent` (only if an attestation was actively stored) |

---

### `get_attestation() -> Option<WasmAttestation>`

Returns the pending attestation, if any. Returned regardless of expiry, since
seeing a lapsed attestation is what explains a rejected upgrade.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |

---

### `get_provenance() -> Option<WasmProvenance>`

Returns the provenance of the currently deployed WASM. `None` on an instance
that has never been upgraded.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |

```bash
stellar contract invoke --id $ID --source deployer --network testnet \
  -- get_provenance
```

---

### `migrate(new_version: (u32, u32, u32)) -> Result<(), ContractError>`

Updates the contract schema version following a WASM upgrade. Target version must be strictly higher than current version. Admin-only.

---

## Events

All events are defined with `#[contractevent]` and include a topic field for filtering.

For idempotent handling of these events by indexers — replays, gaps, and
duplicate deliveries of `RegisteredEvent` / `VerifiedEvent` /
`VerificationRevokedEvent` / `RemovedEvent` — see
[DASHBOARD_SYNC.md — Event Idempotency & Replay Handling](DASHBOARD_SYNC.md#event-idempotency--replay-handling).

### RegisteredEvent

```
topics: ["registered_event", github_username]
data:   { stellar_address, timestamp, sponsor: Option<Address> }
```

### RemovedEvent

```
topics: ["removed_event", github_username]
data:   { stellar_address, timestamp }
```

`RemovedEvent` is the only signal an indexer receives that a record is gone, so
its payload is treated as a compatibility surface:

- `github_username` is a **topic**, so subscribers can filter server-side.
- `stellar_address` is the address that was registered at removal time, and
  `timestamp` is the ledger timestamp of the removal — together they let a
  consumer reconstruct the retired record without a follow-up read.
- A **failed** `remove` (wrong caller, unknown username) publishes no event.

`test_removed_event_payload_is_complete` asserts the full published event
against a fully-specified `RemovedEvent`, plus the topic count and topic symbol
independently, so renaming the event or dropping a field fails the build rather
than silently breaking every subscriber's filter.
`test_removed_event_not_published_on_failed_remove` covers the failure path.

### VerifiedEvent

```
topics: ["verified_event", github_username]
data:   { stellar_address, timestamp }
```

`VerifiedEvent` is the primary signal an indexer uses to learn that a
contributor's GitHub identity has been confirmed. Its payload is a
compatibility surface: any rename of the topic symbol, reorder of topics,
or change to the data fields silently breaks every downstream subscriber.

The following tests in `src/lib.rs` pin the full payload (Issue #64 / Wave #65):

| Test | What it checks |
|------|----------------|
| `test_verified_event_payload_is_complete` | Full event list matches: topic symbol `"verified_event"`, `github_username` topic, `stellar_address` and `timestamp` in data |
| `test_verified_event_not_published_on_already_verified` | **Failure path**: `AlreadyVerified` must not publish an event |
| `test_verified_event_not_published_on_unregistered_username` | **Failure path**: `NotRegistered` must not publish an event |
| `test_verified_event_carries_current_stellar_address` | Event carries the address current at verify time, not a stale pre-update address |

### VerificationRevokedEvent

```
topics: ["verification_revoked_event", github_username]
data:   { stellar_address, timestamp, reason_code }
```

Mirrors `VerifiedEvent`. Indexers subscribe to both and reconcile verified
state from the event stream, so `VerificationRevokedEvent` is the same
compatibility surface.

| Test | What it checks |
|------|----------------|
| `test_verification_revoked_event_payload_is_complete` | Full event list matches: topic symbol, `github_username` topic, `stellar_address`, `timestamp`, and `reason_code` in data |
| `test_verification_revoked_event_not_published_on_not_verified` | **Failure path**: `NotVerified` must not publish an event |

### UpgradedEvent

```
topics: ["upgraded_event", new_wasm_hash]
data:   { version, timestamp }
```

### PausedEvent / UnpausedEvent

```
topics: ["paused_event" / "unpaused_event", admin]
data:   { timestamp }
```

### RoleGrantedEvent

```
topics: ["role_granted_event", address]
data:   { role, admin, timestamp }
```

`RoleGrantedEvent` is emitted when an admin assigns a role to an address.
Subscribers can filter on the `address` topic to track role changes for a
specific identity.

### RoleRevokedEvent

```
topics: ["role_revoked_event", address]
data:   { admin, timestamp }
```

`RoleRevokedEvent` is emitted when an admin revokes a role from an address.
Like `RoleGrantedEvent`, the `address` topic lets indexers filter server-side.
Note that `role` is **not** present in the data payload — a consumer that needs
the previous role must track it from the corresponding `RoleGrantedEvent`.

---

### AdminTransferProposedEvent (Issue #195)

```
topics: ["admin_transfer_proposed_event", new_admin]
data:   { proposed_by, executable_at, timestamp }
```

Emitted when `propose_admin_transfer` is called. `new_admin` is indexed as a
topic for efficient filtering. `executable_at` is the earliest timestamp at
which `execute_admin_transfer` may succeed.

### AdminTransferCancelledEvent (Issue #195)

```
topics: ["admin_transfer_cancelled_event", cancelled_by]
data:   { timestamp }
```

Emitted when the current admin cancels a pending transfer proposal.

### AdminTransferExecutedEvent (Issue #195)

```
topics: ["admin_transfer_executed_event", new_admin]
data:   { old_admin, timestamp }
```

Emitted when the proposed new admin successfully accepts the transfer. After
this event `ADMIN_KEY` points at `new_admin`.

---

### `version() -> (u32, u32, u32)`

Returns the deployed contract version as `(major, minor, patch)`.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |

The version is written to instance storage by `initialize`. Instances deployed
before version tracking existed carry no stored version and report the build
constant `1.0.0` instead.

```bash
stellar contract invoke --id $ID --source deployer --network testnet \
  -- version
```

---

### `is_compatible(major: u32, minor: u32, patch: u32) -> bool`

Reports whether the deployed contract satisfies a client's minimum required
version.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |

Rules:

- A higher major version is always compatible with a lower required major
- Within the same major, the deployed minor and patch must be at least the
  required ones
- A lower deployed version than required returns `false`

```bash
stellar contract invoke --id $ID --source deployer --network testnet \
  -- is_compatible --major 1 --minor 0 --patch 0
```

---

## Admin Transfer Functions (Issue #195)

### `propose_admin_transfer(new_admin: Address, delay_seconds: u64) -> Result<(), ContractError>`

Proposes a transfer of admin rights to `new_admin` with a mandatory delay.

| | |
|---|---|
| **Auth** | Contract admin |
| **Mutates** | Yes |
| **Errors** | `NotInitialized`, `Paused`, `NotAuthorized`, `ZeroAddress` |
| **Events** | `AdminTransferProposedEvent` |

A second call while a proposal is pending **overwrites** it (correcting address or delay is allowed during the window). The current admin remains the only admin throughout.

### `cancel_admin_transfer() -> Result<(), ContractError>`

Cancels a pending admin transfer proposal. No-op if no proposal is pending.

| | |
|---|---|
| **Auth** | Contract admin |
| **Mutates** | Yes |
| **Errors** | `NotInitialized`, `NotAuthorized` |
| **Events** | `AdminTransferCancelledEvent` |

### `execute_admin_transfer(caller: Address) -> Result<(), ContractError>`

Accepts the pending transfer. Must be called by the proposed new admin after the delay elapses.

| | |
|---|---|
| **Auth** | `caller` must be the proposed new admin |
| **Mutates** | Yes — atomically rotates `ADMIN_KEY`, removes old admin's `Role::Admin`, grants `Role::Admin` to the new admin |
| **Errors** | `NotInitialized`, `Paused`, `NoPendingAdminTransfer`, `NotAuthorized`, `AdminTransferDelayActive` |
| **Events** | `AdminTransferExecutedEvent` |

### `get_admin_transfer() -> Option<AdminTransferProposal>`

Returns the pending admin transfer proposal, or `None` if no transfer is pending.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |

```rust
struct AdminTransferProposal {
    new_admin: Address,
    proposed_by: Address,
    proposed_at: u64,
    executable_at: u64,
}
```

---

## Pending Re-verification Functions (Issue #208)

### `get_pending_reverify(github_username: String) -> Result<bool, ContractError>`

Returns whether `github_username` has a pending re-verification flag.

The flag is set automatically when a verified user re-registers to a different
Stellar address, invalidating their prior verification. It is cleared once
the record is `verify`'d again. Returns `false` for unknown usernames and
removed users. Works while the contract is paused.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |
| **Errors** | `NotInitialized` |

### `get_pending_reverify_page(offset: u32, limit: u32) -> Result<Vec<String>, ContractError>`

Returns a page of usernames that have a pending re-verification flag set.
`limit` is capped at `MAX_PAGE_LIMIT` (100). Works while the contract is paused.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |
| **Errors** | `NotInitialized` |

**Dashboard sync note:** Poll this endpoint to discover which contributors need
a fresh off-chain GitHub identity check after an address change. See
[DASHBOARD_SYNC.md § Pending Re-verification](DASHBOARD_SYNC.md#pending-re-verification-issue-208).

---

## Attestation-Required Config (Issue #198)

### `set_attestation_required(required: bool) -> Result<(), ContractError>`

Configures whether a published attestation is mandatory before `upgrade`.

When `required = true`, `upgrade` fails with `AttestationRequired` (code 20)
if no valid attestation is published. When `required = false` (default),
unattested upgrades are allowed and provenance notes them as such.

| | |
|---|---|
| **Auth** | Contract admin |
| **Mutates** | Yes |
| **Errors** | `NotInitialized`, `NotAuthorized` |

**Since:** added in Issue #198. Existing deployments default to `false`.

### `is_attestation_required() -> bool`

Returns whether WASM attestation is required before upgrade. Defaults to `false`.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |

---

## batch_remove Return Value (Issue #205)

`batch_remove` returns `BatchSummary` (was previously `void` in the ABI
documentation). Clients that ignored the return value continue to work
without any changes. New clients should use the returned summary to
determine partial-success outcomes without scanning events.

```rust
struct BatchSummary {
    total: u32,       // total items attempted
    successful: u32,  // items removed
    failed: u32,      // items skipped (not registered, etc.)
    success_rate: u32 // integer percentage (0–100)
}
```

**Since:** Issue #205. The function signature in the WASM ABI changed from
`() -> void` to `() -> BatchSummary`. Clients compiled against the old
bindings should regenerate via `make bindings`.

---

## TypeScript Bindings

The Stellar CLI generates a typed client package straight from the deployed
WASM, so the bindings never drift from the contract that produced them.

```bash
make bindings CONTRACT_ID=$CONTRACT_ID NETWORK=testnet
make bindings-build CONTRACT_ID=$CONTRACT_ID NETWORK=testnet   # also installs and compiles
```

| Variable | Default | Purpose |
|----------|---------|---------|
| `CONTRACT_ID` | *(required)* | Deployed contract to read the ABI from |
| `NETWORK` | `testnet` | Network the contract lives on |
| `BINDINGS_DIR` | `bindings/typescript` | Output directory for the generated package |
| `PKG_MANAGER` | `pnpm` | Package manager used by `bindings-build` |

The output directory is git-ignored. Generated bindings are a build artifact,
not source: regenerate them after every deploy rather than committing them.

### Version handshake

Call `is_compatible` once at client startup and fail fast when the deployed
contract is older than the bindings expect:

```ts
const client = new Client({ contractId, networkPassphrase, rpcUrl });

const { result: compatible } = await client.is_compatible({
  major: 1,
  minor: 0,
  patch: 0,
});

if (!compatible) {
  const { result: deployed } = await client.version();
  throw new Error(
    `trustbridge-contract ${deployed.join(".")} is older than this client requires`,
  );
}
```

Read-only calls simulate against RPC and never submit a transaction, so the
handshake costs no fees.

### Regeneration checklist

1. Bump `CONTRACT_VERSION` in `src/lib.rs` for any ABI change.
2. Deploy, then run `make bindings CONTRACT_ID=...`.
3. Bump the minimum version in the consuming client's handshake.

A contract change that alters the ABI without a version bump leaves clients
unable to detect the drift, so the bump is a review blocker.

---

## Cross-Contract Read Interface

Sibling TrustBridge contracts (payout, attestation, and future Waves) can
query registry state directly via Soroban's cross-contract call
(`env.invoke_contract`), instead of duplicating registration/verification
storage of their own. This section is the safe subset of the ABI for that use
case — no new functions were added for it, only a fence around which existing
ones are appropriate to call from another contract's execution context.

### Safe to call cross-contract

All of the following are read-only (no `.set`/`.remove` on storage) and
require **no authorization** beyond the standard `require_not_paused` guard
where noted, so a calling contract needs no signature from the registry's
admin or any registrant to invoke them:

| Function | Returns | Notes |
|---|---|---|
| `get_address(github_username)` | `Option<ContributorRecord>` | Core identity lookup |
| `has_record(github_username)` | `bool` | Cheap existence check, avoids decoding the full record |
| `get_record_proof(github_username)` | `RecordProof` | Existence proof for light clients: verified bit, storage key, TTL policy |
| `get_public_paginated(cursor, limit)` | `Result<ExportPage, ContractError>` | Paginated read; fails with `Paused` while the registry is paused |
| `get_stats()` | `Stats` | `{ total, verified, ever_verified }` |
| `get_verified_count()` | `u32` | Live count; drops on revoke |
| `get_ever_verified_count()` | `u32` | Monotonic; never drops |
| `get_role(address)` | `Option<Role>` | RBAC lookup, e.g. to gate a payout on `Role::Verifier` |
| `is_paused()` / `is_contract_paused()` | `bool` | Check before a call that would otherwise fail on `Paused` |
| `is_registration_in_cooldown(github_username)` | `bool` | |
| `version()` | `(u32, u32, u32)` | |
| `is_compatible(major, minor, patch)` | `bool` | Guard the same way a TypeScript client would (see version handshake above) |
| `max_username_len()`, `is_username_valid(github_username)`, `usernames_match(a, b)` | — | Pure validation helpers, no storage access at all |

`Version::supports_cross_contract_reads()` (`src/version.rs`) gates on this
surface the same way `supports_batch_verify()` gates on the batched
verification entry point, for a caller that wants to assert compatibility
before invoking.

### Not part of this surface: admin exports

`get_all_registered`, `get_registered_page`, and `get_registered_paginated`
call `admin.require_auth()` internally. A cross-contract invocation executes
in the *calling* contract's authorization context — it cannot supply the
registry admin's signature — so these calls will fail auth when invoked from
another contract, regardless of caller identity. Sibling contracts needing a
bulk export must go through the admin's own off-chain tooling, not a
cross-contract call. This is deliberate: the registry has no notion of a
"trusted contract" allowlist, so widening the export surface would mean any
deployed contract could exfiltrate the full registry, not just the intended
consumer.

### Example: a hypothetical payout contract reading verification status

```rust
// From within another contract's implementation:
let registry_id: Address = /* stored sibling contract ID */;
let verified: Option<ContributorRecord> = env.invoke_contract(
    &registry_id,
    &Symbol::new(&env, "get_address"),
    (github_username,).into_val(&env),
);
```

No storage migration is required to adopt this: every function above already
exists in the deployed 1.0.0 ABI, so a v0.1 consumer contract can start
reading today without waiting on a TrustBridge registry upgrade.

---

## Cost and Benchmarks

Every state-changing call consumes ledger CPU instructions and memory. The
benchmark suite lives with the unit tests in `src/lib.rs` under the
`// === Cost benchmarks` section and reports metered cost per operation using
`env.cost_estimate().budget()`.

```bash
make bench              # print CPU/memory cost for every benchmarked operation
make bench-export       # export-only run, results written to bench-results.txt
make bench-max-username # register at the max-length username, written to bench-max-username-register.txt
```

Output is CSV so it can be diffed between branches:

```
operation,size,cpu_instructions,memory_bytes
get_all_registered,1,...,...
get_all_registered,10,...,...
get_all_registered,50,...,...
get_all_registered,100,...,...
```

### What is benchmarked

| Benchmark | Covers |
|-----------|--------|
| `test_bench_export_cpu_cost` | `get_all_registered` at registry sizes 10, 20, 40, 80 |
| `test_bench_username_case_normalization` | `usernames_match` at 10, 50, 100, 200 comparisons (`make bench-username`) |
| `test_bench_core_operation_cpu_cost` | `register`, `get_address`, `get_stats` |
| `test_bench_double_verify_rejection` | Rejected double-verify (`AlreadyVerified`) versus accepted `verify` (Issue #58) |
| `test_bench_failure_path_costs_less_than_success` | Rejected `verify` versus accepted `verify` |
| `test_bench_max_length_username_register` | `register` at a 1-character username versus the maximum accepted length (`MAX_USERNAME_LEN`, currently 39 — read `max_username_len()` rather than hardcoding it) (`make bench-max-username`, Issue #91) |

### Regression guards

Absolute instruction counts shift between `soroban-sdk` releases, so the suite
asserts on shape rather than fixed numbers:

- Export cost is **monotonic** in registry size. A drop means the export stopped
  visiting every record.
- Export cost at the largest size stays within **3x the size ratio** of the
  smallest-size baseline. This passes for a linear scan and fails for quadratic
  growth.
- Username case normalization is **monotonic** in comparison count and obeys the
  same 3x linearity ceiling. Normalization runs on a fixed stack buffer, so a
  regression that introduces per-comparison allocation or a nested scan fails
  here.
- A rejected call costs **strictly less** than the equivalent accepted call, so
  a missing-username lookup cannot become a cheap way to burn ledger budget.
- The max-length username register costs **at least as much** as a
  1-character register, and no more than **5x** that baseline. `register`'s
  extra work for a longer username is a fixed-size copy into the 39-byte
  validation buffer, not a nested or per-character scan, so a wide gap over
  the baseline signals a complexity regression rather than expected growth.
  Since `MAX_USERNAME_LEN` (39) is pinned by an assertion in
  `test_bench_max_length_username_register`, an incompatible change to the
  username length policy fails the benchmark outright instead of silently
  benchmarking a username that no longer represents the worst case.

### Max-length username register — expected range (Issue #91)

`make bench-max-username` prints one CSV line for a 1-character register and
one for a 39-character (`MAX_USERNAME_LEN`) register. As with the other
benchmarks, absolute instruction counts drift between `soroban-sdk` releases —
what matters is the **ratio** between the two lines, which the test enforces
must stay within 5x. Re-run after any change to `register`, `is_valid_github_username`,
or the storage/index write path, and compare the new ratio against the
previous CSV output committed alongside the change (or against
`bench-max-username-register.txt` from the prior run) to catch a regression
before it reaches testnet.

### Caveats

- Benchmarks run in the native test host, not in WASM. Numbers are useful for
  comparing branches and spotting complexity regressions, not for predicting
  exact mainnet fees. Use `stellar contract invoke` against testnet for fee
  estimates.
- The measured section resets the budget to unlimited. This keeps cost tracking
  on while removing the ledger ceiling that a 100-entry export would otherwise
  trip mid-measurement.
- `get_all_registered` reads one ledger entry per record, and Soroban rejects an
  invocation whose footprint exceeds **100 ledger entries**. The export
  benchmark therefore tops out below that ceiling; past roughly 100
  contributors, `get_registered_page` / `get_registered_paginated` is the only
  workable export path.
- `get_all_registered` is admin-only and scans the full index. At large
  contributor counts, prefer event indexing (see
  [EVENT_INDEXING.md](EVENT_INDEXING.md)) over repeated full exports.

### Simulate-register gas reporting

In addition to the in-process benchmark suite, operators can simulate real network
resource consumption using `stellar contract invoke` **without** submitting a transaction
or spending funds.  This is the recommended way to set Wave invoke budgets.

```bash
make simulate-register CONTRACT_ID=$CONTRACT_ID STELLAR_ADDR=$STELLAR_ADDR
# or directly:
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account deployer \
  --network testnet \
  -- register \
  --github-username octocat \
  --stellar-address $STELLAR_ADDR
```

Omitting `--send=yes` triggers simulation mode.  The CLI prints resource fields
including `cpu_instructions`, `mem_bytes`, and `min_resource_fee` (in stroops).

Compare baseline (short username) vs. max-length (39 chars) to see the
username-length delta (Issue #111 — `#77` cross-reference):

```bash
make simulate-register-compare CONTRACT_ID=$CONTRACT_ID STELLAR_ADDR=$STELLAR_ADDR
# Writes both results to simulate-register-results.txt for diffing
```

**Limitations:** simulation fees are approximations — actual fees may differ under
ledger load, after protocol upgrades, or if the simulated footprint differs from the
live footprint.  See [DEPLOYMENT.md#simulate-register-gas-reporting](DEPLOYMENT.md#simulate-register-gas-reporting)
for a full field-by-field interpretation guide and caveats.

---

## GDPR Privacy & Right to Erasure Hook

For GDPR compliance, the contract maps only technical identifiers: a GitHub username string, a Stellar public address, a registration timestamp, and a verification status boolean. 

### Data Inventory
The contract stores no personal identifiable information (PII) like names, email addresses, or phone numbers. All data relating to a user is contained in the `ContributorRecord` struct:
- `stellar_address: Address`
- `registered_at: u32`
- `verified: bool`

### Requesting Export
A user can export their on-chain registry data by invoking `get_address` (publicly available). Admins can export all registrations via `get_all_registered` or page-by-page via `get_registered_paginated`.

### Erasing Data
To fulfill a GDPR "Right to Erasure" request, a user or admin should call the `remove` function. Calling `remove` deletes the user's `ContributorRecord` entry from persistent storage and cleans up the index reference, deleting all trace of the mapping on the active ledger state.

---

## CLI Tips

- Use `--` to separate Stellar CLI flags from contract arguments
- Read-only functions simulate locally — no `--send` needed
- State-changing functions require `--send=yes`
- Run `stellar contract invoke --id $ID -- --help` for auto-generated help from the WASM schema

See also: [Stellar CLI invoke argument types](https://developers.stellar.org/docs/tools/cli/cookbook/contract-invoke-arguments)

---

## New Functions (Issues #207, #210, #212, #214)

### `get_health() -> Result<HealthSnapshot, ContractError>` (Issue #210)

Returns a packed health snapshot for dashboards and CI probes.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |
| **Works while paused** | ✅ |
| **Errors** | `NotInitialized` |

See [CONTRACT_HEALTH.md](CONTRACT_HEALTH.md) for full reference.

---

### `migrate(new_version: (u32, u32, u32)) -> Result<(), ContractError>` (Issue #207)

Advances the schema version and runs applicable data-migration steps.

New in this release: `migrate` now executes registered migration steps, not
just a version bump. The registered step for `v1.0.0 → v1.1.0` rewrites every
`ContributorRecord` to normalise the `registered_at` field layout. Calling
`migrate` twice with the same target version returns `InvalidVersion` (idempotent).

| | |
|---|---|
| **Auth** | Admin |
| **Mutates** | Yes |
| **Errors** | `NotInitialized`, `Paused`, `NotAuthorized`, `InvalidVersion` |

---

### Challenge-period functions (Issue #214)

#### `start_challenge(caller: Address, github_username: String) -> Result<(), ContractError>`

Starts a challenge on a registered username. Locks the name for
`DEFAULT_CHALLENGE_DELAY_SECS` (48 h). Re-registration is blocked while the
challenge is active. Emits `ChallengeStartedEvent`.

| | |
|---|---|
| **Auth** | Admin |
| **Errors** | `NotInitialized`, `Paused`, `NotAuthorized`, `NotRegistered`, `ChallengeAlreadyActive` |

#### `cancel_challenge(caller: Address, github_username: String) -> Result<(), ContractError>`

Cancels a pending challenge, restoring normal registration behaviour.
Emits `ChallengeCancelledEvent`.

| | |
|---|---|
| **Auth** | Admin |
| **Errors** | `NotInitialized`, `Paused`, `NotAuthorized`, `NoChallengeActive` |

#### `complete_challenge(caller: Address, github_username: String) -> Result<(), ContractError>`

Completes a challenge after the delay has elapsed, removing the registration.
Emits `RemovedEvent` and `ChallengeCompletedEvent`.

| | |
|---|---|
| **Auth** | Admin |
| **Errors** | `NotInitialized`, `Paused`, `NotAuthorized`, `NoChallengeActive`, `ChallengeNotResolvable`, `NotRegistered` |

#### `get_challenge(github_username: String) -> Option<ChallengeRecord>`

Returns the active challenge record, or `None`.

| | |
|---|---|
| **Auth** | None |
| **Mutates** | No |

---

## Role enumeration (Issue #228)

### `get_role_holders(offset: u32, limit: u32) -> Vec<RoleHolder>`

Read-only; no auth. One page of `(address, role)` pairs, ordered by grant time
(oldest first).

- `limit` is capped at `MAX_ROLE_PAGE_LIMIT` (50). `0` or any larger value
  yields the cap.
- An `offset` past the end returns an empty page rather than an error.
- **The admin is included** — `initialize` grants `Role::Admin` through the same
  path that maintains the index.
- Revoking a role compacts the index, shifting later entries down one. A caller
  paging with a stored offset should restart if `get_role_holder_count()`
  changed mid-walk.

### `get_role_holder_count() -> u32`

Read-only; no auth. Number of addresses currently holding a role. Lets a caller
size its pagination loop, and gives a dashboard a cheap drift check.

## Network tagging (Issue #231)

### `get_network_tag() -> Option<BytesN<32>>`

Read-only; no auth. The `network_id` recorded at `initialize`, or `None` for an
instance initialized before network tagging existed.

### `adopt_network_tag() -> Result<(), ContractError>`

Admin-only. Tags an untagged instance with the network it is running on, for
instances deployed before this field existed.

Deliberately **not** a re-tagging entry point: if a tag is already present and
disagrees with the live network, this returns `NetworkMismatch` (code 21) rather
than overwriting it. An entry point that could rewrite the tag would defeat the
check entirely. Re-adopting the *same* network is a no-op and succeeds, so a
migration script can call it unconditionally.

### Enforcement

`require_initialized` — which every gated entry point already calls — now also
compares the recorded network id against `env.ledger().network_id()`. A
mismatch returns `NetworkMismatch` (code 21) from **every** gated function,
read or write.

An instance with no recorded tag is allowed through, so contracts deployed
before this change keep working. Once a tag is present it is compared on every
call and never rewritten.
