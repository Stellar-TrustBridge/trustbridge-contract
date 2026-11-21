# Bindings golden fixtures (Issue #328)

Shared XDR goldens between this crate and `trustbridge-action`
(`src/soroban.ts`). The action should decode these instead of hand-mocking
`simulateTransaction` XDR, so a return-type change here breaks its tests too.

## `get_address_simulate.v1.json`

Produced and verified by `tests/bindings_golden.rs` (`make bindings-golden`).
CI fails if the contract's `get_address` encoding drifts from the checked-in
file, and uploads the file as the `bindings-golden-fixtures` workflow artifact.

| Field | Meaning |
|---|---|
| `fixture` / `version` | Fixture id and format version. `version` bumps (and the file name changes to `.vN.json`) on any breaking change to the format or to `get_address`'s return type. Additive fields keep the version. |
| `cases[].retval_xdr` | Base64 XDR of the `ScVal` returned by simulation — the value at `simulateTransaction` → `results[0].xdr` (or `result.retval` in the JS SDK's parsed response). |
| `cases[].decoded` | The expected decoded value; `null` for `None` (`ScVal::Void`). |

Cases: `registered` (`Some(ContributorRecord)`, a 5-field `ScMap` with keys
sorted `is_bot, payout_address, registered_at, stellar_address, verified`) and
`not_registered` (`None` → `scvVoid`). Addresses and the ledger timestamp are
fixed so the XDR is byte-stable.

## Using it from `trustbridge-action`

1. Pin a fixture version: vendor the file (or download the
   `bindings-golden-fixtures` artifact from a tagged contract CI run) into the
   action's test fixtures, keeping the `v1` name.
2. In the `soroban.ts` tests, feed `retval_xdr` through the same parse path
   used for live responses, e.g.
   `scValToNative(xdr.ScVal.fromXDR(retval_xdr, "base64"))`, and assert it
   equals `decoded` (convert `registered_at` to a number and addresses to
   G-strkeys as the action does).
3. Assert the `not_registered` case yields "no record", not an exception.
4. Check `version === 1`; when the contract ships `v2`, update the action in
   the same release window.

## Regenerating

```bash
UPDATE_GOLDEN=1 cargo test --test bindings_golden
```

Commit the updated JSON with the contract change, bump `FIXTURE_VERSION` in
the test for breaking changes, and note it in `CHANGELOG.md`.
