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
| `RoleGrantPendingEvent` | `role_grant_pending_event` | `address` (topic), `role`, `admin`, `activate_at`, `timestamp` | `RoleGrantPendingEvent` |

> **Pending role grants (Issue #630):** `set_role` behind a `set_role_delay`
> timelock emits `RoleGrantPendingEvent` instead of `RoleGrantedEvent`. Handler:
> upsert a `RoleGrantPendingEvent` entity keyed by the stable event id above,
> keyed additionally on `(address)` for dashboard lookup of the queued grant;
> on `RoleGrantedEvent` (activation) or `RoleGrantCancelledEvent` /
> `remove_role`, mark the pending row superseded/cancelled so no orphan remains.
> `activate_at == 0` never occurs on-chain (queued grants always carry a future
> `activate_at`); treat `activate_at == 0` as the instant-grant path, i.e. expect
> `RoleGrantedEvent` directly. Only one pending grant per target exists by design
> (`set_pending_role` overwrites). Topic symbols: see `TOPIC_TABLE` in
> [`scripts/event_indexer.sh`](../../scripts/event_indexer.sh) and
> [`scripts/check_event_topics.sh`](../../scripts/check_event_topics.sh).
> Timelock operator flow: [ADMIN_RUNBOOK timelocked role grants](../ADMIN_RUNBOOK.md#timelocked-role-grants-issue-220).
> Instant vs pending: `RoleGrantedEvent` fires on immediate `set_role` or on
> `activate_role`; `RoleGrantPendingEvent` fires only when the grant is queued.
> Dashboard state: cross-link [DASHBOARD_SYNC.md](../DASHBOARD_SYNC.md#abi-event-reference).

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

## Alert mapping: `UpgradeAttestedEvent` and index-generation bumps (Issue #787)

`UpgradeAttestedEvent` and an index-generation bump are both **unmodelled,
contract-instance-level signals** (no contributor identity), so neither gets a
subgraph entity under the scope rule above. They still matter for Wave
operators, so indexers should alert on them directly from the raw event log
rather than silently dropping them.

### `UpgradeAttestedEvent`

| Field | Type | Meaning |
|---|---|---|
| `wasm_hash` (topic) | `BytesN<32>` | The WASM hash the admin attested for the *next* `upgrade` call |
| `expires_at` | `u64` | Unix timestamp after which `upgrade` will reject this attestation (`AttestationExpired`) |
| `timestamp` | `u64` | When the attestation was recorded |

Alert mapping:

- **Treat every `UpgradeAttestedEvent` as a change-window signal.** Fire a
  Warn-level alert (not critical — attesting is a normal pre-upgrade step) so
  on-call knows a WASM swap is imminent and can cross-check it against the
  planned change log.
- **Unexpected attestation outside a declared change window is the actual
  incident.** If `UpgradeAttestedEvent` fires with no matching entry in the
  Wave change log, escalate to critical — see [`get_health_probes` →
  Unexpected upgrade window](../CONTRACT_HEALTH.md#choosing-a-probe--alert-slos-issue-756),
  which reports the same condition (`attestation_present: true`) as a polled
  snapshot.
- **Pair with `AttestationClearedEvent`.** If an attestation is withdrawn
  (`AttestationClearedEvent`, same shape) before the matching `UpgradedEvent`
  arrives, clear the Warn alert rather than letting it dangle.
- Example handler (`AssemblyScript`, mirrors the sketch below — logs only,
  no entity write):
  ```ts
  export function handleUpgradeAttested(ev: UpgradeAttestedEvent): void {
    log.warning(
      "upgrade attested: wasm_hash={}, expires_at={}",
      [ev.params.wasm_hash.toHexString(), ev.params.expires_at.toString()],
    );
    // Forward to the alerting pipeline (e.g. a webhook sink); do not persist
    // as an entity — this event names no contributor and is out of schema
    // scope per "Event → entity mapping" above.
  }
  ```

### Index-generation bumps

There is **no dedicated `IndexGenerationBumpedEvent`** emitted on-chain today
— `index_gen` is an internal counter (bumped by `repair_index` and by registry
structural changes) surfaced only through `get_health` /
`get_health_probes().index_gen`. Indexers cannot subscribe to this as a log
event; they must poll it.

Alert mapping:

- **Poll `get_health_probes().index_gen` on the same cadence as the health
  probe itself** (see [CONTRACT_HEALTH.md](../CONTRACT_HEALTH.md)) and diff
  against the last-seen value.
- **A bump invalidates any cached `get_public_paginated` / registry-index
  cursor the dashboard is holding**, exactly as [DASHBOARD_SYNC.md → Health
  probes for dashboards](../DASHBOARD_SYNC.md#health-probes-for-dashboards-issue-756)
  documents for cursor invalidation. Treat a bump as Warn-level ("re-sync
  cached registry views"), not critical — it is routine maintenance, not
  contract failure.
- **An `index_gen` bump with no corresponding `repair_index` invocation in the
  audit log** is the escalation case: it implies a structural registry change
  nothing recorded, and should page.
- Example poll-and-diff handler:
  ```ts
  // Run on the indexer's existing polling loop; `lastSeenIndexGen` is
  // persisted indexer-local state, not a subgraph entity.
  export function checkIndexGeneration(probes: HealthProbesSnapshot): void {
    if (probes.indexGen != lastSeenIndexGen) {
      log.warning(
        "index_gen bumped: {} -> {} — invalidating cached cursors",
        [lastSeenIndexGen.toString(), probes.indexGen.toString()],
      );
      invalidateCachedCursors();
      lastSeenIndexGen = probes.indexGen;
    }
  }
  ```

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
## Topic coverage across docs (Issue #754)

Every published topic appears in exactly one of the two tables in this file:
the *Event → entity mapping* table or the waiver table below. It also appears
in the [`EVENT_INDEXING.md` topic table](../EVENT_INDEXING.md#topic-classification),
whose **Subgraph** column must show the same outcome: the entity name, or
`waived`. `make subgraph-parity` enforces all of this in CI, including the
`event_kind` / `category` columns against `TOPIC_TABLE` in
`scripts/event_indexer.sh`. Operational topics (pause, upgrade, attestation,
batch, audit config) are waived here and are consumed from the raw indexer.

## Explicit subgraph event waivers (Issue #600)

The contributor-history subgraph intentionally excludes the operational events
listed below. These are explicit per-topic waivers, **not** mappings; raw
contract events remain available through `scripts/event_indexer.sh`. Every
`#[contractevent]` must have either an `@entity` type of the same name and a
mapping-table row above, or one waiver row below. The parity check rejects
missing, duplicate, stale or ambiguous entries.

| Waived topic | Reason |
|---|---|
| `upgraded_event` | Contract upgrade/attestation telemetry; available via raw event indexer |
| `paused_event` | Operational administration or RBAC telemetry; available via raw event indexer |
| `unpaused_event` | Operational event outside contributor-history schema; available via raw event indexer |
| `emergency_paused_event` | Operational administration or RBAC telemetry; available via raw event indexer |
| `emergency_cleared_event` | Operational event outside contributor-history schema; available via raw event indexer |
| `guardian_changed_event` | Operational administration or RBAC telemetry; available via raw event indexer |
| `role_grant_cancelled_event` | Operational administration or RBAC telemetry; available via raw event indexer |
| `wasm_staged_event` | Contract upgrade/attestation telemetry; available via raw event indexer |
| `upgrade_attested_event` | Contract upgrade/attestation telemetry; available via raw event indexer |
| `attestation_cleared_event` | Contract upgrade/attestation telemetry; available via raw event indexer |
| `role_granted_event` | Operational administration or RBAC telemetry; available via raw event indexer |
| `role_revoked_event` | Operational administration or RBAC telemetry; available via raw event indexer |
| `batch_remove_proposed_event` | Batch administration event; individual removed records use RemovedEvent entity |
| `batch_remove_executed_event` | Batch administration event; individual removed records use RemovedEvent entity |
| `batch_remove_cancelled_event` | Batch administration event; individual removed records use RemovedEvent entity |
| `staged_wasm_cleared_event` | Contract upgrade/attestation telemetry; available via raw event indexer |
| `upgrade_proposed_event` | Contract upgrade/attestation telemetry; available via raw event indexer |
| `upgrade_approved_event` | Contract upgrade/attestation telemetry; available via raw event indexer |
| `upgrade_proposal_executed_event` | Contract upgrade/attestation telemetry; available via raw event indexer |
| `upgrade_proposal_cancelled_event` | Contract upgrade/attestation telemetry; available via raw event indexer |
| `verification_configured_event` | Operational administration or RBAC telemetry; available via raw event indexer |
| `audit_config_changed_event` | Operational administration or RBAC telemetry; available via raw event indexer |

Run `make subgraph-parity` to verify the event inventory, mapping table,
Graph schema, and waivers locally. Changes to an event name (including topic
renames) require corresponding schema or waiver changes. `Address` and
`BytesN<32>` on-chain values map to Graph `Bytes` in the schema; this check
validates event/entity identity, not payload field type equivalence.
