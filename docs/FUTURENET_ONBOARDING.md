# Futurenet Contributor Onboarding

End-to-end guide for new contributors to interact with **trustbridge-contract** on Futurenet:
faucet → deploy → register → verify, with expected on-chain events at each step.

No mainnet keys are required for the happy path.

Related docs: [DEPLOYMENT](DEPLOYMENT.md) · [ABI](ABI.md) · [ADMIN_RUNBOOK](ADMIN_RUNBOOK.md) · [EVENT_INDEXING](EVENT_INDEXING.md)

---

## Prerequisites

| Tool | Minimum version | Install |
|------|----------------|---------|
| Rust | 1.84 | `rustup update stable` |
| `wasm32v1-none` target | — | `rustup target add wasm32v1-none` |
| Stellar CLI | 26.x | `cargo install --locked stellar-cli@26.1.0` |

---

## Environment variables

Copy `.env.example` to `.env` and set:

```bash
export NETWORK=futurenet
export SOURCE=contributor          # Stellar CLI identity name
export ADMIN=$SOURCE               # for single-identity testing; use a separate admin in production
```

All commands below assume these variables are set.

---

## Step 1 — Faucet: fund your Futurenet account

```bash
# Generate a new identity (skip if you already have one)
stellar keys generate contributor --network futurenet --fund

# Verify the account is funded
stellar keys address contributor
# Expected: a G... address
```

**Expected outcome:** Account exists on Futurenet with enough XLM to cover deploy + invoke fees.

**No on-chain contract event at this step** (account funding is a Horizon operation, not a contract event).

---

## Step 2 — Build the contract

```bash
make build
# Output: target/wasm32v1-none/release/trustbridge-contract.wasm
```

The WASM is built with LTO and optimised for size. The output file is ~85 KB.

---

## Step 3 — Deploy to Futurenet

```bash
export ADMIN=$(stellar keys address contributor)
NETWORK=futurenet ADMIN=$ADMIN SOURCE=contributor ./scripts/deploy.sh
```

The deploy script:

1. Builds the WASM if missing.
2. Runs `stellar contract deploy` and writes `deployments/futurenet.json`.
3. Calls `initialize(admin)`.

```bash
export CONTRACT_ID=$(cat deployments/futurenet.json | python3 -c "import sys,json; print(json.load(sys.stdin)['contract_id'])")
```

**No user-visible contract event** — `initialize` does not emit an event in the current ABI.

Post-deploy check:

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account contributor \
  --network futurenet \
  -- get_stats
# Expected: {"total":0,"verified":0}
```

---

## Step 4 — Register your GitHub username

```bash
export GITHUB_USER=your-github-username
export STELLAR_ADDR=$(stellar keys address contributor)

stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account contributor \
  --network futurenet \
  --send=yes \
  -- register \
  --github-username $GITHUB_USER \
  --stellar-address $STELLAR_ADDR
```

Or use the Make target:

```bash
make invoke-register \
  CONTRACT_ID=$CONTRACT_ID \
  GITHUB_USER=$GITHUB_USER \
  STELLAR_ADDR=$STELLAR_ADDR \
  SOURCE=contributor \
  NETWORK=futurenet
```

**Expected on-chain event:** `RegisteredEvent`

| Field | Value |
|-------|-------|
| Topic symbol | `registered_event` |
| `github_username` | your username |
| `stellar_address` | your G-address |
| `timestamp` | ledger close time (Unix seconds) |

Post-register check:

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account contributor \
  --network futurenet \
  -- get_address --github-username $GITHUB_USER
# Expected: {"stellar_address":"G...","verified":false,"timestamp":<n>}
```

---

## Step 5 — Verify (admin marks contributor as verified)

> In a Wave context, the admin or a Verifier-role key performs this step after confirming your
> GitHub identity off-chain. For local Futurenet testing, use the same identity as admin.

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account contributor \
  --network futurenet \
  --send=yes \
  -- verify \
  --github-username $GITHUB_USER
```

Or use the Make target:

```bash
make invoke-verify \
  CONTRACT_ID=$CONTRACT_ID \
  GITHUB_USER=$GITHUB_USER \
  SOURCE=contributor \
  NETWORK=futurenet
```

**Expected on-chain event:** `VerifiedEvent`

| Field | Value |
|-------|-------|
| Topic symbol | `verified_event` |
| `github_username` | your username |
| `stellar_address` | your G-address |
| `timestamp` | ledger close time (Unix seconds) |

Post-verify check:

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account contributor \
  --network futurenet \
  -- get_address --github-username $GITHUB_USER
# Expected: {"stellar_address":"G...","verified":true,"timestamp":<n>}
```

---

## Event summary

| Step | Action | Event emitted | Topic symbol |
|------|--------|---------------|--------------|
| 1 | Faucet fund | — (Horizon op) | — |
| 2 | Build | — | — |
| 3 | Deploy + initialize | — | — |
| 4 | Register | `RegisteredEvent` | `registered_event` |
| 5 | Verify | `VerifiedEvent` | `verified_event` |

Indexers and dashboards should subscribe to `registered_event` and `verified_event` topic
symbols via the Stellar RPC `getEvents` endpoint. See [EVENT_INDEXING.md](EVENT_INDEXING.md)
for filter patterns and field layouts.

---

## Makefile targets reference

| Target | Description |
|--------|-------------|
| `make build` | Build optimised WASM |
| `make deploy-testnet` | Deploy to testnet (same flow for futurenet with `NETWORK=futurenet`) |
| `make invoke-register` | Register a GitHub username |
| `make invoke-lookup` | Read-only address lookup |
| `make invoke-verify` | Verify a contributor (admin or Verifier role) |
| `make invoke-stats` | Read registry statistics |

---

## Troubleshooting

### `NotInitialized` (error code 2)

The contract was deployed but `initialize(admin)` was not called. Run:

```bash
make invoke-init CONTRACT_ID=$CONTRACT_ID ADMIN=$ADMIN SOURCE=contributor NETWORK=futurenet
```

### `NotAuthorized` (error code 3) on `verify`

The signing identity does not have admin or Verifier role. For local Futurenet testing,
ensure `SOURCE` is the same identity used as `ADMIN` during `initialize`.

### `AlreadyRegistered` on `register`

The username is already mapped to an address. Read the current record:

```bash
stellar contract invoke --id $CONTRACT_ID --source-account contributor \
  --network futurenet -- get_address --github-username $GITHUB_USER
```

If the address matches yours, registration succeeded in an earlier session. Proceed to Step 5.

### `wasm32v1-none` target not installed

```bash
rustup target add wasm32v1-none
```

### Account not funded / insufficient fee

```bash
stellar keys fund contributor --network futurenet
```

### Futurenet node unavailable

Futurenet is a pre-release network; node availability may be intermittent. Check the
[Stellar Developer Discord](https://discord.gg/stellardev) `#futurenet` channel for status.
Use testnet (`NETWORK=testnet`) as a stable fallback.

---

## No mainnet keys required

Every step above runs on `NETWORK=futurenet`. The Futurenet Friendbot (`--fund`) provides
XLM at no cost. No mainnet keys, no real XLM, and no production contract IDs are needed
for the full onboarding path.

To graduate to testnet, replace `NETWORK=futurenet` with `NETWORK=testnet` and re-run
from Step 1. Testnet also provides free XLM via Friendbot.
