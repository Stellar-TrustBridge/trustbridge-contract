# Dashboard Sync Notes

The dashboard can combine contract state with Horizon checks to show payout readiness.

## Recommended sync order

1. Read registered contributors from the contract.
2. Normalize GitHub usernames for display and filtering.
3. Query Horizon for funding, trustline, and reserve status.
4. Cache readiness results with a short TTL.
5. Re-check before payout export.

Contract verification proves the registry entry was approved; Horizon readiness proves the address can receive the selected asset.
