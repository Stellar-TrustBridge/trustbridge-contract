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

## Post-checklist

- Record the contract ID, network passphrase, deployer address, admin address, and commit hash used. Store these in `deployments/testnet.json` or a runbook.
- Confirm no mainnet environment variables (`NETWORK=mainnet`, `ADMIN` with a mainnet G-address) are active in the shell session before running any step above.

## Metadata

Store the contract ID, network passphrase, deployer address, admin address, and commit hash used for the deployment.
