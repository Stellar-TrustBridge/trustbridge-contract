# Admin Runbook

The admin role exists for off-chain GitHub verification and operational recovery.

Related docs: [SECURITY](SECURITY.md) · [ABI](ABI.md) · [DEPLOYMENT](DEPLOYMENT.md) · [EVENT_INDEXING](EVENT_INDEXING.md)

## Routine actions

- Verify contributors only after confirming the GitHub identity off-chain.
- Revoke verification cleanly when contributor identities change or a registration is invalidated.
- Export registered records before large dashboard migrations.
- Keep the admin account in a secure wallet or multisig flow.

---

## Storage TTL Maintenance (Keeper)

Soroban persistent entries expire unless their TTL is extended. A registry with inactive contributors will silently lose its cold entries if not maintained.

### When to run it
Run the `ttl_keeper.sh` script periodically (e.g., weekly or monthly) to walk the entire registry and bump the TTL of every registered contributor.

```bash
CONTRACT_ID=C... SOURCE=keeper-identity NETWORK=testnet ./scripts/ttl_keeper.sh
```

**Notes:**
- **Permissionless**: You do not need to use the contract admin key for this. Any funded identity can pay the transaction fees to extend TTLs.
- **Batching**: The script handles batching automatically to avoid exceeding transaction limits.
- **Dry-run**: You can pass `--dry-run` to test the script without submitting any transactions.

---

## Emergency Pause Lifecycle

In case of a detected security vulnerability, operational incident, or during maintenance windows, the contract admin can pause all state mutations.

### 1. Trigger Pause
To pause the contract:
```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account admin-identity \
  --network testnet \
  --send=yes \
  -- pause
```
This sets the internal `Symbol("pause")` state to `true` and publishes a `PausedEvent`.

### 2. Restore Normal Operations (Unpause)
Once maintenance is complete or the incident is resolved:
```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account admin-identity \
  --network testnet \
  --send=yes \
  -- unpause
```
This restores operations and publishes an `UnpausedEvent`.

### 3. Function Behavior During Pause

While the contract is paused, functions behave as follows:

| Gated & Blocked (Panics with `ContractError::Paused`) | Allowed (Read-only or non-mutating) |
|-------------------------------------------------------|--------------------------------------|
| `register`                                            | `is_paused`                          |
| `remove`                                              | `is_contract_paused`                 |
| `verify`                                              | `get_address`                        |
| `batch_verify`                                        | `has_record`                         |
| `revoke_verification`                                 | `get_role`                           |
| `set_role`                                            | `get_cooldown`                       |
| `remove_role`                                         | `get_version`                        |
| `upgrade`                                             | `version`                            |
| `migrate`                                             | `is_compatible`                      |
| `get_public_paginated`                                | `max_username_len`                   |
|                                                       | `is_username_valid`                  |
|                                                       | `usernames_match`                    |
|                                                       | `get_registered_page`                |
|                                                       | `get_all_registered`                 |
|                                                       | `get_registered_paginated`           |
|                                                       | `get_stats`                          |
|                                                       | `get_verified_count`                 |
|                                                       | `get_provenance`                     |
|                                                       | `get_attestation`                    |

*Note: Administrative read operations (`get_all_registered`, `get_registered_page`, `get_registered_paginated`) remain accessible to the admin to facilitate data export during maintenance.*

### 4. Simulation & Validation
You can simulate the entire pause/unpause flow using the provided Makefile target:
```bash
make simulate-pause-flow CONTRACT_ID=$CONTRACT_ID NETWORK=testnet SOURCE=admin-identity
```

### 5. Ops Resource & Performance Notes
- **Metered CPU/Memory Cost**: Initiating a pause/unpause uses minimal Soroban resources (~130,000 CPU instructions and ~10KB RAM) as it only mutates a single instance storage boolean flag and publishes one event.
- **Client Latency**: Client check integrations calling `is_paused` locally via RPC simulate in 0ms and consume no network gas.

---

## Recovery notes

If an admin key is rotated in a future contract version, announce the new admin address and keep the old deployment metadata available for auditors.

---

## Guardian Circuit Breaker (Issue #196)

The guardian is a designated address that can freeze writes without holding the
admin key, useful when the admin key is in cold storage or when a rapid
incident response is needed.

### Who can do what

| Action | Admin | Guardian |
|--------|-------|----------|
| `emergency_pause` (trip circuit breaker) | ✅ | ✅ |
| `clear_emergency_pause` (lift circuit breaker) | ✅ | ❌ |
| `pause` / `unpause` (normal pause) | ✅ | ❌ |
| `upgrade` | ✅ | ❌ |
| `set_guardian` / `remove_guardian` | ✅ | ❌ |

The guardian **cannot upgrade the contract** and **cannot clear the emergency
pause**. This intentionally requires a slower admin review before writes resume.

### 1. Designate a guardian

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account admin-identity \
  --network testnet \
  --send=yes \
  -- set_guardian --guardian $GUARDIAN_ADDRESS
```

### 2. Guardian trips the circuit breaker

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account guardian-identity \
  --network testnet \
  --send=yes \
  -- emergency_pause --caller $GUARDIAN_ADDRESS
```

Emits `EmergencyPausedEvent`. All mutating operations immediately return
`ContractError::Paused`.

### 3. Admin reviews and clears

After confirming the incident is resolved:

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account admin-identity \
  --network testnet \
  --send=yes \
  -- clear_emergency_pause
```

Emits `EmergencyClearedEvent`. Mutating operations resume only if the normal
pause (`PAUSED_KEY`) is also cleared.

### 4. Removing a guardian

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account admin-identity \
  --network testnet \
  --send=yes \
  -- remove_guardian
```

### Both flags set

If both the normal pause **and** the emergency pause are active, both must be
cleared before writes resume. Clear them in either order; the contract tests
both flags independently in `require_not_paused`.

## Wave Pause Checklist

Use this when freezing writes during an active Wave.

1. Announce freeze window start time, reason, and expected duration in
	contributor channels (dashboard banner, Discord/Telegram, GitHub discussion).
2. Call `set_paused(true)` (or `pause`) from the admin identity.
3. Confirm pause status with `is_paused`.
4. Share contributor-facing impact:
	- `register`, `remove`, `verify`, and other write paths return
	  `ContractError::Paused` (code `7`).
	- Read-only lookups remain available.
5. Keep updates periodic until unpause, including ETA changes.
6. After remediation, call `set_paused(false)` (or `unpause`).
7. Validate recovery: run a known-good `register` and a read call (`get_stats`
	or `get_address`) to confirm normal behavior is restored.
8. Post-incident note: include window duration, impacted functions, and
	follow-up actions.
