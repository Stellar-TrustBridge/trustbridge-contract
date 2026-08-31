# XDR Fixtures for Differential Testing

These XDR fixtures are generated from Rust contract tests and used to verify TypeScript bindings decode correctly.

## Generating Fixtures

Run the Rust test to generate fixtures:

```bash
cargo test generate_xdr_fixtures -- --ignored --exact --nocapture
```

Copy the output XDR and address values into the corresponding `.xdr` and `.address` files.

## Fixture Files

- `get_address_octocat.xdr` - XDR-encoded ContributorRecord for username "octocat"
- `get_address_octocat.address` - Expected Stellar address string (G...)

## Purpose

These fixtures ensure that TypeScript SDK XDR decoding matches the contract's actual output format. If the contract ABI changes (e.g., field reordering, type changes), the TypeScript decode will fail, catching drift between bindings and contract.
