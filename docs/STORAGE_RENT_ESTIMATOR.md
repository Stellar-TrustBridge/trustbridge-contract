# Storage Rent Estimator — UI Data Spec

Versioned input/output shape for a dashboard UI that estimates **on-chain
Soroban storage rent** for TrustBridge registry growth.

Related docs: [ARCHITECTURE](ARCHITECTURE.md) · [DASHBOARD_SYNC](DASHBOARD_SYNC.md) · [SECURITY](SECURITY.md#storage-ttl) · [DEPLOYMENT](DEPLOYMENT.md)

> **Scope:** this is a **specification** for estimator inputs/outputs, not a
> dashboard implementation.
>
> **Issue #98:** no dedicated storage-economics document is checked in yet.
> When that doc lands, link it here as the narrative companion to this data
> shape. Until then, this file is the machine-readable source of truth for
> Wave budgeting UIs.

---

## Estimator input version

Bump `estimator_inputs_version` whenever the on-chain storage layout or TTL
policy constants change in `src/storage.rs`.

| Field | Current value | When to bump |
|-------|---------------|--------------|
| `estimator_inputs_version` | `1` | Storage key layout, `CHUNK_SIZE`, or TTL constants change |
| `derived_from` | Wave #7 TTL policy + Issue #2 chunked index + instance keys in `src/storage.rs` | Always cite the commit / PR that changed layout |

### Re-computation checklist (layout change)

1. Diff `src/storage.rs` for new/removed keys, `CHUNK_SIZE`, `TTL_THRESHOLD`,
   `TTL_BUMP`, `LEDGERS_PER_DAY`.
2. Update the [canonical constants](#canonical-constants-implementation-facts)
   table below (facts only — no network price guesses).
3. Update the versioned JSON in
   [`storage-rent-estimator.inputs.v1.json`](storage-rent-estimator.inputs.v1.json).
4. Bump `estimator_inputs_version` (v1 → v2, …) and keep the prior JSON file
   for historical Wave budgets.
5. Recompute example tables for N = 1, 10, 100, 500, 1000.
6. Cross-check [ARCHITECTURE.md](ARCHITECTURE.md#storage-layout) and
   [DASHBOARD_SYNC.md](DASHBOARD_SYNC.md).

---

## Separation: on-chain rent vs off-chain indexer storage

| Layer | What it stores | Who pays | In this estimator? |
|-------|----------------|----------|--------------------|
| **On-chain (Soroban)** | Instance keys + persistent entries (`reg`, chunks, roles, `lastact`, …) | Contract / operator via storage rent + TTL extensions | **Yes** |
| **Off-chain indexer / dashboard DB** | Event history, denormalized contributor views, Horizon lag metadata | Operator infra (disk/DB) | **No** — report separately; see [EVENT_INDEXING.md](EVENT_INDEXING.md) |

Do **not** mix indexer disk growth into the on-chain rent chart. UIs should
render two independent series:

1. `on_chain_rent_estimate` (this spec)
2. `off_chain_indexer_storage_estimate` (operator-defined; out of scope here)

---

## Canonical constants (implementation facts)

These are **facts** derived from the intended Wave #7 / Issue #2 storage policy
documented alongside `src/storage.rs`. Label anything else as an assumption.

| Constant | Value | Meaning |
|----------|-------|---------|
| `LEDGERS_PER_DAY` | `17_280` | ≈ 5 s ledger close |
| `TTL_THRESHOLD` | `LEDGERS_PER_DAY * 30` (= `518_400`) | Extend when remaining TTL drops below ~30 days |
| `TTL_BUMP` | `LEDGERS_PER_DAY * 90` (= `1_555_200`) | Restore TTL to ~90 days from current ledger |
| `CHUNK_SIZE` | `100` | Usernames per persistent index chunk (Issue #2 / DASHBOARD_SYNC) |
| `DEFAULT_PAGE_LIMIT` | `20` or `50` (see notes) | Export default when `limit == 0` — **not a rent driver** |
| `MAX_PAGE_LIMIT` | `100` | Export page cap — **not a rent driver** |

### TTL extension schedule (fact)

Persistent entries call `extend_ttl(key, TTL_THRESHOLD, TTL_BUMP)` on read/write
of records and chunks (`get_record` / `set_record` / chunk helpers), and via the
permissionless keeper `extend_registry_ttl(usernames)`.

| Phase | Behavior |
|-------|----------|
| Hot path | Reads/writes bump when remaining TTL &lt; threshold |
| Keeper path | Off-chain job supplies username lists; contract extends without deserializing full export |
| Cadence (ops) | Schedule keeper runs so cold records are bumped before archival — see [DEPLOYMENT.md](DEPLOYMENT.md) / [SECURITY.md](SECURITY.md#storage-ttl) |

**Assumption (ops, not encoded on-chain):** operators run a keeper at least
every ~25–30 days for cold usernames so entries do not approach archival.

---

## Cost drivers vs N users

Let `N` = number of live registered contributors (`count`).

### Per-user persistent keys (on-chain)

| Key pattern | Count vs N | Notes |
|-------------|------------|-------|
| `(Symbol("reg"), username)` → `ContributorRecord` | **N** | Primary rent driver |
| `(Symbol("lastact"), username)` | **0..N** | Present only after cooldown tracking writes; treat as optional |
| `(Symbol("role"), Address)` | **R** (role holders, not N) | Admin + Verifier + Upgrader assignments; usually ≪ N |

### Chunked index (persistent)

| Driver | Formula | Notes |
|--------|---------|-------|
| Chunk entries | `ceil(N / CHUNK_SIZE)` | Key `(Symbol("chunk"), chunk_idx)` |
| Chunk count key | `1` instance entry | `chunkcnt` / chunk-count symbol |

Dual-write note: registration also maintains the legacy instance `idx`
`Vec<String>` (full username list). That is **instance** overhead that grows
with N, separate from chunk rent.

### Instance overhead (shared, not per-user)

| Key | Type | Scales with N? |
|-----|------|----------------|
| `admin` | `Address` | No |
| `count` / `vcount` | `u32` | No (values change; entry count fixed) |
| `idx` | `Vec&lt;String&gt;` | **Yes — payload size grows with N** |
| `pause` | `bool` | No |
| `cdown` | `u64` | No |
| `lastupg` | `u64` | No |
| `ver` | version tuple | No |
| chunk count | `u32` | No |
| provenance / attestation (when used) | structs | No |

### Scaling table (entry counts — not XLM)

| N users | `reg` entries | Index chunks (`ceil(N/100)`) | Instance keys (fixed + `idx`) | Optional `lastact` (worst case) |
|--------:|-------------:|-----------------------------:|-------------------------------:|--------------------------------:|
| 1 | 1 | 1 | fixed set + small `idx` | 0–1 |
| 10 | 10 | 1 | fixed + `idx` | 0–10 |
| 100 | 100 | 1 | fixed + `idx` | 0–100 |
| 500 | 500 | 5 | fixed + larger `idx` | 0–500 |
| 1000 | 1000 | 10 | fixed + large `idx` | 0–1000 |

**Assumption:** converting entry counts → XLM requires network rent parameters
from the current Stellar protocol (write fee, rent rate, TTL). Those prices are
**not** hard-coded in this contract; the UI must inject them as
`network_rent_params` (see JSON). Mark price fields as `assumption` /
`operator_supplied`.

---

## UI-consumable estimator input shape

Machine-readable copy:
[`docs/storage-rent-estimator.inputs.v1.json`](storage-rent-estimator.inputs.v1.json).

Logical schema:

```json
{
  "estimator_inputs_version": 1,
  "on_chain": {
    "ttl": {
      "ledgers_per_day": 17280,
      "threshold_ledgers": 518400,
      "bump_ledgers": 1555200,
      "threshold_days_approx": 30,
      "bump_days_approx": 90
    },
    "layout": {
      "chunk_size": 100,
      "per_user_persistent_keys": ["reg"],
      "optional_per_user_persistent_keys": ["lastact"],
      "per_address_persistent_keys": ["role"],
      "instance_keys": ["admin", "count", "vcount", "idx", "pause", "cdown", "lastupg", "ver", "chunkcnt"]
    },
    "cost_drivers": {
      "reg_entries": "N",
      "chunk_entries": "ceil(N / chunk_size)",
      "instance_idx_payload": "grows_with_N",
      "role_entries": "R",
      "lastact_entries": "0..N"
    }
  },
  "off_chain_indexer": {
    "included_in_on_chain_rent": false,
    "guidance": "Estimate event/index DB size separately; see EVENT_INDEXING.md"
  },
  "network_rent_params": {
    "source": "operator_supplied",
    "note": "Inject current protocol rent/write fees; not stored in the contract"
  },
  "assumptions": [
    "Ledger close ≈ 5s → 17280 ledgers/day",
    "Keeper extends cold records before archival (~every 25–30 days)",
    "Worst-case lastact assumes every user has a cooldown timestamp entry",
    "Role count R is independent of N and usually small"
  ]
}
```

### Suggested UI outputs

| Output field | Formula sketch |
|--------------|----------------|
| `persistent_entry_count(N)` | `N + ceil(N/CHUNK_SIZE) + R + lastact_estimate` |
| `instance_overhead` | fixed keys + `idx` size model |
| `ttl_extension_schedule` | threshold 30d / bump 90d (+ keeper cadence assumption) |
| `on_chain_rent_estimate` | `f(entry_counts, network_rent_params, ttl)` |
| `off_chain_indexer_storage_estimate` | **separate**; not from this JSON |

---

## Example Markdown table for Wave budgeting

| N | Persistent `reg` | Chunks | Approx persistent total (ex-roles, ex-lastact) | TTL policy |
|--:|-----------------:|-------:|-----------------------------------------------:|------------|
| 50 | 50 | 1 | 51 | bump to ~90d when &lt; ~30d remain |
| 100 | 100 | 1 | 101 | same |
| 250 | 250 | 3 | 253 | same |
| 1000 | 1000 | 10 | 1010 | same |

Multiply by operator-supplied rent params to get currency. Do not hard-code XLM
in the UI against this table alone.

---

## Cross-links

- Storage layout narrative: [ARCHITECTURE.md](ARCHITECTURE.md#storage-layout)
- Dashboard sync / chunk size: [DASHBOARD_SYNC.md](DASHBOARD_SYNC.md)
- TTL ops: [SECURITY.md](SECURITY.md#storage-ttl), [DEPLOYMENT.md](DEPLOYMENT.md)
- Event/indexer (off-chain): [EVENT_INDEXING.md](EVENT_INDEXING.md)
