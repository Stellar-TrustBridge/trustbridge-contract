# Storage Rent Economics

How registry growth translates into storage rent and TTL extension needs for operators running Waves.

Related docs: [ARCHITECTURE](ARCHITECTURE.md) · [DEPLOYMENT](DEPLOYMENT.md) · [ABI](ABI.md)

---

## Background: Soroban Storage Rent

Soroban charges **rent** for persistent ledger state.  Every entry in persistent storage has a
**time-to-live (TTL)** measured in ledger sequence numbers (ledgers).  Once an entry's TTL
reaches zero, it is **archived** — meaning it is removed from the live ledger state and becomes
unreadable until it is explicitly restored.

> **Assumption:** Numbers below are based on Stellar's Mainnet operational parameters as of early
> 2026.  The network charges in stroops per byte per ledger; exact rates are subject to change.
> Always label your own measurements with the network and date.

### Key facts

| Parameter | Approximate value | Notes |
|-----------|------------------|-------|
| Ledger close time | ~5 seconds | Average; can vary |
| Ledgers per day | ~17 280 | 86 400 s ÷ 5 s |
| TTL unit | Ledgers | Not seconds |
| Maximum persistent TTL | 3 110 400 ledgers | ~180 days (network-enforced cap) |
| Minimum TTL after `extend_ttl` | Must exceed the current ledger | Any extension is valid |

---

## Storage Classes in trustbridge-contract

### Instance Storage

Instance storage is tied to the **contract instance** itself.  It lives and dies with the
contract — as long as the contract is alive, all instance-storage keys are accessible.

| Key | Content | Notes |
|-----|---------|-------|
| `Symbol("admin")` | Admin `Address` | Written once at `initialize` |
| `Symbol("count")` | Total registration count (`u32`) | Updated on every `register` / `remove` |
| `Symbol("vcount")` | Verified count (`u32`) | Updated on every `verify` / `revoke_verification` |
| `Symbol("idx")` | Username index `Vec<String>` | Updated on every `register` / `remove` |
| `Symbol("pause")` | Pause state (`bool`) | Written by `pause` / `unpause` |
| `Symbol("cdown")` | Upgrade cooldown seconds (`u64`) | Written by `set_cooldown` |
| `Symbol("lastupg")` | Last upgrade timestamp (`u64`) | Written by `upgrade` |
| `Symbol("ver")` | Contract version `(u32, u32, u32)` | Written by `initialize` / `migrate` |
| `Symbol("role")` + `Address` | Role enum | Written by `set_role` / `remove_role` |

**Who extends the TTL of instance storage?**  Every successful contract invocation extends the
instance TTL automatically.  As long as the contract is called at least once every ~90 days
(matching the `TTL_BUMP` constant), instance storage never expires.  For a Wave that sees
daily activity, this is never a concern.

### Persistent Storage

Persistent storage is **per-entry** and has its own independent TTL.  Each entry must be
extended separately.

| Key pattern | Content | TTL policy |
|-------------|---------|------------|
| `(Symbol("reg"), github_username)` | `ContributorRecord` | Extended by `get_record` / `set_record` via `extend_ttl(TTL_THRESHOLD, TTL_BUMP)` |
| `(Symbol("chunk"), chunk_idx)` | Chunked index slice | Extended when read/written |
| `(Symbol("role"), Address)` | Role assignment | Extended when read/written |
| `(Symbol("lastact"), github_username)` | Last action timestamp | Extended when written |

**`TTL_THRESHOLD` and `TTL_BUMP` (from `src/storage.rs`):**

```rust
pub const LEDGERS_PER_DAY: u32 = 17_280;
pub const TTL_THRESHOLD: u32   = LEDGERS_PER_DAY * 30;  // ~30 days remaining
pub const TTL_BUMP: u32        = LEDGERS_PER_DAY * 90;  // extend to ~90 days
```

A record is only extended when its remaining TTL is below 30 days.  Active records (touched by
`get_address`, `verify`, or `remove`) receive automatic TTL extensions as a side-effect of the
call.  **Cold** records — those that nobody reads or writes for more than 30 days — approach
expiry and must be extended by an off-chain keeper.

---

## Cost Intuition vs. N Users

### Persistent entry size

A single `ContributorRecord` serializes to roughly:

| Field | Approximate XDR size |
|-------|---------------------|
| `stellar_address` (G-address) | ~36 bytes |
| `registered_at` (`u32`) | 4 bytes — downsized from u64 to save 4 bytes/record |
| `verified` (`bool`) | 1 byte |
| XDR framing overhead | ~20 bytes |
| **Total** | **~65 bytes** |

At 5 000 stroops per byte per ledger (approximate, label this assumption), extending one record
by 90 days costs roughly:

```
65 bytes × 5 000 stroops/byte/ledger × (17 280 × 90 ledgers)
= 65 × 5 000 × 1 555 200
≈ 505 billion stroops ≈ 50 500 XLM per record
```

> ⚠️ **This is wrong for typical deployments.**  The actual rent rate on Stellar Mainnet is
> orders of magnitude lower.  Do not use the formula above for budgeting.  Run
> `stellar contract invoke … simulate` against testnet to get real estimates (see
> [simulate-register](#simulate-register) in DEPLOYMENT.md).

The point of the formula is the **shape**, not the numbers: rent is proportional to
`bytes × ledgers`.  Smaller records and shorter TTL extensions cost less.

### Scaling with N contributors

| Registry size | Persistent entries (approx.) | Notes |
|---------------|------------------------------|-------|
| 10 | 10 `ContributorRecord` + 1 chunk | The chunk index also costs rent |
| 100 | 100 records + 2 chunks | One chunk holds up to 50 usernames |
| 1 000 | 1 000 records + 20 chunks | — |
| 10 000 | 10 000 records + 200 chunks | — |

At large scale, the dominant cost is extending the long tail of cold `ContributorRecord`
entries — contributors who registered once and never interacted with the contract again.

---

## Operational Checklist: Wave Timeline

Use this checklist before and during each Wave to avoid rent surprises.

### Before the Wave starts

- [ ] Confirm the contract instance TTL is well above the Wave's end date:
  ```bash
  stellar contract invoke \
    --id $CONTRACT_ID \
    --source-account deployer \
    --network testnet \
    -- get_version
  # If the call succeeds, instance storage is live. Check the ledger explorer
  # for the exact TTL of the contract instance entry.
  ```

- [ ] Identify cold records (not touched in the last 30 days) using your event indexer.
  Any record with `registered_at` older than `now - 30 days` that has not been
  verified, removed, or re-registered is a cold-extension candidate.

- [ ] Estimate the number of cold records and budget for keeper calls.
  A single `extend_registry_ttl` call can extend up to `MAX_PAGE_LIMIT` (200) records
  per invocation (see [ABI.md](ABI.md#extend_registry_ttl)).

- [ ] Set aside XLM for keeper fees.  The exact amount depends on how many cold records
  exist and how much each extension costs at the time of the Wave.

### During the Wave

- [ ] Run the TTL keeper at least every **20 days** (two-thirds of `TTL_THRESHOLD`).
  Waiting until a record is at 1 day remaining risks a race with ledger close times.

- [ ] After each Wave batch of new registrations, check that `get_stats` returns the
  expected count.  A mismatch may indicate archived records.

### After the Wave ends

- [ ] Run one final keeper pass to extend all records to the full `TTL_BUMP` (90 days).
  This gives the next Wave operator time to onboard without emergency TTL work.

- [ ] Archive the keeper call logs so the next operator knows when each record was last extended.

---

## Keeper Implementation

The contract exposes `extend_registry_ttl(usernames: Vec<String>)` (permissionless) as the
on-chain keeper endpoint.  An off-chain job should:

1. Read the full username index via `get_registered_paginated` (admin-only) or `get_public_paginated`.
2. For each username, check whether its remaining TTL is approaching `TTL_THRESHOLD` (30 days).
3. Batch usernames into groups of up to 200 and call `extend_registry_ttl`.

```bash
# Extend a batch of cold records (example)
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account keeper \
  --network testnet \
  --send=yes \
  -- extend_registry_ttl \
  --usernames '["alice","bob","carol"]'
```

The function returns the count of entries actually extended.  A username that is no longer
registered is silently skipped rather than returning an error.

---

## Limitations of Simulation

The `stellar contract invoke … simulate` (or `--send=no`) path returns resource fields
including `cpu_instructions`, `memory`, and `min_resource_fee` without submitting a transaction.
Use this to estimate keeper costs:

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account keeper \
  --network testnet \
  -- extend_registry_ttl \
  --usernames '["alice","bob"]'
# (omit --send=yes)
```

**Known limitations of simulation for rent estimates:**

| Limitation | Impact |
|------------|--------|
| Simulation reads current TTL but does not commit it | The fee shown is valid at simulation time only |
| Rent fees change with network upgrades | Re-simulate after protocol upgrades |
| The `min_resource_fee` is a floor, not a ceiling | The actual fee can be higher if ledger load is elevated |
| Cold records not in the simulator's footprint may have a higher cost | Pre-fetch the entries before estimating |

---

## Related Docs

- [ARCHITECTURE.md](ARCHITECTURE.md) — Storage layout, instance vs. persistent keys
- [DEPLOYMENT.md](DEPLOYMENT.md) — Simulate-register and fee estimation
- [ABI.md](ABI.md) — `extend_registry_ttl`, `get_public_paginated`, `get_registered_paginated`
- [SECURITY.md](SECURITY.md) — TTL extension is permissionless by design
