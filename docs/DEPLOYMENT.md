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

### WASM size budget

The optimized release WASM must remain at or below **204,800 bytes (200 KiB)**.
This is the deployment budget enforced by CI and mirrored by the Makefile's
`WASM_SIZE_LIMIT` value. The check uses the same release artifact produced by
`stellar contract build`, preferring `wasm32v1-none` and falling back to
`wasm32-unknown-unknown`.

Inspect the current artifact locally with:

```bash
make wasm-size
```

CI runs this check immediately after the release build. A size increase beyond
the budget fails the job; intentional increases must update the documented
budget, `WASM_SIZE_LIMIT` in the Makefile, and the CI limit together.

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

### Optional CI live smoke

Maintainers can opt into the invoke-only live testnet smoke job by setting the
repository variable `TRUSTBRIDGE_LIVE_TESTNET=true`. Configure the existing
testnet contract and a funded source account using these values:

- Secret `TRUSTBRIDGE_TESTNET_CONTRACT_ID`: pre-deployed testnet contract ID.
- Secret `TRUSTBRIDGE_TESTNET_SECRET_KEY`: source account secret key, used only
  to create an ephemeral CI identity.
- Variable `TRUSTBRIDGE_TESTNET_USERNAME`: an existing username to query.

The job runs `get_stats` and `get_address` against `testnet`; it does not deploy
or initialize anything. It is restricted to non-pull-request events in the
canonical repository, which prevents fork PRs from accessing organization
secrets. Enabling the variable without all required values fails the job
immediately rather than silently skipping the smoke.

---

## Futurenet vs Testnet Parameters (Issue #671)

Futurenet and Testnet are separate networks: a contract ID, identity funding,
or deployment JSON from one is **not valid** on the other. Do not copy
parameters across. `make network-params NETWORK=<net>` prints the values below
and exits non-zero for an unknown `NETWORK`.

| Parameter | Testnet | Futurenet |
|-----------|---------|-----------|
| `NETWORK` | `testnet` (default) | `futurenet` |
| RPC URL | `https://soroban-testnet.stellar.org` | `https://rpc-futurenet.stellar.org` (`FUTURENET_RPC_URL`) |
| Network passphrase | `Test SDF Network ; September 2015` | `Test SDF Future Network ; October 2022` |
| Friendbot | `https://friendbot.stellar.org` (via `--fund`) | `https://friendbot-futurenet.stellar.org` (`FUTURENET_FRIENDBOT_URL`) |
| Deploy target | `make deploy-testnet` | `make futurenet-smoke` |
| Deployment record | `deployments/testnet.json` | `deployments/futurenet.json` |
| Contract / admin addresses | Per-network; re-deploy and re-record | Per-network; re-deploy and re-record |
| WASM pin (`wasm-hash.pin`) | Must match before release sign-off | Informational — Futurenet may run a newer protocol; rebuild with the pinned toolchain before comparing |
| Reset cadence | Periodic resets wipe contracts | More frequent resets; expect redeploys |

A `network passphrase` mismatch error almost always means `NETWORK` or the
CLI identity points at the other network — see
[TESTNET_CHECKLIST.md](TESTNET_CHECKLIST.md#testnet-vs-futurenet-failure-modes)
and [FUTURENET_ONBOARDING.md](FUTURENET_ONBOARDING.md).

---

## Mainnet Deployment

### Dual-confirm checklist

Before every mainnet deploy, complete both confirmation steps:

1. **Build hash pin** — confirm the WASM hash matches the pinned value in `wasm-hash.pin`:
   ```bash
   make wasm-hash-pin
   # or manually:
   sha256sum target/wasm32v1-none/release/trustbridge_contract.wasm
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

## Failed WASM Apply Rollback / Forward-Fix

If a WASM upgrade is applied but post-upgrade health checks fail, treat the
contract as potentially unsafe for writes until the operator confirms a
rollback or forward-fix path. Do **not** silently retry with a different hash.

### Immediate containment

1. **Pause writes**. Call `set_paused(true, reason_code)` (or `pause`) from the
   admin so mutating paths fail closed with `Paused` while reads remain
   available for triage.
2. **Capture provenance and attestation state** before changing anything else:
   ```bash
   stellar contract invoke --id $CONTRACT_ID --network $NETWORK -- get_provenance
   stellar contract invoke --id $CONTRACT_ID --network $NETWORK -- get_attestation
   stellar contract invoke --id $CONTRACT_ID --network $NETWORK -- get_health
   stellar contract invoke --id $CONTRACT_ID --network $NETWORK -- get_version
   ```
   Record the current WASM hash, expected previous hash, attestation state,
   ledger/time, and failing health symptoms.
3. **Confirm the failure mode**. Distinguish a bad WASM apply (new code active,
   health checks failing) from an operator/RPC failure (upgrade not actually
   applied). Use `get_provenance`, `get_version`, and contract health probes.

### Rollback

Soroban contract code can be replaced by another admin-authorized upgrade, but
state is preserved. A rollback is a forward upgrade to a known-good WASM hash.

1. Select the last known-good WASM hash from release records / `wasm-hash.pin`
   / prior provenance.
2. Rehearse the candidate hash on testnet/futurenet against a snapshot-shaped
   state where possible.
3. If attestation is required, publish a fresh attestation for the known-good
   hash with `attest_upgrade`.
4. Execute `upgrade` to the known-good hash while the contract remains paused.
5. Re-run post-upgrade checks: `get_provenance`, `get_health`,
   `get_health_probes`, `get_attestation_required`, `get_version`, and a sample
   register/verify round-trip on non-production or a controlled test account.
6. Only after checks pass, unpause with `set_paused(false, reason_code)`.

### Forward-fix

Use forward-fix when rollback is unsafe (for example, new state/schema changes
are incompatible with the previous binary) or the known-good hash has its own
risk.

1. Keep the contract paused.
2. Build and review a fixed WASM from the pinned toolchain; record its hash.
3. Stage/attest the fixed hash as required by policy.
4. Apply upgrade to the fixed hash.
5. Run the same post-upgrade verification queries before unpausing.
6. If the fix cannot be produced quickly, keep paused and escalate.

### Communication template

```text
[TrustBridge Contract Incident]
Network: <testnet|mainnet>
Contract: <CONTRACT_ID>
Status: <paused|unpaused>
Impact: <writes blocked / verification unavailable / etc.>
Detected: <UTC timestamp / ledger>
Applied WASM hash: <hash>
Previous known-good hash: <hash>
Health check failure: <command + observed result>
Current action: <pause/rollback/forward-fix>
Next update ETA: <UTC timestamp>
Operator contact: <name/channel>
```

### Operator checklist

- [ ] Writes paused (`Paused` / `is_paused` confirmed)
- [ ] Provenance, attestation, version, and health captured
- [ ] Known-good or fixed WASM hash identified and reviewed
- [ ] Upgrade rehearsed where feasible
- [ ] Post-upgrade verification queries passed
- [ ] Stakeholders notified with communication template
- [ ] Incident notes include hashes, commands, and outcome

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
| `make check-export-layout` | Verify an export JSON matches the v1 layout snapshot (`FILE=...`) |
| `make invoke-verify` | Verify a contributor (admin or verifier role) |
| `make invoke-revoke-verification` | Revoke verification (admin or verifier role) |
| `make invoke-remove` | Remove a registration |
| `make invoke-batch-remove` | Directly remove a batch of registrations |
| `make invoke-set-batch-remove-threshold` | Set dual-control threshold for batch removal |
| `make invoke-get-batch-remove-threshold` | Read configured dual-control threshold |
| `make invoke-propose-batch-remove` | Propose dual-control batch removal (admin) |
| `make invoke-execute-batch-remove` | Execute pending dual-control batch removal (second admin key) |
| `make invoke-cancel-batch-remove` | Cancel pending dual-control batch removal (admin) |
| `make invoke-get-pending-batch-remove` | Read pending dual-control proposal |
| `make bulk-verify-dry-run` | Dry-run bulk verify from `BULK_VERIFY_FILE` |
| `make bulk-verify` | Bulk verify with pacing and audit log |
| `make bulk-revoke-dry-run` | Dry-run bulk revoke from `BULK_REVOKE_FILE` |
| `make bulk-revoke` | Bulk revoke with audit log |
| `make testnet-checklist` | Run the testnet smoke checklist |
| `make demo-e2e` | Run the cross-repo E2E demo (register → verify → lookup → export) |
| `make futurenet-smoke` | Deploy + smoke-check a fresh Futurenet instance (`ADMIN=G...`) |

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

## Futurenet smoke test

`scripts/futurenet_smoke_test.sh` deploys a fresh contract to Futurenet and checks
that `get_stats` reads `{total: 0, verified: 0}` and that `has_record` returns
`false` for an unregistered username. It is a deploy sanity check, not a
replacement for `cargo test`.

```bash
ADMIN=G... make futurenet-smoke
# or: ADMIN=G... ./scripts/futurenet_smoke_test.sh
```

The script prints the failing check by name and exits non-zero. If the deploy
phase reports `429`, `too many requests`, `rate limit`, or a friendbot/funding
error, it prints the faucet remediation directly: reuse an already-funded
identity, retry with exponential backoff (30s → 60s → 120s → 240s), or fall back
to testnet. Those limits are per account/IP and per time window, so repeated
`stellar keys generate --fund` calls make the problem worse.

Full contributor walkthrough:
[FUTURENET_ONBOARDING.md § Faucet rate limits and workarounds](FUTURENET_ONBOARDING.md#faucet-rate-limits-and-workarounds).
Threat-model rationale: [SECURITY.md § Futurenet Deploy Smoke Workflow](SECURITY.md#futurenet-deploy-smoke-workflow).

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

**Output schema** (stable field names for dashboard/indexer consumers): the
field order and integer widths are pinned as a v1 snapshot in
[ABI.md § Export page byte layout (v1)](ABI.md#export-page-byte-layout-v1).
Verify any generated file with
`make check-export-layout FILE=registry-export-testnet.json`.

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
| `REPORT_FILE` | No | Writes a versioned JSON dry-run repair report with counter proofs (see below) |

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

For machine consumers, set `REPORT_FILE` to emit a versioned JSON report with
the same categories plus `get_stats()` counter proofs:

```bash
CONTRACT_ID=$CONTRACT_ID ADMIN_SOURCE=admin REPORT_FILE=repair-report.json \
  ./scripts/validate_registry.sh registry-export-testnet.json
```

The report shape (`schema_version`, `export`, `counters`, `mismatches`,
`result`) is pinned in [ABI.md § Dry-run repair report](ABI.md#dry-run-repair-report);
operator interpretation is in
[CONTRACT_HEALTH.md § Dry-run repair report](CONTRACT_HEALTH.md#dry-run-repair-report).
The script remains validate-only — the report never triggers a write.

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

1. The CI `make check` job builds the WASM via `stellar contract build`.
2. Its _Compute and verify WASM hash pin_ step runs `make wasm-hash-pin WASM_HASH_STRICT=1`,
   which runs `sha256sum` on `target/wasm32v1-none/release/trustbridge_contract.wasm`.
3. The computed hash is compared to the value in `wasm-hash.pin`.
4. A mismatch fails CI with clear instructions to update the pin.
5. CI runs the gate with `WASM_HASH_STRICT=1`, so an unpinned `PLACEHOLDER` value also
   fails. The computed hash is written to the job summary so it can be copied into the pin.

### Reproducible inputs

The hash is only stable across machines when every input that ends up in the
WASM is pinned. `stellar contract build` embeds the rustc version (`rsver`),
the soroban-sdk version (`rssdkver`), and the CLI version (`cliver`) as
contract metadata, so the pin depends on:

- the Rust toolchain in `rust-toolchain.toml` (pinned, not `stable`),
- Stellar CLI `26.1.0` (the CI `make check` job installs this exact release),
- the committed `Cargo.lock` (dependency versions).

Changing any of these is a WASM change: re-pin in the same commit. The build
output does not depend on the checkout path, so a Linux workstation with the
same three inputs reproduces the CI hash.

### Operator flow for (re)pinning

1. Land the contract change and wait for a green `make build` (locally or in CI).
2. Run `make wasm-hash-update` on the release commit (or copy the SHA-256 from the CI job
   summary into `wasm-hash.pin`, replacing the previous hash / `PLACEHOLDER` line).
3. Run `make wasm-hash-pin WASM_HASH_STRICT=1` — it must print `OK: WASM hash matches pin.`
4. Commit `wasm-hash.pin` in the same PR as the WASM-changing code and include the
   before/after hashes in the PR description.

### Local verification

```bash
# Verify the current build matches the pin:
make wasm-hash-pin

# After an intentional WASM change, update the pin:
make wasm-hash-update
git add wasm-hash.pin
git commit -m "chore: bump wasm-hash.pin after <feature>"
```

### Operator verification before deploy

On the release commit, with the pinned toolchain and Stellar CLI 26.1.0:

```bash
make wasm-hash-pin WASM_HASH_STRICT=1   # must print "OK: WASM hash matches pin."
```

After uploading, `stellar contract fetch --id <CONTRACT_ID> --network <NETWORK> --out-file deployed.wasm`
followed by `sha256sum deployed.wasm` must print the same value as `wasm-hash.pin`.

### Explorer verification

After deploying, verify the uploaded WASM on [Stellar Expert](https://stellar.expert/)
using the explorer for the same network. Stellar Expert displays the hash of the
WASM code stored for the contract; compare it with the local **SHA-256** hash.

1. Set the deployed contract ID and choose the network used for the deployment:

  ```bash
  export CONTRACT_ID=$(jq -r .contract_id deployments/testnet.json)
  export NETWORK=testnet
  ```

  For mainnet, use `deployments/mainnet.json` and set `NETWORK=mainnet`.

2. Compute the hash of the exact release artifact that was deployed. Use the
  `wasm32v1-none` path produced by the recommended Stellar CLI build:

  ```bash
  WASM=target/wasm32v1-none/release/trustbridge-contract.wasm
  sha256sum "$WASM"
  grep -v '^#' wasm-hash.pin | grep -v '^$' | tr -d '[:space:]'
  ```

  The two SHA-256 values must match. If the legacy target was used, compute
  the hash from `target/wasm32-unknown-unknown/release/trustbridge-contract.wasm`
  instead. Do not hash a debug or rebuilt artifact that was not deployed.

3. Open the matching contract page:

  - [Stellar Expert Testnet contract explorer](https://stellar.expert/explorer/testnet/contract/)
  - [Stellar Expert Mainnet contract explorer](https://stellar.expert/explorer/public/contract/)

  Append `$CONTRACT_ID` to the selected URL, or search for the contract ID in
  Stellar Expert. Confirm that the contract address and network are correct.

4. Open the contract's **Code** or **WASM** details and compare its displayed
  SHA-256 hash with the value printed in step 2 and the committed
  `wasm-hash.pin`. Record the contract ID, network, hash, release commit, and
  explorer URL in the deployment runbook.

Never compare a testnet contract with the mainnet explorer, or vice versa.
Explorer availability is a convenience check; CI's pinned hash check and the
local `make wasm-hash-pin` result remain the authoritative artifact checks.

### Strict mode checklist (`WASM_HASH_STRICT=1`)

Strict mode (`WASM_HASH_STRICT=1`) converts pin warnings into hard build failures. It is the gate that guarantees no unpinned code or unverified bytecode ever reaches a deployment environment.

#### CI vs Local Mode Matrix

| Environment / Context | `WASM_HASH_STRICT` Mode | Default Value | Expected Behavior / Purpose |
|-----------------------|-------------------------|---------------|-----------------------------|
| **CI quality gate (`make check` / `make ci`)** | `WASM_HASH_STRICT=1` (Strict) | `1` (via `make ci` / CI script) | **Mandatory gate.** Rejects both `PLACEHOLDER` and hash mismatches. Prevents unpinned code from merging to `main`. |
| **Local development (iterative editing)** | `WASM_HASH_STRICT=0` (Permissive) | `0` (Makefile default) | Allows local builds and fast iteration when `wasm-hash.pin` is set to `PLACEHOLDER`, emitting a non-blocking warning. |
| **Local pre-push verification** | `WASM_HASH_STRICT=1` (Strict) | `1` | Run `make wasm-hash-pin WASM_HASH_STRICT=1` or `make ci` before opening a PR to mirror CI and avoid build failures. |
| **Pre-release & Wave sign-off gate** | `WASM_HASH_STRICT=1` (Strict) | `1` | **Mandatory gate.** Operators must verify the release commit compiles to the exact pinned hash prior to deployment. |
| **Testnet / Mainnet deploy gate** | `WASM_HASH_STRICT=1` (Strict) | `1` | Block deployment if hash does not match `wasm-hash.pin` or if `PLACEHOLDER` is present. |

#### Strict Mode Checklist

Before deploying or merging contract changes, verify every item:

- [ ] **Pinned Toolchain Active**: Verified `rustc --version` matches `rust-toolchain.toml` and `stellar --version` is `26.1.0`.
- [ ] **Artifact Built**: Executed `make build` with target `wasm32v1-none`.
- [ ] **Strict Verification Passes**: Ran `make wasm-hash-pin WASM_HASH_STRICT=1` and received `OK: WASM hash matches pin.`
- [ ] **No PLACEHOLDER in Release**: Confirmed `wasm-hash.pin` contains a 64-character lowercase hexadecimal SHA-256 hash, not `PLACEHOLDER`.
- [ ] **Automated Hash Pin Tests Pass**: Ran `make wasm-hash-pin-test` (and `cargo test --test wasm_hash_pin`) to ensure verification logic invariants hold.
- [ ] **Atomic Commit**: `wasm-hash.pin` is committed in the same commit / PR as the contract code change.
- [ ] **PR Documentation**: Before and after SHA-256 hashes are recorded in the PR description.
- [ ] **On-Chain Post-Deploy Match**: After deploying, fetched bytecode (`stellar contract fetch`) matches `wasm-hash.pin`.

### Rotating pins after intentional WASM changes

Whenever contract code (`src/`), dependencies (`Cargo.lock`), compiler version (`rust-toolchain.toml`), or Stellar CLI version changes, the compiled WASM binary changes. Follow this exact rotation sequence:

1. **Build the new artifact**:
   ```bash
   make build
   ```
2. **Rotate the pin file**:
   ```bash
   make wasm-hash-update
   ```
   This replaces `PLACEHOLDER` or the previous SHA-256 in `wasm-hash.pin` with the new actual hash, then automatically runs `make wasm-hash-pin WASM_HASH_STRICT=1` to verify.
3. **Run automated pin gate tests**:
   ```bash
   make wasm-hash-pin-test
   ```
4. **Inspect the git diff**:
   ```bash
   git diff wasm-hash.pin
   ```
   Ensure only the single 64-character hash line was updated.
5. **Commit atomically**:
   ```bash
   git add wasm-hash.pin
   git commit -m "chore: bump wasm-hash.pin after <feature>"
   ```
6. **Include hash metadata in PR**:
   Record both the previous and new SHA-256 hashes in the PR description so reviewers can cross-check with the CI build output.

### Common failure logs and troubleshooting

#### 1. Strict mode rejects PLACEHOLDER
```text
WARNING: wasm-hash.pin contains PLACEHOLDER — run 'make wasm-hash-update' to pin.
ERROR: WASM_HASH_STRICT=1 requires a real pinned hash.
make: *** [Makefile:280: wasm-hash-pin] Error 1
```
- **Cause**: `wasm-hash.pin` contains `PLACEHOLDER` instead of a real SHA-256 hash while running under `WASM_HASH_STRICT=1` (standard in CI, `make ci`, or pre-deploy checks).
- **Remediation**:
  1. Build the contract: `make build`.
  2. Rotate the pin: `make wasm-hash-update`.
  3. Verify: `make wasm-hash-pin WASM_HASH_STRICT=1`.
  4. Commit `wasm-hash.pin`.

#### 2. WASM hash mismatch (drift or unpinned change)
```text
WASM SHA-256: 4a2b8e39f...
ERROR: WASM hash mismatch! Expected: da94f06adb...  Actual: 4a2b8e39f...
Run 'make wasm-hash-update' if this change is intentional.
make: *** [Makefile:284: wasm-hash-pin] Error 1
```
- **Cause**: The compiled artifact's SHA-256 differs from the pinned value in `wasm-hash.pin`.
- **Remediation**:
  - *If intentional change*: Run `make wasm-hash-update`, test with `make wasm-hash-pin WASM_HASH_STRICT=1`, and commit `wasm-hash.pin`.
  - *If unintentional drift*: Run `git status` and `git diff` to identify unintended file changes. Verify your compiler matches `rust-toolchain.toml` and Stellar CLI is `26.1.0`.

#### 3. Missing WASM artifact
```text
ERROR: No WASM artifact found. Run 'make build' first.
make: *** [Makefile:274: wasm-hash-pin] Error 1
```
- **Cause**: `make wasm-hash-pin` was run before building the contract binary, so `target/wasm32v1-none/release/trustbridge_contract.wasm` does not exist.
- **Remediation**: Run `make build` (or `stellar contract build`) to produce the release WASM before hashing.

#### 4. Host environment or toolchain divergence
```text
CI log:    WASM SHA-256: da94f06adb...
Local log: WASM SHA-256: c39a1172fb...
```
- **Cause**: Local build host uses a different toolchain version than CI.
- **Remediation**:
  1. Verify active rustc: `rustc --version` (must match `rust-toolchain.toml`).
  2. Verify Stellar CLI: `stellar --version` (must be `26.1.0`).
  3. Verify target: `rustup target list --installed | grep wasm32v1-none`.
  4. Rebuild from clean state: `make clean && make build`.

### Mainnet and Testnet pre-deploy checklist addition

- [ ] `make wasm-hash-pin WASM_HASH_STRICT=1` passes locally on the exact release commit.
- [ ] `wasm-hash.pin` committed hash matches the CI artifact hash shown in CI logs.
- [ ] `make wasm-hash-pin-test` passes without failures.
- [ ] Hash recorded in deploy runbook before running `make deploy-testnet` or `make deploy-mainnet`.

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

### Local wasm-size gate

Before opening a PR, always run the wasm size check locally to catch oversized
builds early:

```bash
make wasm-size
```

This command builds the optimized release WASM and compares its size against
`WASM_SIZE_LIMIT` in the Makefile. If the build exceeds the budget, the command
exits with a non-zero status and prints the current size vs. the limit.

**Important**: Always run `make wasm-size` after making changes to the contract
code, especially when adding new features or dependencies. CI runs this check
immediately after the release build, so a size increase beyond the budget will
fail the CI job.

### Intentional limit bump process

If a new feature legitimately requires increasing the WASM size budget, follow
this process:

1. **Document the reason** in the PR description: explain why the size increase
   is necessary and reference the relevant benchmark data (see
   [BENCHMARK_BUDGETS.md](./BENCHMARK_BUDGETS.md)).
2. **Update the Makefile**: increase `WASM_SIZE_LIMIT` in the Makefile.
3. **Update CI**: increase the corresponding limit in
   `.github/workflows/ci.yml`.
4. **Update documentation**: update this section (DEPLOYMENT.md) to reflect the
   new budget.
5. **Get approval**: the PR must be reviewed and approved by maintainers before
   merging.

**Note**: Debug builds are not compared against the release limit. Only
optimized release builds are checked.

### Troubleshooting

If `make wasm-size` reports a size that differs from what you expect:

- **Stellar contract build vs. cargo build**: The check uses
  `stellar contract build` (the same command used for deployment), not plain
  `cargo build`. If you see a size discrepancy, ensure you're comparing release
  builds, not debug builds.
- **Feature flags**: Size can vary based on enabled feature flags. Check the
  Makefile for any feature flags that affect the build.
- **wasm-opt**: The release build runs wasm-opt for size optimization. If you're
  building locally without wasm-opt, the size will be larger.
- **wasm-hash.pin**: The wasm hash pin is separate from the size check. See the
  [WASM Hash Verification](#wasm-hash-verification) section for details.

### Related documentation

- [BENCHMARK_BUDGETS.md](./BENCHMARK_BUDGETS.md) — Benchmark data and size
  budgets for different features
- [CONTRIBUTING.md](./CONTRIBUTING.md) — Contribution guidelines and quality
  gates
- [DEPLOYMENT.md](./DEPLOYMENT.md) — This file (deployment guide)

## Provenance digests: SBOM and source (Issue #224)

`WasmProvenance` carries two digests beyond `wasm_hash`:

| Field | Meaning |
|-------|---------|
| `sbom_hash` | SHA-256 of the SBOM generated for this build |
| `source_hash` | SHA-256 of the source tree the WASM was built from |

Both are `Option<BytesN<32>>`. **Every** provenance write path sets them
explicitly as `None` — the single-admin `upgrade` and the multisig
`execute_upgrade_proposal` alike (Issue #400; the multisig path previously
omitted the fields entirely, which did not default them, it failed to compile).
The chain cannot know either digest at upgrade time, so the operator backfills
them right after the upgrade:

```bash
stellar contract invoke \
  --id $CONTRACT_ID --source-account admin-identity --network testnet --send=yes \
  -- set_provenance_digests --sbom_hash $SBOM_SHA256 --source_hash $SOURCE_SHA256
```

Passing `None` for either argument leaves the stored value untouched, so the two
can be filled in independently.

Both upgrade paths behave identically here, so a multisig-executed upgrade needs
the same `set_provenance_digests` follow-up as an admin-executed one.

**Migration.** Provenance records written before these fields existed decode with
both digests absent, which reads as "not recorded" rather than "verified empty".
The migration is one `set_provenance_digests` call per deployed instance; there is
no automatic backfill, because only the operator holds the build outputs.

Suggested way to produce the values:

```bash
sha256sum target/wasm32v1-none/release/trustbridge_contract.wasm   # wasm_hash
sha256sum sbom.spdx.json                                           # sbom_hash
git archive --format=tar HEAD | sha256sum                          # source_hash
```

## Pre-upgrade reproducible-build check (Issue #225)

`assert_build(hash)` compares a locally built hash against the hash in stored
provenance and fails with a typed error instead of a mismatched deploy:

| Situation | Result |
|-----------|--------|
| Hash equals stored provenance | `Ok(())` |
| No provenance recorded yet (never upgraded) | `ProvenanceMissing` (36) |
| Hash differs | `ProvenanceMismatch` (37) |

It is a read-only check on **provenance only**. It does not read, require, or
consume an upgrade attestation — `attest_upgrade` remains a separate, optional
control over the *next* upgrade, while `assert_build` describes what is deployed
*now*. On a contract that has never been upgraded there is nothing to compare
against, and the check fails closed rather than passing.

```bash
stellar contract build
HASH=$(sha256sum target/wasm32v1-none/release/trustbridge_contract.wasm | cut -d' ' -f1)
make assert-build CONTRACT_ID=$CONTRACT_ID WASM_HASH=$HASH
```

## Version Inspection for CLI Operators

Contract entrypoints (`version()`, `get_version()`, `migrate()`) return and accept
`(u32, u32, u32)` tuples, which `stellar contract invoke` prints as JSON arrays
(e.g. `[1, 1, 0]`). In Rust/CLI diagnostics (`src/version.rs`), `Display`
formats `Version` as `"1.1.0"`, `MigrationState` as `not_required` / `pending` /
`in_progress` / `completed` / `failed`, and `CompatibilityInfo` as
`current=1.0.0 target=1.1.0 migration_required=true breaking_changes=false data_migration_required=true`.

```bash
stellar contract invoke --id $CONTRACT_ID --source deployer --network testnet \
  -- version
# Output: [1,1,0]  (equivalent to Version::Display "1.1.0")
```
