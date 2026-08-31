# TrustBridge Registry Subgraph (Issue #284)

The dashboard currently polls RPC directly. This directory is a **schema +
mapping spec** so Wave UIs can query contributor history from a subgraph instead
of writing an indexer from scratch. Operating a hosted indexer is out of scope
for this repo — [`scripts/event_indexer.sh`](../../scripts/event_indexer.sh)
remains the runnable local reference.

- [`schema.graphql`](schema.graphql) — entities for `RegisteredEvent`,
  `VerifiedEvent`, `RemovedEvent`, plus a derived `Contributor` aggregate.

## Event → entity mapping

Field names and types below are copied from the on-chain `#[contractevent]`
definitions in [`src/events.rs`](../../src/events.rs). Do not add payload fields
the contract does not emit.

| Contract event | Topic symbol | Payload fields (from `src/events.rs`) | Entity |
|---|---|---|---|
| `RegisteredEvent` | `registered_event` | `github_username` (topic), `stellar_address`, `timestamp`, `sponsor: Option<Address>` | `RegisteredEvent` |
| `VerifiedEvent` | `verified_event` | `github_username` (topic), `stellar_address`, `timestamp`, `domain` | `VerifiedEvent` |
| `RemovedEvent` | `removed_event` | `github_username` (topic), `stellar_address`, `timestamp`, `domain` | `RemovedEvent` |

Notes and gotchas:

- **`RegisteredEvent` has no `domain` field.** Only `VerifiedEvent` and
  `RemovedEvent` carry `EventDomain` (Issue #226). The schema reflects this —
  `RegisteredEvent.domain` does not exist. Take `contractId` / `networkId` for a
  registration from the deployment the subgraph is pointed at.
- **`EventDomain`** = `{ contract_id: Address, network_id: BytesN<32>,
  contract_version: (u32,u32,u32), domain_version: u32 }`. Mapped as an embedded
  type, not a queryable entity.
- **`reason_code`** belongs to `VerificationRevokedEvent` and `PausedEvent`, not
  to any of the three events modelled here. It is intentionally absent.
- **Batch removes**: `batch_remove` emits one `RemovedEvent` per removed
  contributor, all in one transaction. The entity `id` includes the per-tx
  `eventIndex` so they do not collide.
- **Entity `id`** is the stable event id from
  [`DASHBOARD_SYNC.md`](../DASHBOARD_SYNC.md#stable-event-id-issue-283):
  `{networkId}:{contractId}:{ledgerSequence}:{txHash}:{eventIndex}`. Using it as
  the primary key makes ingestion replay-idempotent for free.
- **`Contributor` aggregate** is last-write-wins keyed on `ledgerSequence`:
  a `RegisteredEvent` sets `stellarAddress` and clears `verified`; a
  `VerifiedEvent` sets `verified = true`; a `RemovedEvent` sets `removed = true`
  and `stellarAddress = null`. A later `RegisteredEvent` clears `removed`.
- Treat the subgraph as a change-notification cache. After any gap, reconcile
  against `get_public_paginated` on-chain — see `DASHBOARD_SYNC.md`.

## Mapping handler sketch

```ts
export function handleRegistered(ev: RegisteredEvent): void {
  let e = new RegisteredEventEntity(eventId(ev)); // networkId:contractId:ledger:tx:index
  e.githubUsername = ev.params.github_username;
  e.stellarAddress = ev.params.stellar_address;
  e.timestamp = ev.params.timestamp;
  e.sponsor = ev.params.sponsor; // may be null
  e.ledgerSequence = ev.ledger;
  e.txHash = ev.transaction.hash;
  e.contributor = ev.params.github_username;
  e.save();
  touchContributor(ev.params.github_username, ev.ledger, ev.params.timestamp, {
    stellarAddress: ev.params.stellar_address, verified: false, removed: false,
  });
}
```

`handleVerified` / `handleRemoved` are the same shape, reading `domain.*` from
the payload and updating the `Contributor` `verified` / `removed` flags.

## Running locally

No hosted service is required to develop against this schema:

```bash
# 1. schema check — the schema is plain GraphQL SDL
npx graphql-schema-linter docs/subgraph/schema.graphql
#    or, with the Graph tooling:
npx --yes @graphprotocol/graph-cli@latest codegen --skip-migrations \
  --output-dir /tmp/tb-subgraph docs/subgraph/schema.graphql

# 2. produce a local event stream to map against
CONTRACT_ID=C... ONESHOT=1 ./scripts/event_indexer.sh
#    -> ./.indexer/events-<network>.jsonl  (one raw event per line)

# 3. a full local Graph Node stack (Postgres + IPFS + graph-node) via
#    docker-compose is the standard path once a manifest exists; that manifest
#    is deployment-specific (contract address, start block) and is not checked
#    in here.
```

## Example query

```graphql
{
  # every currently-verified, not-removed contributor
  contributors(where: { verified: true, removed: false }, orderBy: lastEventAt, orderDirection: desc) {
    githubUsername
    stellarAddress
    firstRegisteredAt
    lastEventAt
  }

  # full history for one username
  registeredEvents(where: { githubUsername: "octocat" }, orderBy: ledgerSequence) {
    id
    stellarAddress
    sponsor
    timestamp
  }
  verifiedEvents(where: { githubUsername: "octocat" }, orderBy: ledgerSequence) {
    id
    timestamp
    domain { contractId networkId contractVersion }
  }
  removedEvents(where: { githubUsername: "octocat" }, orderBy: ledgerSequence) {
    id
    timestamp
  }
}
```
