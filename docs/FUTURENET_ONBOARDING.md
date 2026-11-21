# Futurenet Contributor Onboarding

End-to-end guide for new contributors to interact with trustbridge-contract on Futurenet: faucet → deploy → register → verify, with expected on-chain events at each step.

No mainnet keys are required for the happy path.

Related docs: [DEPLOYMENT](./DEPLOYMENT.md) · [ABI](./ABI.md) · [ADMIN_RUNBOOK](./ADMIN_RUNBOOK.md) · [EVENT_INDEXING](./EVENT_INDEXING.md)

---

## Prerequisites

| Tool | Minimum version | Install |
|------|-----------------|---------|
| Rust | 1.84 | `rustup update stable` |
| wasm32v1-none target | — | `rustup target add wasm32v1-none` |
| Stellar CLI | 26.x | `cargo install --locked stellar-cli@26.1.0` |

## Environment variables

Copy `.env.example` to `.env` and set:

```bash
export NETWORK=futurenet
export SOURCE=contributor          # Stellar CLI identity name
export ADMIN=$SOURCE               # for single-identity testing; use a separate admin in production
```

All commands below assume these variables are set. Futurenet RPC, passphrase,
and Friendbot differ from Testnet — run `make network-params NETWORK=futurenet`
or see [DEPLOYMENT.md](DEPLOYMENT.md#futurenet-vs-testnet-parameters-issue-671).

---

## Makefile Targets

The repository provides several Makefile targets for common operations. Run `make help` to see all available targets.

### Deploy Targets

| Target | Description | Required Variables |
|--------|-------------|---------------------|
| `build` | Build optimized WASM via Stellar CLI | None |
| `build-legacy` | Build with cargo directly (wasm32-unknown-unknown) | None |
| `deploy-testnet` | Deploy contract to testnet | `SOURCE`, `ADMIN`, `CONTRACT` |
| `deploy-mainnet` | Deploy contract to mainnet | `SOURCE`, `ADMIN`, `CONTRACT` |
| `futurenet-smoke` | Run complete deploy and read-only smoke workflow | `SOURCE`, `ADMIN`, `IDENTITY` |

### Invoke Targets

| Target | Description | Required Variables |
|--------|-------------|---------------------|
| `invoke-register` | Register GitHub username to Stellar address | `CONTRACT_ID`, `GITHUB_USER`, `STELLAR_ADDR`, `SOURCE`, `NETWORK` |
| `invoke-lookup` | Look up Stellar address by GitHub username | `CONTRACT_ID`, `GITHUB_USER`, `SOURCE`, `NETWORK` |
| `invoke-init` | Initialize contract (admin only) | `CONTRACT_ID`, `ADMIN`, `SOURCE`, `NETWORK` |
| `invoke-stats` | Get contract stats (total, verified) | `CONTRACT_ID`, `SOURCE`, `NETWORK` |
| `invoke-verify` | Verify a user (admin/verifier only) | `CONTRACT_ID`, `STELLAR_ADDR`, `SOURCE`, `NETWORK` |
| `invoke-revoke-verification` | Revoke verification (admin only) | `CONTRACT_ID`, `STELLAR_ADDR`, `SOURCE`, `NETWORK` |
| `invoke-get-all-registered` | Get all registered users (admin only) | `CONTRACT_ID`, `SOURCE`, `NETWORK` |
| `invoke-export-paginated` | Export registries with pagination | `CONTRACT_ID`, `SOURCE`, `NETWORK` |
| `invoke-public-paginated` | Public paginated read (no admin) | `CONTRACT_ID`, `SOURCE`, `NETWORK` |
| `invoke-remove` | Remove a user (self or admin) | `CONTRACT_ID`, `STELLAR_ADDR`, `SOURCE`, `NETWORK` |
| `invoke-set-paused` | Pause/unpause contract (admin only) | `CONTRACT_ID`, `PAUSED`, `SOURCE`, `NETWORK` |
| `invoke-extend-ttl` | Extend contract TTL | `CONTRACT_ID`, `SOURCE`, `NETWORK` |

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `NETWORK` | `testnet` | Stellar network (testnet/futurenet/mainnet) |
| `SOURCE` | `default` | Stellar CLI identity name |
| `ADMIN` | `$(stellar keys address $(SOURCE))` | Admin address |
| `CONTRACT_ID` | `$(CONTRACT)` | Contract ID (alias for `CONTRACT`) |
| `CONTRACT` | — | Contract ID |
| `GITHUB_USER` | — | GitHub username |
| `STELLAR_ADDR` | — | Stellar G-address |
| `FUTURENET_RPC_URL` | `https://rpc-futurenet.stellar.org` | Futurenet RPC endpoint |
| `FUTURENET_FRIENDBOT_URL` | `https://friendbot-futurenet.stellar.org` | Futurenet Friendbot endpoint |
| `FUTURENET_DRY_RUN` | `false` | Set to `true` to print commands without executing |

---

The repository pins the Futurenet RPC and Friendbot endpoints in the Makefile. Use the smoke target to print the complete workflow without requiring a funded account or contract credentials:

```bash
make futurenet-smoke FUTURENET_DRY_RUN=true
```

## Auth Identity Gotchas

- **Identity names are local, not on-chain.** `stellar keys generate contributor --network futurenet --fund` only creates a *local* CLI identity named `contributor`; it does not reserve that name anywhere on-chain. Two contributors on the same machine (CI runner, shared devbox) must use distinct identity names (e.g. `contributor-$USER`) or the second `generate` silently overwrites the first identity's keypair in `~/.config/stellar/identity/`.
- **`SOURCE` and `ADMIN` must resolve to the same funded identity** the first time you deploy, unless you intentionally want a separate admin. If `ADMIN` is set to an address with no matching local key, `initialize` will deploy fine but every subsequent admin-gated `invoke-*` target fails with a signing error, not an auth error — because the CLI can't find a signer for `ADMIN`, not because the contract rejected the call.
- **Re-running `stellar keys generate <name> --fund` on an existing identity name is a no-op for the keypair** but still calls Friendbot again, which counts against the rate limit below. Use `stellar keys address <name>` to check if an identity already exists before regenerating.

## Friendbot Rate Limits

- The pinned Futurenet Friendbot (`https://friendbot-futurenet.stellar.org`) rate-limits funding requests **per account and per source IP**, not just per account. Funding several identities back-to-back from the same CI runner or shared network can trip the IP-level limit even though each individual account has only been funded once.
- A rate-limited Friendbot call returns an HTTP error (commonly 400/429) and the CLI surfaces this as a generic funding failure — it does **not** retry automatically. If `stellar keys generate ... --fund` fails, wait before retrying rather than looping the command, which only adds more rejected requests against the same limit window.
- Funding is idempotent per account: if an identity is already funded, a repeated `--fund` call is unnecessary. Check balance first with `stellar keys address <name>` + a balance query before re-funding.

## Common RPC Errors Linked From the Smoke Script

`scripts/futurenet_smoke_test.sh` surfaces these failure modes; use this table to map its error output to the fix:

| Smoke script message | Likely cause | Fix |
|---|---|---|
| `connection refused` / timeout on RPC calls | `FUTURENET_RPC_URL` points at a stale or unreachable endpoint | Confirm you're using the pinned endpoint (`https://rpc-futurenet.stellar.org`); only override it with a known-compatible service |
| `tx submission failed: txBadSeq` | The source account's sequence number is stale (e.g. a prior invoke partially submitted) | Re-fetch the account with `stellar keys address <name>` / re-run the smoke script; do not manually bump sequence numbers |
| `signing key not found for <address>` | `ADMIN` or `SOURCE` resolves to an address with no local identity (see Auth Identity Gotchas above) | Verify `stellar keys address $SOURCE` matches the address you expect, and that `ADMIN` was set from a local identity, not pasted from elsewhere |
| `account not found` from Friendbot or RPC | The identity was generated but never successfully funded, or funding silently failed under the Friendbot rate limit | Re-check with `stellar keys address <name>`, then retry funding after waiting out the rate-limit window |

## Step 1 — Faucet: fund your Futurenet account

```bash
# Generate a new identity (skip if you already have one)
stellar keys generate contributor --network futurenet --fund

# Verify the account is funded
stellar keys address contributor
# Expected: a G... address
```

The equivalent pinned Friendbot endpoint is `https://friendbot-futurenet.stellar.org`. The smoke target does not fund an account automatically; run the command above first to respect Friendbot rate limits and avoid silently funding the wrong identity.

**Expected outcome:** Account exists on Futurenet with enough XLM to cover deploy + invoke fees.

*No on-chain contract event at this step (account funding is a Horizon operation, not a contract event).*

## Step 2 — Build the contract

```bash
make build
# Output: target/wasm32v1-none/release/trustbridge-contract.wasm
```

The WASM is built with LTO and optimised for size. The output file is ~85 KB.

## Step 3 — Deploy to Futurenet

```bash
export ADMIN=$(stellar keys address contributor)
NETWORK=futurenet ADMIN=$ADMIN SOURCE=contributor ./scripts/deploy.sh
```

Or run the complete deploy and read-only smoke workflow through Make:

```bash
make futurenet-smoke   ADMIN=$ADMIN SOURCE=contributor IDENTITY=contributor
```

Override `FUTURENET_RPC_URL` or `FUTURENET_FRIENDBOT_URL` only when using a known compatible Futurenet service. `make -n futurenet-smoke` prints the resolved command without requiring secrets or contacting the network.

The deploy script:
- Builds the WASM if missing.
- Runs `stellar contract deploy` and writes `deployments/futurenet.json`.
- Calls `initialize(admin)`.

```bash
export CONTRACT_ID=$(cat deployments/futurenet.json | python3 -c "import sys,json; print(json.load(sys.stdin)['contract_id'])")
```

*No user-visible contract event — `initialize` does not emit an event in the current ABI.*

**Post-deploy check:**

```bash
stellar contract invoke   --id $CONTRACT_ID   --source-account contributor   --network futurenet   -- get_stats
# Expected: {"total":0,"verified":0}
```

## Step 4 — Register your GitHub username

```bash
export GITHUB_USER=your-github-username
export STELLAR_ADDR=$(stellar keys address contributor)

stellar contract invoke   --id $CONTRACT_ID   --source-account contributor   --network futurenet   --send=yes   -- register   --github-username $GITHUB_USER   --stellar-address $STELLAR_ADDR
```

Or use the Make target:

```bash
make invoke-register   CONTRACT_ID=$CONTRACT_ID   GITHUB_USER=$GITHUB_USER   STELLAR_ADDR=$STELLAR_ADDR
