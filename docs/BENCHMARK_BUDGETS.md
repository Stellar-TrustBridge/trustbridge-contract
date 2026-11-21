# Benchmark Budgets & Recommended Page Sizes

Operational guidance for dashboard and indexer consumers of the paginated
registry reads. It records the contract's hard limits, a recommended page size
for each reader, and the failure symptoms that mean "page too big".

Related: [DASHBOARD_SYNC](DASHBOARD_SYNC.md) · [ABI](ABI.md) ·
[STORAGE_FOOTPRINT](STORAGE_FOOTPRINT.md) · `src/storage.rs`, `src/lib.rs`.

---

## Hard limits (contract-enforced)

These are constants in `src/storage.rs` and are part of the ABI; they are
**not** network tuning knobs:

| Constant | Value | Behavior |
|----------|------:|----------|
| `DEFAULT_PAGE_LIMIT` | `20` | Used when the caller sends `limit = 0` |
| `MAX_PAGE_LIMIT` | `100` | `limit > 100` is **clamped** to `100`, not rejected |
| `CHUNK_SIZE` | `50` | Usernames per persistent chunk entry (`(chunk, idx)`) |

The full index (`idx`) is one instance-storage entry that grows linearly with
the registry size, while each returned row also requires a persistent
`(reg, username)` read. A page is therefore bounded by both the instruction
budget and the ledger-footprint budget, not by `MAX_PAGE_LIMIT` alone. See
[STORAGE_FOOTPRINT.md](STORAGE_FOOTPRINT.md#estimated-footprint-by-registry-size)
for the sizing model and the ~3,259-entry crossover point of the legacy `idx`
entry.

---

## Recommended page sizes

| Consumer | Endpoint | Recommended `limit` | Rationale |
|----------|----------|--------------------:|-----------|
| Public dashboard poll | `get_public_paginated` | `20` (`DEFAULT_PAGE_LIMIT`) | Smallest footprint, no admin key, cheap to retry; blocks while paused |
| Indexer backfill / reconciliation | `get_public_paginated` | `50` (`CHUNK_SIZE`) | Aligns one page with one persistent chunk entry |
| Admin export / audit snapshot | `get_registered_paginated` | `50` | Same footprint as above, plus one admin signature per page |
| Recovery from a suspected OOG | Either | `10`, then increase | Establish a working baseline before tuning upward |

`100` is the ceiling, not the target. Use it only after a successful
`make bench-export` on a comparable registry size. A request above `100`
silently returns `100`, so an apparently "ignored" page size is the clamp, not
a server bug.

CLI examples:

```bash
# Public dashboard poll (default page)
stellar contract invoke --id $ID --network testnet \
  -- get_public_paginated --cursor 0 --limit 20

# Admin export aligned to one chunk
stellar contract invoke --id $ID --source-account admin --network testnet \
  -- get_registered_paginated --cursor 0 --limit 50
```

Always follow `next_cursor` until `has_more == false` / `next_cursor == null`;
never increase `limit` to compensate for a stalled cursor.

---

## Failure symptoms of an oversized page

| Symptom | Meaning | Action |
|---------|---------|--------|
| `Budget exceeded` / `Exceeded resource limit` / instruction-limit error | CPU budget exhausted while materialising the page | Halve `limit`, retry with the same cursor |
| Ledger footprint / `max_contract_data_entry_size` error | Too many distinct ledger entries touched | Drop to `20`, then walk chunks one at a time |
| RPC timeout or 5xx while returning a large page | Response/invoke budget exceeded off-chain | Use `20`, increase client timeout, keep the cursor loop |
| Page shorter than requested without `has_more` | Registry exhausted, or a stale cursor | Trust `has_more`/`next_cursor`, re-read `get_stats()` |
| `Paused` on `get_public_paginated` | Circuit breaker active, not a budget issue | Retry after unpause; `get_address` remains readable |
| Requested 150, received 100 | `MAX_PAGE_LIMIT` clamp | Set the limit explicitly; split into two pages |

An oversized page fails the **whole** invocation — it never returns a partial
page. Keep the last successful cursor as the retry point.

---

## Reproducing the budget numbers

No absolute instruction counts are pinned here: they move with `soroban-sdk`
releases. The suite asserts on shape (monotonic cost, failure paths cheaper
than success) so a regression is caught without brittle constants.

```bash
make bench              # CPU/memory per operation, CSV output
make bench-export       # export-only run → bench-results.txt
make bench-username     # case-normalization cost → bench-username-normalization.txt
```

The benchmarks live in `src/lib.rs` under `// === Cost benchmarks`; see
[ABI.md § Cost and Benchmarks](ABI.md#cost-and-benchmarks) for the full list.
Revisit the recommended sizes above if `CHUNK_SIZE`, `MAX_PAGE_LIMIT`, the
`ContributorRecord` shape, or the target network's resource config changes.
