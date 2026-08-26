# Event Indexing Notes

TrustBridge emits events so dashboards and indexers can build a readable contributor timeline.

## Suggested consumer behavior

- Treat contract storage as the source of truth.
- Treat events as an append-only activity stream.
- Reconcile from storage after missed ledger ranges or indexer downtime.

## Useful event fields

Indexers should capture the GitHub username, Stellar address, verification flag changes, ledger sequence, and transaction hash whenever available from the host environment.

---

## Pause and freeze-window events

Three code paths can pause the contract. Indexers must subscribe to all three
event types to correctly detect freeze windows:

| Entry point | Event emitted | Condition |
|-------------|---------------|-----------|
| `pause` | `PausedEvent` | Always (admin-only) |
| `set_paused(true)` | `PausedEvent` | Only when state changes (idempotent) |
| `emergency_pause` | `EmergencyPausedEvent` | Only when state changes (admin or guardian, idempotent) |
| `unpause` | `UnpausedEvent` | Always (admin-only) |
| `set_paused(false)` | `UnpausedEvent` | Only when state changes (idempotent) |
| `clear_emergency_pause` | `EmergencyClearedEvent` | Only when state changes (admin-only) |

**Important:** `set_paused` previously emitted no event, making freeze windows
invisible to indexers when that path was used. As of Issue #197 it now emits
`PausedEvent` / `UnpausedEvent` on state transitions — identical to `pause` /
`unpause`. Idempotent calls (already-paused → pause again) produce no event and
leave indexer state unchanged.

### Emergency pause circuit breaker (Issue #196)

`EmergencyPausedEvent` carries `triggered_by` (admin or guardian address) and
`timestamp`. Indexers should treat this as equivalent to a normal `PausedEvent`
for freeze-window logic: all mutating operations are blocked while either the
normal pause **or** the emergency pause is active.

`EmergencyClearedEvent` is emitted exclusively by the admin when the emergency
pause is lifted. Only the admin can clear it — the guardian cannot. Use this
event to close the freeze window opened by `EmergencyPausedEvent`.

### Recommended indexer state machine

```
NORMAL ──── PausedEvent / EmergencyPausedEvent ──▶ FROZEN
FROZEN ──── UnpausedEvent ─────────────────────▶ NORMAL  (if no emergency pause)
FROZEN ──── EmergencyClearedEvent ─────────────▶ NORMAL  (if no normal pause)
```

Because both flags are independent, an indexer should track them separately and
consider the contract frozen when **either** is set. Query `is_paused()` and
`is_emergency_paused()` after any indexer gap to reconcile state directly from
the contract rather than relying solely on the event stream.
