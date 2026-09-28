# TrustBridge Registry Subgraph (Issues #284, #386)

The dashboard currently polls RPC directly. This directory is a **schema +
mapping spec** so Wave UIs can query contributor history from a subgraph instead
of writing an indexer from scratch. Operating a hosted indexer is out of scope
for this repo — [`scripts/event_indexer.sh`](../../scripts/event_indexer.sh)
remains the runnable local reference.

- [`schema.graphql`](schema.graphql) — entities for every contributor-directed
  contract event (`RegisteredEvent`, `BotStatusChangedEvent`, `VerifiedEvent`,
  `VerificationRevokedEvent`, `RemovedEvent`, `RenamedEvent`, the challenge
  lifecycle, and the address-rotation lifecycle), plus a derived `Contributor`
  aggregate.

## Schema Overview & Evolution

The [`schema.graphql`](schema.graphql) file defines the entities that model the contract's state. It includes 1-to-1 mappings for every contributor-directed event (e.g., `RegisteredEvent`, `VerifiedEvent`) and a mutable `Contributor` aggregate that tracks the current state of a registered user.

**Schema Evolution:**
- The contract uses an `EventDomain` payload (introduced in 1.1.0) on most events, which includes `contractVersion`.
- If the smart contract is upgraded and new fields are added to an event, or if new events are introduced, this `schema.graphql` and the corresponding mapping handlers must be updated to support them.
- The `Contributor` aggregate might also need to be evolved if new core properties are tracked on-chain.

## Sync Flow & Caveats

When onboarding a new indexer (like Graph Node or Envio) using this schema, be aware of the following sync assumptions:

- **Idempotency & Event IDs:** Entity IDs are stable, following the format `{networkId}:{contractId}:{ledgerSequence}:{txHash}:{eventIndex}` (see [`DASHBOARD_SYNC.md`](../DASHBOARD_SYNC.md#stable-event-id-issue-283)). Using this as the primary key ensures that ingestion is replay-idempotent.
- **Event Ordering:** The indexer must process events strictly in order of `ledgerSequence` and then `eventIndex` within the transaction to maintain the correct state in the `Contributor` aggregate.
- **Reconciliation:** Treat the subgraph as a change-notification cache. After any indexing gap or indexer restart, you should reconcile the indexer's state against the contract's authoritative storage using `get_public_paginated` (see [`DASHBOARD_SYNC.md`](../DASHBOARD_SYNC.md)).
- **Re-orgs:** While standard indexers handle ledger rollbacks, the last-write-wins approach on the `Contributor` entity helps ensure the state eventually matches the chain even if events are re-processed.

## Network & Address Configuration

To deploy a subgraph from this schema, your indexer configuration (e.g., `subgraph.yaml`) must specify the exact environment:

- **Contract Address (`contractId`):** The indexer must be scoped to the specific `C...` address of the deployed TrustBridge contract.
- **Network / Passphrase:** You must configure the target network (e.g., Testnet or Public). Note that the `networkId` (SHA-256 of the network passphrase) is also emitted inside the `EventDomain` of most events for cross-verification.
- **Start Ledger:** To avoid scanning the entire Stellar history from ledger 1, always set the start block/ledger to the sequence number where the contract was deployed.
- For local testing and raw event viewing without a full indexer stack, refer to [`scripts/event_indexer.sh`](../../scripts/event_indexer.sh).

## Event → entity mapping

Field names and types below are copied from the on-chain `#[contractevent]`
definitions in [`src/events.rs`](../../src/events.rs). Do not add payload fields
the contract does not emit.

| Contract event | Topic symbol | Payload fields (from `src/events.rs`) | Entity |
|---|---|---|---|
| `RegisteredEvent` | `registered_event` | `github_username` (topic), `stellar_address`, `timestamp`, `sponsor: Option<Address>`, `domain` | `RegisteredEvent` |
| `BotStatusChangedEvent` | `bot_status_changed_event` | `github_username` (topic), `is_bot`, `actor`, `timestamp`, `domain` | `BotStatusChangedEvent` |
| `VerifiedEvent` | `verified_event` | `github_username` (topic), `stellar_address`, `timestamp`, `domain` | `VerifiedEvent` |
| `VerificationRevokedEvent` | `verification_revoked_event` | `github_username` (topic), `stellar_address`, `timestamp`, `reason_code`, `domain` | `VerificationRevokedEvent` |
| `RemovedEvent` | `removed_event` | `github_username` (topic), `stellar_address`, `timestamp`, `domain` | `RemovedEvent` |
| `RenamedEvent` | `renamed_event` | `old_username` (topic), `new_username` (topic), `stellar_address`, `verification_cleared`, `timestamp` | `RenamedEvent` |
| `ChallengeStartedEvent` | `challenge_started_event` | `github_username` (topic), `challenged_by`, `resolve_after`, `timestamp`, `domain` | `ChallengeStartedEvent` |
| `ChallengeCancelledEvent` | `challenge_cancelled_event` | `github_username` (topic), `cancelled_by`, `timestamp`, `domain` | `ChallengeCancelledEvent` |
| `ChallengeCompletedEvent` | `challenge_completed_event` | `github_username` (topic), `completed_by`, `timestamp`, `domain` | `ChallengeCompletedEvent` |
| `RotationRequestedEvent` | `rotation_requested_event` | `github_username` (topic), `current_address`, `new_address`, `executable_at`, `timestamp` | `RotationRequestedEvent` |
| `RotationExecutedEvent` | `rotation_executed_event` | `github_username` (topic), `old_address`, `new_address`, `timestamp` | `RotationExecutedEvent` |
| `RotationCancelledEvent` | `rotation_cancelled_event` | `github_username` (topic), `cancelled_by`, `timestamp` | `RotationCancelledEvent` |

Notes and gotchas:

- **All three events carry `domain`.** `RegisteredEvent` gained `EventDomain`
  in 1.1.0 (Issue #373), matching `VerifiedEvent` and `RemovedEvent` (Issue
  #226). Registrations emitted by pre-1.1.0 WASM have no `domain`; fall back to
  the deployment the subgraph is pointed at for those.
- **`EventDomain`** = `{ contract_id: Address, network_id: BytesN<32>,
  contract_version: (u32,u32,u32), domain_version: u32 }`. Mapped as an embedded
  type, not a queryable entity.
- **`reason_code`** is a `u32` `RevokeReason` discriminant carried by
  `VerificationRevokedEvent` (and by the unmodelled `PausedEvent` /
  `UnpausedEvent`) — it lives on `VerificationRevokedEvent.reasonCode` here.
- **`RenamedEvent` has two topic usernames.** `old_username` and `new_username`
  are both topics; the entity keeps both as plain string fields. The `contributor`
  relation points at **`old_username`** (the contributor being renamed); the
  mapping must additionally migrate the `Contributor` aggregate from
  `old_username` to `new_username` (create the new id, apply
  `verification_cleared`, leave the old one with `removed = true`).
- **Rotation events carry no `domain`** but do change the contributor: a
  `RotationExecutedEvent` moves the aggregate's `stellarAddress` from
  `old_address` to `new_address`.
- **A completed challenge also emits a `RemovedEvent`** in the same transaction
  (the squatted registration is deleted), so a `ChallengeCompletedEvent` and a
  `RemovedEvent` for the same username arrive together.
- **Batch removes**: `batch_remove` emits one `RemovedEvent` per removed
  contributor, all in one transaction. The entity `id` includes the per-tx
  `eventIndex` so they do not collide. The operational proposal events around
  dual-control batches (`BatchRemoveProposedEvent`, `BatchRemoveExecutedEvent`,
  `BatchRemoveCancelledEvent`) are **not** modelled — see scope below.
- **Entity `id`** is the stable event id from
  [`DASHBOARD_SYNC.md`](../DASHBOARD_SYNC.md#stable-event-id-issue-283):
  `{networkId}:{contractId}:{ledgerSequence}:{txHash}:{eventIndex}`. Using it as
  the primary key makes ingestion replay-idempotent for free.
- **`Contributor` aggregate** is last-write-wins keyed on `ledgerSequence`:
  a `RegisteredEvent` sets `stellarAddress` and clears `verified`; a
  `VerifiedEvent` sets `verified = true`; a `VerificationRevokedEvent` sets
  `verified = false`; a `RemovedEvent` sets `removed = true` and
  `stellarAddress = null`; a later `RegisteredEvent` clears `removed`. A
  `RotationExecutedEvent` updates `stellarAddress`; a `RenamedEvent` migrates
  the aggregate to `new_username`. Challenge events change no aggregate fields —
  they are recorded for history only.
- **Scope: events without a contributor identity are not modelled.** Pause /
  unpause, role grant / revoke / cancel / pending, guardian changes, WASM
  upgrades / attestations / staging, and emergency pause events are telemetry
  about the contract instance itself and name no contributor, so they have no
  entity here and stay queryable only through the raw event log
  (`scripts/event_indexer.sh`). The dashboard reconstructs contributor history
  entirely from the modelled events.
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

`handleVerified` / `handleRemoved` / `handleVerificationRevoked` /
`handleChallenge*` are the same shape, reading `domain.*` from the payload and
updating the `Contributor` `verified` / `removed` flags.

`handleRenamed` is the one handler that touches two `Contributor` ids — link
the event to `old_username`, then migrate the aggregate:

```ts
export function handleRenamed(ev: RenamedEvent): void {
  let e = new RenamedEventEntity(eventId(ev));
  e.oldUsername = ev.params.old_username;
  e.newUsername = ev.params.new_username;
  e.stellarAddress = ev.params.stellar_address;
  e.verificationCleared = ev.params.verification_cleared;
  e.timestamp = ev.params.timestamp;
  e.ledgerSequence = ev.ledger;
  e.txHash = ev.transaction.hash;
  e.contributor = ev.params.old_username;
  e.save();
  migrateContributor(ev.params.old_username, ev.params.new_username,
    ev.params.verification_cleared); // set verified=false if cleared, else keep
}
```

`handleRotationExecuted` is the same shape minus the dual-username handling and
updates the aggregate's `stellarAddress` to `new_address`.

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
  verificationRevokedEvents(where: { githubUsername: "octocat" }, orderBy: ledgerSequence) {
    id
    timestamp
    reasonCode
  }
  removedEvents(where: { githubUsername: "octocat" }, orderBy: ledgerSequence) {
    id
    timestamp
  }
  renamedEvents(where: { oldUsername: "octocat" }, orderBy: ledgerSequence) {
    id
    newUsername
    verificationCleared
    timestamp
  }
}
```