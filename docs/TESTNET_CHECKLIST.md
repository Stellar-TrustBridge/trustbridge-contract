# Testnet Checklist

Before sharing a testnet deployment, confirm the full registry lifecycle. Run every step in order.

## Prerequisites

| Variable | Required | Description |
|----------|----------|-------------|
| `NETWORK` | Yes | Must be `testnet` — never omit or set to `mainnet` for this checklist |
| `ADMIN` | Yes | G-address of the contract admin account |
| `SOURCE` | Yes | Stellar CLI identity name (must be a funded testnet account) |
| `CONTRACT_ID` | Only after deploy | Contract ID written by `deploy.sh` |

## Checklist

1. **Build** the optimized WASM from a clean checkout.
   ```bash
   make build
   ```
2. **Deploy** to Stellar testnet.
   ```bash
   make deploy-testnet
   ```
3. **Verify** the WASM hash from the deployment output and record it.
   Confirm `deployments/testnet.json` contains the `contract_id`.
4. **Initialize** the contract with the admin address (if not auto-initialized by deploy).
   ```bash
   make invoke-init
   ```
   Then run the post-deploy probe before registering anything:
   ```bash
   stellar contract invoke --id $CONTRACT_ID --network testnet --source $SOURCE --send=no -- get_health_probes
   # Expect { "paused": false, "admin_set": true, "index_gen": 0 }; record index_gen as baseline
   # Failure: admin_set=false -> wrong ID/network or initialize missing; paused=true -> check get_pause_reason
   ```
5. **Register** a sample GitHub username with a test Stellar address.
   ```bash
   make invoke-register GITHUB_USER=testuser STELLAR_ADDR=G... SOURCE=deployer
   ```
6. **Look up** the registered username to confirm the record is correct.
   ```bash
   make invoke-lookup GITHUB_USER=testuser SOURCE=deployer
   ```
7. **Verify** the sample contributor as admin.
   ```bash
   make invoke-verify GITHUB_USER=testuser SOURCE=admin CONTRACT_ID=$CONTRACT_ID
   ```
8. **Check stats** to confirm the verified count incremented.
   ```bash
   make invoke-stats SOURCE=deployer CONTRACT_ID=$CONTRACT_ID
   ```
9. **Revoke** verification to confirm the admin can unwrap a verified record.
   ```bash
   make invoke-revoke-verification GITHUB_USER=testuser SOURCE=admin CONTRACT_ID=$CONTRACT_ID
   ```
10. **Remove** the sample registration (admin or registrant).
    ```bash
    make invoke-remove CALLER=$ADMIN GITHUB_USER=testuser SOURCE=admin CONTRACT_ID=$CONTRACT_ID
    ```
11. **Confirm** lookup returns empty for the removed username.
    ```bash
    make invoke-lookup GITHUB_USER=testuser SOURCE=deployer CONTRACT_ID=$CONTRACT_ID
    ```

## Optional CI live smoke

The repository has an opt-in GitHub Actions job for checking Stellar RPC and
argument encoding against a pre-deployed testnet contract. It is invoke-only:
it does not deploy or initialize a contract, so it cannot accidentally call
`initialize` twice. Enable it by setting the repository variable
`TRUSTBRIDGE_LIVE_TESTNET` to `true`.

Configure these repository values before enabling it:

| Name | Type | Description |
|------|------|-------------|
| `TRUSTBRIDGE_LIVE_TESTNET` | Variable | Must be `true` to enable the job |
| `TRUSTBRIDGE_TESTNET_USERNAME` | Variable | Existing username used by `get_address` |
| `TRUSTBRIDGE_TESTNET_CONTRACT_ID` | Secret | Pre-deployed testnet contract ID |
| `TRUSTBRIDGE_TESTNET_SECRET_KEY` | Secret | Testnet source account secret key |

The job imports the secret key into an ephemeral `ci-testnet` Stellar CLI
identity, then invokes `get_stats` and `get_address` on `testnet`. It runs only
for non-PR events in the canonical repository, so fork pull requests never
receive repository secrets. If the opt-in variable is enabled and any required
value is missing, the job fails closed.

## Post-checklist

- Record the contract ID, network passphrase, deployer address, admin address, and commit hash used. Store these in `deployments/testnet.json` or a runbook.
- Confirm no mainnet environment variables (`NETWORK=mainnet`, `ADMIN` with a mainnet G-address) are active in the shell session before running any step above.
- **Protocol-upgrade rehearsal**: Run `make test-rehearsal` before any testnet or mainnet deployment to verify that all on-chain state survives a simulated WASM upgrade. See the rehearsal test in `tests/integration.rs` (`test_protocol_upgrade_rehearsal`) for what "pass" means. This is a prerequisite for mainnet deployment.

## Post-upgrade verification queries (Issue #780)

After a `migrate` call during a testnet protocol upgrade rehearsal (or a
real upgrade), run these read-only `stellar contract invoke` queries
against the upgraded `CONTRACT_ID` before relying on the deployment. Every
command below maps to a real entrypoint in `src/lib.rs` — check `docs/ABI.md`
if the signature ever looks different from what is shown here, since ABI
drift from `src/lib.rs` has happened on this repo before.

1. **Provenance** — confirm the WASM hash and upgrade history recorded on-chain:
   ```bash
   stellar contract invoke \
     --id $CONTRACT_ID --network testnet --source $SOURCE \
     -- get_provenance
   ```
   A `None`/empty result after an upgrade that was supposed to set
   provenance digests means `set_provenance_digests` was never called —
   do not proceed to mainnet.

2. **Health** — confirm the contract reports healthy post-upgrade:
   ```bash
   stellar contract invoke \
     --id $CONTRACT_ID --network testnet --source $SOURCE \
     -- get_health
   ```
   and the lower-level probe breakdown:
   ```bash
   stellar contract invoke \
     --id $CONTRACT_ID --network testnet --source $SOURCE \
     -- get_health_probes
   ```

3. **Attestation flags** — confirm the upgrade attestation was recorded and,
   if the registry requires it, that the requirement flag is still set as
   expected:
   ```bash
   stellar contract invoke \
     --id $CONTRACT_ID --network testnet --source $SOURCE \
     -- get_attestation

   stellar contract invoke \
     --id $CONTRACT_ID --network testnet --source $SOURCE \
     -- get_attestation_required
   ```
   If `get_attestation_required` returns `true` but `get_attestation`
   returns `None`, the upgraded contract is in a state that should block
   mainnet promotion — see `tests/export_page_layout.rs` and
   `docs/ABI.md` for the attestation digest contract.

4. **Version** — confirm the reported version matches the WASM you just deployed:
   ```bash
   stellar contract invoke \
     --id $CONTRACT_ID --network testnet --source $SOURCE \
     -- get_version
   ```

5. **Sample register/verify round-trip** — confirm core read/write paths
   still work end to end on the upgraded contract (reuses steps 5–11 of the
   main checklist above, with a fresh username so it cannot collide with a
   prior run):
   ```bash
   make invoke-register GITHUB_USER=postupgradecheck STELLAR_ADDR=G... SOURCE=deployer CONTRACT_ID=$CONTRACT_ID
   make invoke-lookup GITHUB_USER=postupgradecheck SOURCE=deployer CONTRACT_ID=$CONTRACT_ID
   make invoke-verify GITHUB_USER=postupgradecheck SOURCE=admin CONTRACT_ID=$CONTRACT_ID
   make invoke-stats SOURCE=deployer CONTRACT_ID=$CONTRACT_ID
   make invoke-remove CALLER=$ADMIN GITHUB_USER=postupgradecheck SOURCE=admin CONTRACT_ID=$CONTRACT_ID
   ```

If any of these five checks fails or returns an unexpected value, stop —
do not promote this WASM hash past testnet. File the failure with the
exact command and output; this is the stage where a broken Wave testnet
registry is caught before it reaches mainnet.

## Metadata

Store the contract ID, network passphrase, deployer address, admin address, and commit hash used for the deployment.


## RPC Timeout Triage

When RPC calls timeout or Horizon lags during the export and invoke steps of this checklist, follow this triage procedure instead of abandoning the run.

### Common Timeout Errors

- `request timeout` or `connection refused` from stellar CLI
- `Horizon lag` warning with stale ledger sequence
- `rate limit exceeded` from public RPC endpoints
- `friendbot funding failed` when requesting testnet XLM

### Retry Backoff Strategy

1. **First retry**: Wait 10 seconds, then re-run the failed command.
2. **Second retry**: Wait 30 seconds with exponential backoff (10s → 30s → 90s).
3. **Third retry**: Switch to an alternate RPC endpoint before retrying.

### Alternate RPC Endpoints

Override the default RPC endpoint with environment variables:

```bash
# Testnet public RPC (alternate)
export STELLAR_RPC_URL="https://soroban-testnet.stellar.org"

# Futurenet public RPC
export STELLAR_RPC_URL="https://rpc-futurenet.stellar.org"

# Friendbot for testnet funding
export STELLAR_FRIENDBOT_URL="https://friendbot.stellar.org"
```

### Testnet vs Futurenet Failure Modes

| Symptom | Testnet Behavior | Futurenet Behavior |
|---|---|---|
| RPC timeout | Retry with backoff; public RPC is shared | Use dedicated futurenet RPC endpoint |
| Friendbot failure | Usually transient; retry after 30s | Futurenet friendbot is less reliable |
| Ledger lag | Normal during peak hours; check [CONTRACT_HEALTH.md](./CONTRACT_HEALTH.md) | Expected; futurenet has lower throughput |
| Network passphrase error | Indicates wrong NETWORK env var | Common when mixing testnet/futurenet commands |

### Friendbot Funding Troubleshooting

If `make deploy-testnet` fails because the source account has insufficient funds:

1. Verify the account is on testnet (not futurenet/mainnet).
2. Request funds from friendbot: `curl "https://friendbot.stellar.org?addr=$ADMIN"`
3. Wait 10 seconds for the transaction to confirm.
4. Re-run the deploy step.

### Rate Limiting

Public RPC endpoints have rate limits. If you see `429 Too Many Requests`:

- Wait 60 seconds before retrying.
- Reduce the number of concurrent invocations.
- Consider running the checklist steps sequentially instead of in parallel.

### Related Documentation

- [CONTRACT_HEALTH.md](./CONTRACT_HEALTH.md) — Expected latency and health metrics
- [FUTURENET_ONBOARDING.md](./FUTURENET_ONBOARDING.md) — Futurenet-specific setup
- [DEPLOYMENT.md](./DEPLOYMENT.md) — RPC configuration details in the Makefile
- [DEPLOYMENT.md § Futurenet vs Testnet Parameters](./DEPLOYMENT.md#futurenet-vs-testnet-parameters-issue-671) — side-by-side parameter table (`make network-params`)
- 
