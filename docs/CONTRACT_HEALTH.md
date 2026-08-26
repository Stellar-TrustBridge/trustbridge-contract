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
  "version": [1, 0, 0],
  "total": 42,
  "verified": 17,
  "cooldown_secs": 86400,
  "cooldown_remaining_secs": 0,
  "attestation_present": false
}
```

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
