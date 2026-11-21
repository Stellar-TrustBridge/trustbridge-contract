# XDR Fixtures for Differential Testing

These XDR fixtures are generated from Rust contract tests and used to verify that
the TypeScript SDK decodes contract return values and event data correctly. A
decoding failure signals ABI drift — a field was reordered, a type changed, or a
new required field was added without updating the bindings.

## Generating Fixtures

```bash
# From the repo root — builds and runs the generator (prints marked blocks):
make xdr-fixtures

# Manually (output goes to stdout; paste each block into the matching file):
cargo test generate_xdr_fixtures -- --ignored --nocapture
```

The generator test is in `tests/generate_xdr_fixtures.rs`. It prints clearly
delimited blocks for each fixture; the Makefile target parses those blocks and
prints the fixture blocks for copying into the matching files.

## Fixture Files

Each fixture consists of up to three files:

| Extension | Contents |
|-----------|----------|
| `.xdr` | Base64-encoded XDR `ScVal` (event data or return value). |
| `.address` | Expected Stellar G-address string (for `ContributorRecord` fixtures). |
| `.meta.json` | Expected field values as JSON (for event fixtures). |

Files that start with `#` are placeholders. The TS test suite skips them with a
`t.skip()` rather than failing, so CI does not break before the generator has been
run.

### `get_address_octocat`

| File | Description |
|------|-------------|
| `get_address_octocat.xdr` | `ContributorRecord` returned by `get_address("octocat")`. |
| `get_address_octocat.address` | Expected Stellar address of the registered user. |

**What the test asserts:**
- `stellar_address` field decodes to the expected G-address.
- `registered_at` field is encoded as `u32` (not `u64` — ABI regression guard for Issue #67).
- `verified` field is encoded as `bool`.

### `batch_remove_proposed_event`

| File | Description |
|------|-------------|
| `batch_remove_proposed_event.xdr` | Data `ScVal` of the `BatchRemoveProposedEvent`. |
| `batch_remove_proposed_event.meta.json` | `{ "event_kind": "...", "proposed_by": "G...", "count": 4 }` |

**What the test asserts:**
- `proposed_by` is an `ScAddress`.
- `count` is a `u32 > 0`.
- `timestamp` is a `u64`.
- `domain` struct is present (EventDomain — Issue #226).
- Field values match the `.meta.json` sidecar when available.

### `batch_remove_executed_event`

| File | Description |
|------|-------------|
| `batch_remove_executed_event.xdr` | Data `ScVal` of the `BatchRemoveExecutedEvent`. |
| `batch_remove_executed_event.meta.json` | `{ "executed_by": "G...", "proposed_by": "G...", "count": 4, "successful": 4 }` |

**What the test asserts:**
- `executed_by` and `proposed_by` are both `ScAddress`.
- `executed_by != proposed_by` (dual-control invariant — Issue #219).
- `successful <= count`.
- `domain` struct is present.
- Field values match the `.meta.json` sidecar when available.

### `batch_remove_cancelled_event`

| File | Description |
|------|-------------|
| `batch_remove_cancelled_event.xdr` | Data `ScVal` of the `BatchRemoveCancelledEvent`. |
| `batch_remove_cancelled_event.meta.json` | `{ "cancelled_by": "G...", "proposed_by": "G..." }` |

**What the test asserts:**
- `cancelled_by` and `proposed_by` are both `ScAddress`.
- `timestamp` is a `u64`.
- `domain` struct is present.
- Field values match the `.meta.json` sidecar when available.

## Adding a New Fixture

1. Add the generation logic to `tests/generate_xdr_fixtures.rs` — use `print_fixture()` or follow the `get_address_octocat` pattern.
2. Add a placeholder `.xdr` and `.meta.json` (or `.address`) file here.
3. Add a test case to `ts-differential-tests/fixtures.test.js`.
4. Run `make xdr-fixtures` and copy each emitted block into the matching file.
5. Commit the generated fixtures together with the Rust and TS changes.

### Upgrade, RBAC and multisig event payloads (Issue #599)

The ignored Rust generator now emits these XDR/metadata pairs:

- `upgraded_event`, `role_granted_event`, `role_revoked_event`
- `upgrade_proposed_event`, `upgrade_approved_event`
- `upgrade_proposal_executed_event`, `upgrade_proposal_cancelled_event`

Each metadata sidecar records `fixture_version: 1`, an `event_kind`, a
`ledger_hint` (a sample ledger sequence, **not** part of the event payload),
and a sample event timestamp. The generator publishes deterministic synthetic
events inside a test environment and serializes their actual Soroban event
data, rather than attempting a real contract WASM upgrade.

Run `make xdr-fixtures` from the repository root, inspect the generated
`.xdr` and `.meta.json` pairs and commit them together. Then run
`make diff-test` to check TypeScript decoding. If the event field layout
changes incompatibly, bump `fixture_version` and regenerate the pairs.

**Do not use placeholder XDR as a passing differential test:** regeneration
requires the Rust toolchain and Soroban test runtime.
