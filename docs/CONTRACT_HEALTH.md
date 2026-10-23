# Contract Health Snapshot

This document describes the `get_health` function added in Issue #210.

Related docs: [ABI](ABI.md) · [ARCHITECTURE](ARCHITECTURE.md) · [DEPLOYMENT](DEPLOYMENT.md)

---

## Why it exists

Previously, an operator checking the contract's status had to make five separate
RPC calls:

1. `is_paused()` — is the circuit breaker active?
2. `version()` — what schema version is deployed?
3. `get_stats()` — how many total / verified registrations?
4. `get_cooldown()` — what is the upgrade timelock?
5. `get_attestation()` — is an upgrade attestation live?

Each call is a network round-trip. Dashboard load times, CI health probes, and
on-call scripts all paid that cost on every check.

`get_health` composes all five reads into a single call that returns one packed
`HealthSnapshot` struct.

---

## Function signature

```rust
pub fn get_health(env: Env) -> Result<HealthSnapshot, ContractError>
```

### Properties

| Property | Value |
|----------|-------|
| **Auth** | None |
| **Mutates** | ❌ |
| **Works while paused** | ✅ |
| **Errors** | `NotInitialized` |

The function intentionally works while the contract is paused — that is exactly
when operators need it most.

---

## HealthSnapshot type

```rust
pub struct HealthSnapshot {
    /// Whether the contract is currently paused.
    pub paused: bool,
    /// Schema version as [major, minor, patch].
    pub version: Vec<u32>,
    /// Total registered contributor count.
    pub total: u32,
    /// Verified contributor count.
    pub verified: u32,
    /// Configured WASM upgrade cooldown in seconds (0 = no cooldown).
    pub cooldown_secs: u64,
    /// Seconds remaining until the upgrade cooldown expires, or 0 if
    /// not in cooldown or no cooldown is configured.
    pub cooldown_remaining_secs: u64,
    /// Whether a non-expired upgrade attestation is currently live.
    pub attestation_present: bool,
}
```

---

## CLI usage

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --network testnet \
  -- get_health
```

Expected output when healthy:

```json
{
  "paused": false,
  "version": [1, 1, 0],
  "total": 42,
  "verified": 17,
  "cooldown_secs": 86400,
  "cooldown_remaining_secs": 0,
  "attestation_present": false
}
```

---

## Pre-Wave probes (Issue #668)

`get_health_probes` returns the three checks operators run before a Wave.
Unlike `get_health` it never errors, so an uninitialized deployment reports
`admin_set: false` instead of `NotInitialized`.

```bash
stellar contract invoke --id $CONTRACT_ID --network testnet -- get_health_probes
```

| Field | Healthy value | Unhealthy → action |
|-------|---------------|--------------------|
| `paused` | `false` | `true` → check `get_pause_reason`, then `unpause` per the runbook |
| `admin_set` | `true` | `false` → contract never initialized; run `initialize` before any Wave traffic |
| `index_gen` | stable between probes | unexpected jump → removals happened; re-sync dashboards (cursors are invalidated) |

---

## Edge cases

| Scenario | Snapshot behavior |
|----------|-------------------|
| Contract not initialized | Returns `NotInitialized` error |
| Contract paused | Returns normally; `paused: true` |
| Zero registrations | `total: 0`, `verified: 0` |
| Cooldown not yet elapsed | `cooldown_remaining_secs > 0` |
| Attestation present but expired | `attestation_present: false` |
| Never upgraded (no last_upgrade timestamp) | `cooldown_remaining_secs: 0` |

---

## Migration from manual probing

Replace five calls:

```typescript
// Before
const paused = await contract.is_paused();
const version = await contract.version();
const stats = await contract.get_stats();
const cooldown = await contract.get_cooldown();
const attestation = await contract.get_attestation();
```

With one:

```typescript
// After
const health = await contract.get_health();
// health.paused, health.version, health.total, health.verified,
// health.cooldown_secs, health.cooldown_remaining_secs, health.attestation_present
```

---

## Invoke-trace playbook for operators (Issue #329)

`get_health` tells you the contract's *state*. When a specific
`stellar contract invoke` fails during an incident, use this section to read
the failure and map it to a cause. Error codes are the `u32` discriminants of
`ContractError` in [`src/error.rs`](../src/error.rs); the full table is in
[ABI.md → ContractError](ABI.md#contracterror-u32-discriminant).

### 1. Reproduce without submitting

Simulate first — it costs nothing and returns the same contract error:

```bash
stellar contract invoke --id "$CONTRACT_ID" --source "$SOURCE" --network "$NETWORK" \
  --send=no -- verify --caller "$CALLER" --github_username octocat
```

For more detail from the CLI (RPC requests, simulation result, diagnostic
events), add the global verbosity flags `--verbose` (`-v`) or
`--very-verbose` (`-vv`). Flag names vary between Stellar CLI releases —
confirm against `stellar contract invoke --help` for your installed version
(the repo targets 26.x) rather than copying flags from older guides.

### 2. Read the trace

A failed invoke surfaces the contract error as `Error(Contract, #N)` in the
CLI output and in the diagnostic events of the simulation / transaction
result. `N` is the `ContractError` code. Other shapes mean the contract never
returned its own error:

| Trace shape | Meaning |
|---|---|
| `Error(Contract, #N)` | Contract rejected the call — look up `N` below |
| `Error(Auth, InvalidAction)` / auth errors | Missing or wrong signature for a `require_auth` address (wrong `--source`, or the caller arg ≠ signer) |
| `Error(Storage, MissingValue)` / archived entry | Entry TTL expired — restore it, then run the TTL keeper ([STORAGE_RENT.md](STORAGE_RENT.md)) |
| `Error(Budget, ExceededLimit)` | CPU/memory budget exceeded — see [BENCHMARK_BUDGETS.md](BENCHMARK_BUDGETS.md); usually an oversized batch |
| `Error(WasmVm, ...)` | Contract trapped (panic / abort) — capture the full trace and escalate as a bug |

### 3. Common contract errors → likely cause → first action

| Code | Error | Likely cause during an incident | First action |
|---|---|---|---|
| 1 | `AlreadyInitialized` | Deploy script re-ran `initialize` against a live contract | None needed; contract is already set up |
| 2 | `NotInitialized` | Wrong `CONTRACT_ID` / network, or fresh deploy never initialized | Check `--id` and `--network`; `get_health` also returns this |
| 3 | `NotAuthorized` | Caller lacks the Admin/Verifier/Upgrader role, or role expired | `has_role` / role listing for the caller; see [ADMIN_RUNBOOK.md](ADMIN_RUNBOOK.md) |
| 4 | `NotRegistered` | Username never registered, removed, or typo | `get_address` for the username |
| 5 | `AlreadyVerified` | Duplicate verify (retry after a timeout that actually succeeded) | Treat as success; confirm with `get_address` |
| 6 | `NotVerified` | Revoke on an unverified username | Confirm state with `get_address` |
| 7 | `Paused` | Contract paused (maintenance or emergency) | `get_health` → `paused: true`; see pause procedure in [ADMIN_RUNBOOK.md](ADMIN_RUNBOOK.md) |
| 8 | `CooldownActive` | `upgrade` before the timelock elapsed, or per-user action cooldown | `get_health` → `cooldown_remaining_secs`; wait it out |
| 9 | `InvalidVersion` | `migrate` target not greater than current version | `version()`; fix the migration target |
| 10 | `InvalidRole` | Unknown role discriminant passed to a role call | Use the `Role` values in [ABI.md](ABI.md#role-u32-discriminant) |
| 11 | `InvalidUsername` | Empty, too long, or non-GitHub characters | `is_username_valid` for the input |
| 12 | `AttestationExpired` | Upgrade attestation lapsed before `upgrade` ran | Re-attest, then upgrade promptly |
| 13 | `UnattestedWasm` | Upgrade hash differs from the attested hash | Compare against `wasm-hash.pin` / attestation |
| 14 | `InvalidBatchSize` | Batch empty or larger than the configured max | Split the batch ([ABI.md → Batch size limits](ABI.md#batch-size-limits-issue-227)) |
| 15 | `InvalidReasonCode` | Unknown revoke reason code | Use a `RevokeReason` value |
| 16 | `ZeroAddress` | Zero/burn address supplied as a Stellar address | Fix the input address |

Codes above 16 (challenge, reservation, admin-transfer, allowlist,
provenance errors) are listed in the ABI table; `src/error.rs` is the source
of truth if the two ever disagree.
