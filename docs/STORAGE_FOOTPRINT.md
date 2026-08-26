# Instance Storage Footprint Report

Design-only estimate of the registry's on-chain storage footprint at
representative sizes, so maintainers have a planning number for rent budgets
and for when the contract needs an upgrade to change its storage strategy.
No code changes; a follow-up can wire up automated measurement (e.g. a
`cargo test` that reads `env.budget()` / ledger snapshot sizes at seeded
registry sizes) against the estimates below.

Related: [ARCHITECTURE.md](ARCHITECTURE.md) (storage layout) ·
[ABI.md](ABI.md) (function reference) · `src/storage.rs`, `src/events.rs`
(implementation).

---

## Methodology

Soroban has two storage classes relevant here, and they behave very
differently as the registry grows:

- **Instance storage** (`env.storage().instance()`). All instance keys for a
  contract live together as *one* ledger entry (a key→value map attached to
  the contract instance) — not one entry per key. That shared entry has a
  network-enforced maximum size, so every instance key competes for the same
  budget.
- **Persistent storage** (`env.storage().persistent()`). Each key is its own
  ledger entry with its own TTL, individually rent-paying and individually
  extendable via `extend_ttl`.

The registry currently writes:

| Key | Class | Grows with N? | Holds |
|---|---|---|---|
| `admin`, `count`, `vcount`, `paused`, `cdown`, `lastupg`, `ver`, `chunkcnt`, `prov`, `attest` | instance | No — fixed count | Scalars + the two WASM-provenance structs |
| `idx` | instance | **Yes — linearly** | `Vec<String>` of *every* registered username, in one entry |
| `(reg, username)` | persistent | Yes — one entry per registration | `ContributorRecord` |
| `(chunk, chunk_idx)` | persistent | Yes — one entry per 100 usernames | `Vec<String>` slice (`CHUNK_SIZE = 100`, `src/storage.rs`) |
| `(role, address)` | persistent | No — scales with privileged addresses, not N | `Role` |
| `(lastact, username)` | persistent | Only for usernames with a recorded cooldown action | `u64` timestamp |

`idx` and the `(chunk, *)` entries are two independent indexes over the same
usernames, maintained in parallel by `add_to_index` / `remove_from_index`
(`src/storage.rs`) — `idx` is the legacy flat list `get_all_registered` and
`get_index_page` read; the chunked index backs bounded paging. Both are
counted below.

### Assumptions

These are the knobs to redo this estimate against if the schema, chunk size,
or the target network's resource config changes:

- **Average GitHub username length:** 12 bytes (GitHub's cap is 39; 12 is a
  representative average, not a worst case — redo with 39 for a worst-case
  bound).
- **String XDR cost:** ~8 bytes overhead (length prefix + alignment padding)
  on top of payload bytes.
- **Address XDR cost:** ~40 bytes serialized (32-byte key plus XDR framing).
- **Scalar XDR cost:** ~8 bytes for a `u32`/`bool`, ~12 bytes for a `u64`,
  ~16 bytes for the `(u32,u32,u32)` version tuple.
- **Per-entry envelope overhead** (persistent only): ~40 bytes for the
  `LedgerEntry`/`LedgerKey` framing that individual persistent entries pay
  once, independent of payload size. Instance storage pays this once total
  for the whole map, not per key, which is folded into the fixed-key total
  below.
- **Ledger entry size ceiling:** commonly cited as up to 64 KiB
  (65,536 bytes) per entry on current mainnet/testnet resource configs. This
  is a network parameter, not a contract constant — reconfirm against the
  target network's `max_contract_data_entry_size_bytes` (or equivalent
  current config) before treating the crossover point below as exact.
- **Role grants:** 1 (the initial admin — `initialize` calls `set_role` for
  it). Each additional `set_role` call adds one fixed-size persistent entry,
  independent of N.
- **Cooldown tracking (`lastact`):** 0 by default. These entries are only
  created when `record_action` is called for a username, so they track a
  subset of active users, not all N — add `52 bytes × (tracked usernames)`
  if cooldown enforcement is in active use.
- **All N registrations are live** (none removed). A `remove` frees its
  `(reg, *)` and chunk-slot bytes but the flat `idx` rewrite cost (see below)
  is the same regardless of net growth or churn.

### Per-key size formulas

| Key | Formula (bytes) |
|---|---|
| Fixed instance scalars (`admin`…`attest`, minus `idx`) | ≈ 356 total (dominated by the two WASM structs, ~250 B of the 356) |
| `idx` (instance, one entry, holds all N) | ≈ `4 + 20·N` (vec length prefix + N × (8 B string overhead + 12 B avg payload)) |
| `(reg, username)` (persistent, N entries) | ≈ `110` each (`ContributorRecord`: 40 B address + 12 B u64 + 4 B bool + struct/XDR overhead, plus the 40 B entry envelope) |
| `(chunk, idx)` (persistent, `⌈N/100⌉` entries) | ≈ `44 + 20·(entries in this chunk)`, i.e. ~2,044 B for a full 100-entry chunk |
| `(role, address)` (persistent, R entries) | ≈ `92` each |

---

## Estimated footprint by registry size

| N (contributors) | Instance entry (`idx` + fixed) | Persistent `reg` total | Persistent `chunk` total | Persistent `role` total (R=1) | **Total on-chain bytes** |
|---:|---:|---:|---:|---:|---:|
| 0 | 356 B | 0 B | 0 B | 92 B | **448 B** |
| 10 | 556 B | 1,100 B | 244 B | 92 B | **≈ 1.99 KB** |
| 100 | 2,356 B | 11,000 B | 2,044 B | 92 B | **≈ 15.1 KB** |
| 1,000 | 20,356 B | 110,000 B | 20,440 B | 92 B | **≈ 147.4 KB** |
| ~3,259 | ≈ 65,536 B | 358,490 B | 66,708 B | 92 B | **≈ 480.6 KB** |

The last row is not an arbitrary large-N sample — it is the point where the
**`idx` instance entry itself hits the assumed 64 KiB ceiling**
(`(65,536 − 356) / 20 ≈ 3,259`). Past that, `register()` would start failing
on *any* new username purely because the shared instance entry can no longer
grow, regardless of how much persistent-storage rent the registry is willing
to pay. This is the operationally important number from this report: it is
a hard capacity ceiling on the current design, not just a rent curve, and it
is driven entirely by the legacy flat `idx` — the chunked index has no such
single-entry limit, since it splits growth across many persistent entries.

**Mitigation, for a future issue:** stop writing `idx` once dashboard/indexer
consumers are migrated to `get_registered_page` / the chunked index for
enumeration, and keep `idx` only if something still depends on
`get_all_registered`'s exact ordering guarantee.

---

## On-chain rent vs. indexer event volume

These are separate concerns with different growth drivers, and both matter
for a "planning number":

- **On-chain rent** (table above) is a function of the *current* registry
  size N. Removed registrations shrink it; TTL extension (`extend_registry_ttl`,
  the per-record and per-chunk `extend_ttl` calls in `src/storage.rs`) keeps
  live entries from archiving but does not add new state.
- **Indexer event volume** is a function of *cumulative mutating calls over
  time* — `RegisteredEvent`, `VerifiedEvent`, `VerificationRevokedEvent`,
  `RemovedEvent`, `RoleGrantedEvent`, `RoleRevokedEvent`, `PausedEvent`,
  `UnpausedEvent`, `UpgradedEvent` (`src/events.rs`) — not of N. A registry
  that churns (many registrations followed by removals) can have a small N
  but a large historical event count.
- Soroban events are **not contract state**: they are not stored in the
  entries above and do not consume contract rent. They live on the ledger's
  event stream and are pruned by validators after the network's configured
  retention window (on the order of days, network-config-dependent — verify
  the current value before relying on it). Anything the dashboard needs to
  query beyond that window must already be captured by an off-chain indexer
  in its own storage (Postgres, a data warehouse, etc.), which is indexer
  infrastructure, not part of this contract's rent budget.
- Rough sizing for indexer-side planning: each event's topics + data are
  small (a `github_username`, an `Address`, and a `u64` timestamp, i.e. the
  same ~60 bytes of payload as the per-entry formulas above, plus whatever
  envelope the indexer's own storage format adds). At M mutating calls over
  the contract's lifetime, indexer storage is roughly `M × (60–150 bytes)`
  depending on the indexer's own schema — independent of the 64 KiB on-chain
  ceiling discussed above.

---

## Recomputing these numbers

Redo this estimate if any of the following change:

- `ContributorRecord`, `WasmProvenance`, or `WasmAttestation` gain/remove
  fields.
- `CHUNK_SIZE` (`src/storage.rs`) changes from 100.
- The average/expected username length assumption changes materially from
  12 bytes.
- The target network's maximum ledger entry size changes from the assumed
  64 KiB.
- `idx` is removed in favor of the chunked index alone (removes the N-linear
  instance-entry risk entirely; recompute the "Instance entry" column as the
  fixed ~356 B row only, with no crossover point).

---

## Index Compaction and Storage Footprint (Issue #209)

After a sequence of `remove` calls the chunked index (`(chunk, N)` persistent
entries) can become sparse. Each removed username leaves its chunk slot empty
but the chunk entry itself remains — it is not deleted until the chunk is
entirely empty. Over a full Wave season (hundreds of contributors registering
and some percentage leaving) this means:

- Chunk entries stay allocated even when most of their usernames have been
  removed.
- Pagination skips empty slots, but each access still pays for the ledger
  entry reads.
- Net effect: storage rent for chunks grows with the *peak* registration
  count, not the *current* count.

### `compact_index` (admin operation)

`compact_index` (`src/lib.rs`) rebuilds the chunked index densely from the
current flat `idx` list. It:

1. Deletes all existing `(chunk, N)` persistent entries.
2. Re-partitions the current `idx` list into full `CHUNK_SIZE` chunks plus a
   single partial tail.

After compaction the number of chunk entries equals
`ceil(current_count / CHUNK_SIZE)`, which is the theoretical minimum.
This reclaims one persistent entry per `CHUNK_SIZE` slots that were entirely
empty.

**When to run:** After a bulk `batch_remove` operation at the end of a Wave
season, or any time `get_stats().total` drops significantly below the peak
registration count that created the existing chunks.

**Instruction budget:** Compaction reads the flat `idx` entry once, deletes
`old_chunk_count` persistent entries, and writes `new_chunk_count` entries.
For a registry of N users with C chunks, this is roughly
`(N / CHUNK_SIZE) + (old_C + new_C)` storage operations — well within the
Soroban per-transaction instruction limit for registries up to ~50,000 users
at the current `CHUNK_SIZE = 50`.

**Tests:** `tests/integration.rs` under the `// Issue #209` section; run with:

```bash
cargo test compact
```
