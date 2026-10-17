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
