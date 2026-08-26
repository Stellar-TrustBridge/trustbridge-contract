# Deployment Guide

Step-by-step instructions for deploying **trustbridge-contract** to Stellar Testnet and Mainnet.

Related docs: [README](../README.md) · [ARCHITECTURE](ARCHITECTURE.md) · [ABI](ABI.md) · [CONTRACT_HEALTH](CONTRACT_HEALTH.md) · [FUTURENET_ONBOARDING](FUTURENET_ONBOARDING.md)

---

## Prerequisites

1. **Rust** ≥ 1.84 with `wasm32v1-none` target
2. **Stellar CLI** ≥ 26.x (recommended)
3. A funded Stellar account on the target network

```bash
rustup target add wasm32v1-none
curl -fsSL https://github.com/stellar/stellar-cli/raw/main/install.sh | sh
```

---

## Environment Variables

Copy [`.env.example`](../.env.example) to `.env` and configure:

| Variable | Required | Description |
|----------|----------|-------------|
| `NETWORK` | No | `testnet` (default), `mainnet`, or `futurenet` |
| `ADMIN` | **Yes** | G-address of contract admin |
| `SOURCE` | No | Stellar CLI identity name (default: `default`) |
| `ALIAS` | No | CLI contract alias (default: `trustbridge`) |
| `INIT` | No | Auto-initialize after deploy (default: `true`) |

---

## Testnet Deployment

### 1. Create a deployer identity

```bash
stellar keys generate deployer --network testnet --fund
stellar keys use deployer
export ADMIN=$(stellar keys address deployer)
```

The Friendbot funds testnet accounts automatically via `--fund`.

### 2. Build the contract

```bash
make build
# Output: target/wasm32v1-none/release/trustbridge-contract.wasm
```

### 3. Deploy and initialize

```bash
make deploy-testnet
# or:
NETWORK=testnet ADMIN=$ADMIN SOURCE=deployer ./scripts/deploy.sh
```

The script:

1. Builds WASM if missing
2. Runs `stellar contract deploy`
3. Calls `initialize(admin)`
4. Writes `deployments/testnet.json`

### 4. Verify deployment

```bash
export CONTRACT_ID=$(jq -r .contract_id deployments/testnet.json)

stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account deployer \
  --network testnet \
  -- get_stats
# Expected: { "total": 0, "verified": 0 }
```

---

## Testnet Checklist

A repeatable checklist to validate every release build against testnet. See [TESTNET_CHECKLIST.md](TESTNET_CHECKLIST.md) for the full numbered steps.

Required environment variables:

| Variable | Required | Description |
|----------|----------|-------------|
| `NETWORK` | Yes | Must be `testnet` |
| `ADMIN` | Yes | G-address of the contract admin |
| `SOURCE` | Yes | Funded testnet CLI identity |
| `CONTRACT_ID` | After deploy | Recorded from `deployments/testnet.json` |

The checklist covers deploy → initialize → register → verify → export → remove, ending with cleanup. Defaults are safe: `NETWORK` defaults to `testnet`, so a mainnet run requires explicit configuration.

---

## Mainnet Deployment

### Dual-confirm checklist

Before every mainnet deploy, complete both confirmation steps:

1. **Build hash pin** — confirm the WASM hash matches the pinned value in `wasm-hash.pin`:
   ```bash
   make wasm-hash-pin
   # or manually:
   sha256sum target/wasm32v1-none/release/trustbridge-contract.wasm
   ```
   CI enforces this check automatically via the _Compute and verify WASM hash pin_ step.
   If the hash has intentionally changed (new release), run `make wasm-hash-update`, commit
   the updated `wasm-hash.pin`, and include before/after hashes in the PR description.
   Record the final hash in your deploy runbook and verify it against the CI build artifact.

2. **Human confirmation** — set `CONFIRM_MAINNET=yes` to proceed:
   ```bash
   export CONFIRM_MAINNET=yes
   make deploy-mainnet
   ```
   The `deploy-mainnet` Makefile target refuses to run unless `CONFIRM_MAINNET` is set to `yes`. This prevents accidental mainnet invocations from a default `make` run.

### Post-deploy verification

After deployment, verify the contract is initialized and operational:

```bash
export CONTRACT_ID=$(jq -r .contract_id deployments/mainnet.json)

# Confirm the contract is initialized
stellar contract invoke \
  --id $CONTRACT_ID \
  --source_account deployer \
  --network mainnet \
  -- get_stats
# Expected: { "total": 0, "verified": 0 }

# Confirm the deployed WASM hash matches the pinned build hash
stellar contract get_wasm_hash \
  --id $CONTRACT_ID \
  --network mainnet
```

Checklist before mainnet:

- [ ] Admin address reviewed (prefer multisig)
- [ ] WASM built from a tagged release commit
- [ ] Build hash pinned and recorded
- [ ] `CONFIRM_MAINNET=yes` explicitly set
- [ ] `cargo test` and CI green on that commit
- [ ] Contract ID recorded in `deployments/mainnet.json`
- [ ] TTL extension plan documented for persistent entries

## Upgrade Window Read-Only Mode

When rotating the WASM hash, put the contract into pause mode first so the
upgrade window behaves as read-only for integrators:

1. Call `set_paused(true)` as admin.
2. Publish or apply the new WASM upgrade.
3. Verify the new binary with the existing upgrade checks in [ABI.md](ABI.md)
  and the deployment script flow in [scripts/deploy.sh](../scripts/deploy.sh).
4. Call `set_paused(false)` once the upgrade is confirmed healthy.

During this window, lookups remain safe, but mutation entry points reject with
the existing pause error. In practice that means dashboards and indexers can
keep using `get_address`, `get_stats`, and the export/pagination reads, while
`register`, `remove`, `verify`, `pause`, `unpause`, `set_role`, `remove_role`,
`set_cooldown`, `attest_upgrade`, `clear_attestation`, and `upgrade` are
expected to fail fast until the contract is unpaused.

This mode is an operator procedure, not a new ABI surface, so it does not
change the public contract interface.

---

## Bulk Verify CLI

During Wave onboarding spikes, manual one-off `verify` calls do not scale. Use
`scripts/bulk_verify.sh` (or the Make targets) to verify a list of usernames from a file.

### Auth requirements

`SOURCE` must be the admin or hold `Role::Verifier` on the contract. Verify off-chain
GitHub identity for each username _before_ running the bulk verify.

### Pacing

The default pace is 500 ms between invocations. For large batches (>50 usernames) or when
hitting HTTP 429 responses, increase `--pace-ms` to 1000–2000 ms.

### Usage

```bash
# Create a file with one username per line
echo -e "octocat\nsome-contributor" > usernames.txt

# Dry-run (no transactions, confirm the list)
make bulk-verify-dry-run CONTRACT_ID=C... SOURCE=admin-identity NETWORK=testnet

# Execute with audit log
make bulk-verify CONTRACT_ID=C... SOURCE=admin-identity NETWORK=testnet \
    BULK_VERIFY_FILE=usernames.txt BULK_VERIFY_LOG=verify-audit.log

# Increase pacing for large batches
make bulk-verify CONTRACT_ID=C... SOURCE=admin-identity NETWORK=testnet \
    BULK_VERIFY_PACE=1000
```

Or call the script directly:

```bash
bash scripts/bulk_verify.sh \
    --file usernames.txt \
    --contract $CONTRACT_ID \
    --source admin-identity \
    --network testnet \
    --dry-run

bash scripts/bulk_verify.sh \
    --file usernames.txt \
    --contract $CONTRACT_ID \
    --source admin-identity \
    --network testnet \
    --continue-on-error \
    --pace-ms 500 \
    --audit-log verify-audit.log
```

Audit log lines are emitted to stdout and written to `--audit-log` file:

```json
{"timestamp":"2026-01-01T00:00:00Z","username":"octocat","network":"testnet","result":"ok","detail":"verified"}
```

---

## Using the Makefile

| Target | Description |
|--------|-------------|
| `make deploy-testnet` | Build + deploy to testnet |
| `make deploy-mainnet` | Build + deploy to mainnet (requires `CONFIRM_MAINNET=yes`) |
| `make invoke-init` | Initialize an existing contract |
| `make invoke-register` | Register a username |
| `make invoke-lookup` | Read-only lookup |
| `make invoke-stats` | Read statistics |
| `make invoke-verify` | Verify a contributor (admin or verifier role) |
| `make invoke-revoke-verification` | Revoke verification (admin or verifier role) |
| `make bulk-verify-dry-run` | Dry-run bulk verify from `BULK_VERIFY_FILE` |
| `make bulk-verify` | Bulk verify with pacing and audit log |
| `make bulk-revoke-dry-run` | Dry-run bulk revoke from `BULK_REVOKE_FILE` |
| `make bulk-revoke` | Bulk revoke with audit log |
| `make testnet-checklist` | Run the testnet smoke checklist |
| `make demo-e2e` | Run the cross-repo E2E demo (register → verify → lookup → export) |

Example registration:

```bash
export CONTRACT_ID=C...
make invoke-register GITHUB_USER=octocat STELLAR_ADDR=G... SOURCE=deployer
```

Equivalent raw CLI invocation:

```bash
stellar contract invoke --id $CONTRACT_ID \
  --source-account deployer \
  --network testnet \
  --send=yes \
  -- register \
  --github-username octocat \
  --stellar-address G...
```

`register` requires the source account to authenticate as `stellar-address`. If the
username is already registered to a different address, that previous address must
also sign. See [ABI.md](ABI.md#register) for full auth requirements and failure modes.

---

## deploy.sh Reference

```bash
NETWORK=testnet \
ADMIN=GABC... \
SOURCE=deployer \
ALIAS=trustbridge \
INIT=true \
./scripts/deploy.sh
```

| Flag | Default | Description |
|------|---------|-------------|
| `NETWORK` | `testnet` | Target network |
| `ADMIN` | — | Required admin G-address |
| `SOURCE` | `default` | Signing identity |
| `ALIAS` | `trustbridge` | CLI alias for contract ID |
| `INIT` | `true` | Call `initialize` after deploy |

---

## Registry Export & Import

Two operator scripts cover backups, dashboard migrations, and audit snapshots
of the registry, without giving up the on-chain data as the source of truth.

### Export (Issue #132)

`scripts/export_registry.sh` pages through the admin-only
`get_registered_paginated` and writes a single JSON file with a stable schema.

| Variable | Required | Description |
|----------|----------|--------------|
| `CONTRACT_ID` | **Yes** | Deployed contract ID |
| `SOURCE` | **Yes** | Stellar CLI identity of the contract admin — `get_registered_paginated` is admin-gated |
| `NETWORK` | No | `testnet` (default), `mainnet`, or `futurenet` |
| `OUTPUT_FILE` | No | Output path (default: `registry-export-<network>.json`) |
| `PAGE_LIMIT` | No | Records per page (default: `100`, the contract's `MAX_PAGE_LIMIT`) |

```bash
CONTRACT_ID=$CONTRACT_ID SOURCE=admin NETWORK=testnet ./scripts/export_registry.sh
# or:
make export-registry CONTRACT_ID=$CONTRACT_ID SOURCE=admin
```

The script fails with a clear error and a non-zero exit code if `CONTRACT_ID`,
`SOURCE`, the Stellar CLI, or `jq` are missing.

**Output schema** (stable field names for dashboard/indexer consumers):

```json
{
  "schema_version": 1,
  "contract_id": "C...",
  "network": "testnet",
  "exported_at": "2026-01-01T00:00:00Z",
  "count": 2,
  "records": [
    {
      "github_username": "octocat",
      "stellar_address": "G...",
      "verified": true,
      "registered_at": 1732800000
    }
  ]
}
```

### Import / validate (Issue #133)

Import does not bypass on-chain auth. `scripts/validate_registry.sh` never
writes to the contract — it validates an export file against live state,
which covers staging restores and migration dry-runs.

| Variable | Required | Description |
|----------|----------|--------------|
| `CONTRACT_ID` | **Yes** | Deployed contract ID |
| `NETWORK` | No | `testnet` (default), `mainnet`, or `futurenet` |
| `SOURCE` | No | Identity for per-record reads (default: `default`); `get_address` needs no auth, so any funded identity works |
| `ADMIN_SOURCE` | No | Admin identity; when set, also detects on-chain registrations **missing from** the export via admin-gated `get_registered_paginated` |
| `PAGE_LIMIT` | No | Records per page for the admin-side check (default: `100`) |

```bash
CONTRACT_ID=$CONTRACT_ID NETWORK=testnet ./scripts/validate_registry.sh registry-export-testnet.json
# or, for the full two-way diff:
make validate-registry CONTRACT_ID=$CONTRACT_ID ADMIN_SOURCE=admin EXPORT_FILE=registry-export-testnet.json
```

The script reports every mismatch it finds and exits `1` if any are present,
`0` if the export matches live state exactly, `2` on a usage or config error
(missing file, malformed JSON, missing `CONTRACT_ID`, etc.):

| Diff type | Meaning |
|-----------|---------|
| `MISSING_ONCHAIN` | Export has the username; the contract does not |
| `ADDRESS_MISMATCH` | `stellar_address` differs between export and chain |
| `VERIFIED_MISMATCH` | `verified` flag differs between export and chain |
| `MISSING_FROM_EXPORT` | Contract has the username; the export file does not (`ADMIN_SOURCE` only) |

**Safety warning:** this tool is validate-only by design. It does not replay
writes. Never use an export file to blindly overwrite mainnet state —
`register`/`verify`/`revoke_verification` all require the appropriate signer
to authorize each call individually, so a "replay" is a reviewed, one-by-one
series of ordinary invocations (e.g. `make invoke-register`), not a bulk
import. Treat `scripts/validate_registry.sh` output as the checklist for that
review, run it against testnet first, and never against mainnet without a
human reading every reported diff.

---

## Troubleshooting

### `wasm32v1-none` target not installed

```bash
rustup target add wasm32v1-none
```

### `wasm32-unknown-unknown` build fails on Rust 1.82+

`soroban-sdk` 26.x requires `wasm32v1-none`. Use `make build` (Stellar CLI) instead of legacy cargo target.

### `Unauthorized function call for address`

The `--source-account` must match the address that signed the auth payload. For `register`, source must own `stellar_address`. For `remove`, source must match `caller`.

### Insufficient fee / account not found

Ensure the source account is funded on the target network:

```bash
stellar keys fund deployer --network testnet
```

### Contract not initialized

Run initialize manually:

```bash
make invoke-init CONTRACT_ID=$CONTRACT_ID ADMIN=$ADMIN
```

---

## Simulate-Register Gas Reporting

Operators can estimate `register` resource costs **before** committing funds, using the
`stellar contract invoke` simulation path (no `--send=yes`).  This is the recommended way
to set Wave invoke budgets before contributors hit the contract at scale.

> **Works without spending funds.**  Simulation runs locally against the current ledger state.
> No transaction is submitted and no fees are charged.

### Makefile targets

| Target | Description |
|--------|-------------|
| `make simulate-register` | Baseline: short username (`octocat` by default) |
| `make simulate-register-max` | Max-length: 39-character username |
| `make simulate-register-compare` | Both runs back-to-back, output to `simulate-register-results.txt` |

**Prerequisites:**  `CONTRACT_ID` and `STELLAR_ADDR` must be set.  The `SOURCE` account just
needs to exist on the network — it is not charged.

```bash
export CONTRACT_ID=C...
export STELLAR_ADDR=G...   # the address that *would* be registered

# Baseline simulation
make simulate-register CONTRACT_ID=$CONTRACT_ID STELLAR_ADDR=$STELLAR_ADDR

# Max-length username
make simulate-register-max CONTRACT_ID=$CONTRACT_ID STELLAR_ADDR=$STELLAR_ADDR

# Compare both and write to file
make simulate-register-compare CONTRACT_ID=$CONTRACT_ID STELLAR_ADDR=$STELLAR_ADDR
```

Or call the CLI directly:

```bash
# Simulate register — no --send, no fees spent
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account deployer \
  --network testnet \
  -- register \
  --github-username octocat \
  --stellar-address $STELLAR_ADDR
```

### Output fields

The CLI prints a JSON-like block with at least these resource fields:

| Field | Description |
|-------|-------------|
| `cpu_instructions` | Metered Wasm CPU cost for this invocation |
| `mem_bytes` | Metered memory footprint in bytes |
| `min_resource_fee` | Minimum fee in stroops (1 XLM = 10 000 000 stroops) |
| `read_bytes` | Bytes read from ledger entries |
| `write_bytes` | Bytes written to ledger entries |

**Sample output interpretation (approximate, testnet only):**

```
Simulation result:
  cpu_instructions: 1_234_567
  mem_bytes:        45_678
  min_resource_fee: 9_876  stroops  (~0.001 XLM)
```

A `min_resource_fee` of ~10 000 stroops means each `register` call costs roughly 0.001 XLM
at the simulated ledger state.  Multiply by the expected number of Wave registrations to budget
the total fee pool.

### Baseline vs. max-length comparison

Issue #111 calls for comparing baseline (short username) against the maximum-length (39-char)
username to measure the username-length delta on `cpu_instructions` and `min_resource_fee`.

Run the comparison and diff the results:

```bash
make simulate-register-compare CONTRACT_ID=$CONTRACT_ID STELLAR_ADDR=$STELLAR_ADDR
# Output written to simulate-register-results.txt
diff <(git show HEAD:simulate-register-results.txt) simulate-register-results.txt
```

### Limitations

| Limitation | Impact |
|------------|--------|
| Simulation ≠ live execution | Fee shown is valid at simulation time; live fees can differ under load |
| `min_resource_fee` is a floor | Actual fee may be higher if ledger load is elevated |
| Auth simulation | The `--source-account` is simulated but not the registrant; fees are correct but the call would fail on a live network if `stellar_address` doesn't match `source-account` |
| No ledger commit | `count` and index updates are computed but not persisted; a re-simulation of a second `register` will show the same cost as the first |
| Rent fees change with upgrades | Re-simulate after protocol or fee schedule upgrades |

See [STORAGE_RENT.md](STORAGE_RENT.md) for how simulation fits into the broader rent estimation workflow.

---

## Post-Deployment

1. Publish the contract ID in the TrustBridge dashboard config
2. Configure the GitHub Action with `CONTRACT_ID` and `NETWORK`
3. Monitor events via a Stellar RPC endpoint or indexer
4. Schedule TTL extensions for persistent storage entries on long-lived networks
5. Wire production monitors to the probe sequence in
   [CONTRACT_HEALTH.md](CONTRACT_HEALTH.md) (initialized?, admin set?, stats
   sane?, optional Horizon lag)

See [SECURITY.md](SECURITY.md) for operational security guidance.

---

## WASM Hash Verification

The release WASM artifact is pinned by SHA-256 in `wasm-hash.pin`. CI fails the build when the
artifact hash does not match the pinned value, preventing silent deploy of a wrong or tampered WASM.

### How it works

1. CI builds the WASM via `stellar contract build`.
2. The _Compute and verify WASM hash pin_ CI step runs `sha256sum` on the artifact.
3. The computed hash is compared to the value in `wasm-hash.pin`.
4. A mismatch fails CI with clear instructions to update the pin.

### Local verification

```bash
# Verify the current build matches the pin:
make wasm-hash-pin

# After an intentional WASM change, update the pin:
make wasm-hash-update
git add wasm-hash.pin
git commit -m "chore: bump wasm-hash.pin after <feature>"
```

### Updating the pin (intentional WASM changes)

1. Make your contract changes and build: `make build`.
2. Run `make wasm-hash-update` to rewrite `wasm-hash.pin` with the new hash.
3. Commit `wasm-hash.pin` alongside the contract change.
4. Include before/after hashes in the PR description.
5. CI will verify the committed pin matches the rebuilt artifact.

### Mainnet pre-deploy checklist addition

- [ ] `make wasm-hash-pin` passes locally on the release commit.
- [ ] `wasm-hash.pin` committed hash matches the CI artifact hash shown in CI logs.
- [ ] Hash recorded in deploy runbook before running `make deploy-mainnet`.

---

## WASM Size Budget

Soroban charges upload fees proportional to WASM size, and the protocol imposes
an upper limit. Keeping the binary small reduces deploy cost and makes upgrades
cheaper.

### Current budget

| Metric | Value |
|--------|-------|
| Hard limit (CI gate) | **200 KB** (204 800 bytes) |
| Typical release size | ~85 KB |
| Headroom | ~115 KB |

The hard limit is enforced by:

- **CI**: the _WASM size regression gate_ step in `.github/workflows/ci.yml` fails
  the build when `trustbridge-contract.wasm` exceeds `WASM_SIZE_LIMIT`.
- **Local**: `make wasm-size` runs the same check after `make build`.

### How to measure locally

```bash
make wasm-size
```

Output:

```
──────────────────────────────────────────
  WASM size report
──────────────────────────────────────────
  File   : target/wasm32v1-none/release/trustbridge-contract.wasm
  Size   : 87040 bytes (~85 KB)
  Limit  : 204800 bytes (200 KB)
──────────────────────────────────────────
  Headroom: 117760 bytes remaining

PASS: WASM size is within budget.
```

### Rationale for the 200 KB limit

The optimised release WASM currently sits near 85 KB. 200 KB provides ~115 KB
of headroom for intentional feature additions while still catching accidental
bloat — e.g. a new dependency that pulls in an unintended transitive crate, or
a build profile misconfiguration that disables LTO.

### How to raise the limit

If intentional feature growth pushes the binary past 200 KB:

1. Run `make wasm-size` locally to measure the new size.
2. Round up to the nearest 10 KB and add ~20 KB of headroom to get the new
   ceiling.
3. Update `WASM_SIZE_LIMIT` in **both** places (they must stay in sync):
   - `Makefile` — the `WASM_SIZE_LIMIT ?=` variable
   - `.github/workflows/ci.yml` — the `WASM_SIZE_LIMIT:` env variable
4. Document the new limit and the feature that required the bump in this table.
5. Include before/after sizes in the PR description.
