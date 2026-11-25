# TypeScript Differential Tests

The Node test runner decodes the checked-in `get_address` ScVal XDR with
`@stellar/stellar-sdk` and compares every field with the Rust-produced golden.
It tests both a registered record and a missing record. The canonical fixture
is [`tests/testdata/bindings/get_address_simulate.v1.json`](../tests/testdata/bindings/get_address_simulate.v1.json),
which `tests/bindings_golden.rs` checks byte for byte against contract output.

## Run locally

Requires Node.js 20+ and npm:

```bash
cd ts-differential-tests
npm ci
npm test
```

From the repository root, `make diff-test` runs the same tests. When the
contract's `get_address` return shape intentionally changes, regenerate the
shared golden with `UPDATE_GOLDEN=1 cargo test --test bindings_golden` and
review the resulting diff alongside the TypeScript decoder.

## CI entry point

The `diff-test` job in [`.github/workflows/ci.yml`](../.github/workflows/ci.yml)
runs `make diff-test` (`npm ci` + `npm test`, Node.js 20, npm cache keyed on
`package-lock.json`) on pull requests to `main` and pushes to `main`. The
`node --test` output names the failing test, so an XDR decoding or
golden-value mismatch shows which fixture diverged. Keep that job required in
the repository's branch protection rules to block merges until it passes.

The older `fixtures/get_address_octocat.*` files are placeholders and are not
used by the CI entry point. The shared Rust golden above is the source of
truth for these tests.

## Upgrade, RBAC, and multisig XDR fixture coverage (Issue #599)

Seven event fixtures under `fixtures/` are generated from deterministic
Rust contract-event payloads, including `upgraded_event`,
`role_granted_event`, `role_revoked_event`, and the four multisig upgrade
proposal lifecycle events. The event data XDR is base64 encoded in `.xdr`
files, with `fixture_version`, `event_kind`, `ledger_hint`, and sample
`timestamp` in the corresponding `.meta.json` sidecars. Fields annotated
`#[topic]` in `src/events.rs` are event topics, not data XDR fields.

To regenerate from the repository root:

```bash
make xdr-fixtures
make diff-test
```

The first command invokes the ignored Rust generator test and prints marked
fixture blocks. Copy the emitted XDR and JSON blocks into their matching files
under `ts-differential-tests/fixtures/`; `make xdr-fixtures` does **not**
write these files itself. The second command verifies the committed fixture
data against the TypeScript decoder. Regenerate and review all affected golden
pairs when a payload changes; bump `fixture_version` if the layout is
incompatible. No additional environment variables are required.
