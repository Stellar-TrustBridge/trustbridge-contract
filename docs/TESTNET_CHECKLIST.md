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

## Metadata

Store the contract ID, network passphrase, deployer address, admin address, and commit hash used for the deployment.
