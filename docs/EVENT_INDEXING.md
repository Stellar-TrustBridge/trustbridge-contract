# Event Indexing Notes

TrustBridge emits events so dashboards and indexers can build a readable contributor timeline.

## Suggested consumer behavior

- Treat contract storage as the source of truth.
- Treat events as an append-only activity stream.
- Reconcile from storage after missed ledger ranges or indexer downtime.

## Useful event fields

Indexers should capture the GitHub username, Stellar address, verification flag changes, ledger sequence, and transaction hash whenever available from the host environment.
