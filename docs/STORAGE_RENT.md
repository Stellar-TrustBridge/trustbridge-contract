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

## Operational Revival Check

Run the TTL-focused integration tests before changing storage or archival code:

```bash
cargo test ttl
```

The tests use the Soroban host ledger APIs to move a real persistent record close to its
live-until ledger.  They then exercise both revival paths: reading the record through
`get_address`, and calling `extend_registry_ttl`.  Each test advances beyond the old TTL and
asserts that the record is still present.  This is intentionally different from a mocked success
check: the persistent key's host TTL is observed with `get_ttl` before and after the operation.

For an operational deployment, run the keeper before records reach the threshold and inspect its
exit status and logs:

```bash
CONTRACT_ID=$CONTRACT_ID SOURCE=keeper NETWORK=testnet \
  ./scripts/ttl_keeper.sh
```

The keeper is permissionless at the contract layer, but its signing identity still needs enough
network funds for the submitted transaction.  Run it at least every 20 days, use a small batch
while validating a new deployment, and confirm the transaction succeeded before moving on.  A
keeper cannot revive a key after it has already been archived; use Soroban's explicit restore
workflow for archived entries and then rerun validation.  Do not treat a successful simulation as
proof that a live TTL was extended.

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
  A single `extend_registry_ttl` call can extend up to **100 records**
  per invocation (`BatchConfig::default().max_batch_size` — see
  [ABI.md §extend_registry_ttl](ABI.md#extend_registry_ttlusernames-vecstring---resultu32-contracterror)).

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

## Operator Playbook: TTL Extension (Issue #660)

Step-by-step procedure for keeping every registry entry live across a Wave.
An expired `reg` or `chunk` entry mid-Wave makes `get_public_paginated` and
`get_address` fail for that contributor, which bricks the dashboard's reads.

### Policy constants

The table below is machine-checked against `src/storage.rs` and
`src/batch.rs` by `storage_rent_playbook_tests` (run `make storage-rent-check`),
so it cannot drift from the contract. Estimator inputs in
[STORAGE_RENT_ESTIMATOR.md](STORAGE_RENT_ESTIMATOR.md#canonical-constants-implementation-facts)
use the same values.

<!-- rent-playbook:begin -->
| Constant | Ledgers / count | Meaning |
|---|---:|---|
| `LEDGERS_PER_DAY` | 17280 | ≈ 5 s ledger close |
| `TTL_THRESHOLD` | 518400 | Bump when remaining TTL drops below ~30 days |
| `TTL_BUMP` | 1555200 | Each bump extends to ~90 days ahead of the current ledger |
| `KEEPER_INTERVAL` | 345600 | Run the keeper at least every ~20 days (2/3 of `TTL_THRESHOLD`) |
| `MAX_KEEPER_BATCH` | 100 | Usernames per `extend_registry_ttl` call (`BatchConfig::default().max_batch_size`) |
<!-- rent-playbook:end -->

### Steps

1. **Size the job.** Read `total` from one `get_public_paginated(None, 1)` call
   and divide by `MAX_KEEPER_BATCH` to get the number of keeper transactions.
   Plug `total` into the estimator
   ([STORAGE_RENT_ESTIMATOR.md](STORAGE_RENT_ESTIMATOR.md#ui-consumable-estimator-input-shape))
   to budget XLM for the rent the bumps will pay.
2. **Dry-run.** `make ttl-keeper CONTRACT_ID=C... SOURCE=keeper TTL_KEEPER_DRY_RUN=true`
   walks the registry and prints the batches without sending anything.
3. **Extend.** `make ttl-keeper CONTRACT_ID=C... SOURCE=keeper NETWORK=testnet`.
   `SOURCE` only has to pay fees — `extend_registry_ttl` is permissionless and
   works while the contract is paused, so a pause never blocks rent upkeep.
4. **Check the result.** The keeper exits non-zero if any batch failed. Each
   call returns the number of entries actually extended; a value lower than the
   batch length means some usernames were removed since the index was read
   (skipped, not an error).
5. **Spot-check a cold record.** Read `liveUntilLedgerSeq` for
   `["reg","<username>"]` (see [Reading a record's TTL](#reading-a-records-ttl-from-outside-issue-230));
   it should be roughly `current ledger + TTL_BUMP`.
6. **Schedule the next run** no later than `KEEPER_INTERVAL` ledgers from now.

### Failure handling

| Symptom | Cause | Action |
|---|---|---|
| `InvalidBatchSize` | Empty batch, or batch over `MAX_KEEPER_BATCH` | Let the keeper read the cap from `src/batch.rs`; lower `TTL_KEEPER_BATCH_SIZE` only to go smaller |
| `NotInitialized` | Wrong `CONTRACT_ID` or an uninitialized instance | Confirm the id against the deployment record before retrying |
| Returned count is `0` for a batch of live users | Entries are already **archived** | `extend_ttl` cannot touch archived entries. Restore them first (`stellar contract restore --durability persistent --key '["reg","<username>"]'`), then rerun the keeper |
| Instance entry archived (every call fails) | Instance TTL lapsed | Restore the contract instance, then rerun; instance keys share one TTL, see [Instance Storage](#instance-storage) |
| Network / RPC error | Transient | Rerun — extensions are idempotent, a repeated batch only re-bumps entries already above `TTL_THRESHOLD` (no-op) |

---

## Keeper Implementation

The contract exposes `extend_registry_ttl(usernames: Vec<String>)` (permissionless) as the
on-chain keeper endpoint. An off-chain job should:

1. Read the full username index via `get_registered_paginated` (admin-only) or `get_public_paginated`.
2. For each username, check whether its remaining TTL is approaching `TTL_THRESHOLD` (30 days).
3. Batch usernames into groups of up to **100** and call `extend_registry_ttl`.

The batch size limit is `BatchConfig::default().max_batch_size` (`src/batch.rs`). See
[ABI.md §extend_registry_ttl](ABI.md#extend_registry_ttlusernames-vecstring---resultu32-contracterror)
for the complete specification, and
[tests/extend_registry_ttl.rs](../tests/extend_registry_ttl.rs) for the
contract-side behaviour these batches rely on.

> **Do not copy the number into a keeper.** `scripts/ttl_keeper.sh` reads the
> cap out of `src/batch.rs` at startup and refuses to run if it cannot. A
> hard-coded batch size keeps sending oversized batches after the contract
> tightens its cap, and every one comes back `InvalidBatchSize` — a keeper that
> appears to run and extends nothing, while the records expire silently.

### Partial batches and expiry generations (Issue #669)

`extend_registry_ttl` never leaves a batch half-applied:

- **Unregistered usernames are skipped**, not fatal. Every registered username
  in the same call is still extended, so all of them land in one expiry
  generation (`TTL_BUMP` ledgers from the current ledger). The return value
  counts only the extended entries — compare it with the batch length to spot
  names the keeper's off-chain list should drop.
- **A rejected call extends nothing.** `NotInitialized` and `InvalidBatchSize`
  are returned before any entry is touched, so a keeper retrying after a
  rejection never sees mixed TTLs from the failed attempt.

Locked by `test_extend_registry_ttl_partial_batch_leaves_single_expiry_generation`
and `test_extend_registry_ttl_rejected_batch_extends_nothing` in
[tests/extend_registry_ttl.rs](../tests/extend_registry_ttl.rs).

### Running the keeper

`scripts/ttl_keeper.sh` is the reference implementation of the loop above. Run
it through the Makefile rather than directly, so the contract id, source
identity and network come from the same variables as every other target:

```bash
# Preview the batches without sending anything
make ttl-keeper CONTRACT_ID=C... SOURCE=keeper TTL_KEEPER_DRY_RUN=true

# Extend for real
make ttl-keeper CONTRACT_ID=C... SOURCE=keeper NETWORK=testnet

# Smaller batches, e.g. when the per-transaction budget is tight
make ttl-keeper CONTRACT_ID=C... SOURCE=keeper TTL_KEEPER_BATCH_SIZE=25
```

The keeper is permissionless, so `SOURCE` only needs to be an identity that can
pay transaction fees — not the admin.

It exits non-zero if any batch failed. That matters when it runs unattended on
a timer: exiting 0 after every batch failed would report a successful run while
the records quietly expire.

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

## Reading a record's TTL from outside (Issue #230)

`get_record_proof(github_username)` returns a `RecordProof`:

| Field | Meaning |
|---|---|
| `exists` | Whether a record is stored for this username |
| `verified` | The record's verified flag; `false` when `exists` is `false` |
| `registered_at` | Ledger timestamp the record was last written |
| `as_of_ledger` | Ledger sequence the proof was taken at |
| `ttl_threshold_ledgers` | Remaining TTL below which the entry is bumped (`TTL_THRESHOLD`) |
| `ttl_bump_ledgers` | How far ahead of the current ledger a bump extends it (`TTL_BUMP`) |
| `key_prefix` | Symbol half of the storage key |

It deliberately does **not** return `liveUntilLedgerSeq`. A contract cannot read
its own entry's TTL on-chain — the host exposes `get_ttl` only to test
utilities — so any number the contract reported would be inferred rather than
observed, and would drift from the ledger the moment an extension landed.

Read the real value from the ledger entry instead. The record's key is
`(key_prefix, github_username)`, i.e. the `reg` symbol paired with the
username, in **persistent** storage:

```bash
stellar contract read --id $ID --network testnet --durability persistent \
  --key '["reg","octocat"]'
```

Over RPC, `getLedgerEntries` returns `liveUntilLedgerSeq` for the same key. A
light client can then compare it against `as_of_ledger` from the proof and use
`ttl_threshold_ledgers` / `ttl_bump_ledgers` to predict when the next read will
bump the entry.

If the entry has been archived, restore it with that same key
(`stellar contract restore`) before calling
[`extend_registry_ttl`](ABI.md) — extending skips archived entries (see the
[playbook](#operator-playbook-ttl-extension-issue-660)).

---

## Related Docs

- [ARCHITECTURE.md](ARCHITECTURE.md) — Storage layout, instance vs. persistent keys
- [DEPLOYMENT.md](DEPLOYMENT.md) — Simulate-register and fee estimation
- [ABI.md](ABI.md) — `extend_registry_ttl`, `get_public_paginated`, `get_registered_paginated`
- [SECURITY.md](SECURITY.md) — TTL extension is permissionless by design
- [`scripts/ttl_keeper.sh`](../scripts/ttl_keeper.sh) — reference keeper, run via `make ttl-keeper`
- [`Makefile`](../Makefile) — the `ttl-keeper` target and its variables
- [`tests/extend_registry_ttl.rs`](../tests/extend_registry_ttl.rs) — contract-side batch behaviour
