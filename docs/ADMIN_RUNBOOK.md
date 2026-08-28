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

---

## Role Expiry (Issue #221)

Off-boarded contractor and bot keys should not linger as live `Verifier` /
`Revoker` holders forever. `set_role` still grants a role with no expiry
(unchanged); `set_role_with_expiry` grants a time-bounded one.

```bash
# Grant a contractor Verifier access that lapses in 30 days
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account admin-identity \
  --network testnet \
  --send=yes \
  -- set_role_with_expiry --target $CONTRACTOR --role 3 --expires_at $(($(date +%s) + 2592000))
```

- Check what is set: `get_role_expiry --address $CONTRACTOR` — `None` means
  no expiry (or no role at all); `Some(T)` may already be in the past.
- Expiry is **lazy**. The role entry is not deleted when the clock passes
  `expires_at` — `get_role` (and every check built on it) just stops
  reporting it. Run `remove_role` afterward if you want the storage entry
  itself gone and the address dropped from `get_role_holders`.
- The contract admin's own identity is never affected by this. Only the
  RBAC-style `Role::Admin` grant (the one `initialize` sets alongside
  `ADMIN_KEY`) can expire, and only if you explicitly grant it with
  `set_role_with_expiry` — routine admin auth (`has_admin_role`,
  `get_admin`) reads a separate, immutable storage slot.
- Operational habit: when off-boarding a contractor or rotating a bot key,
  prefer `set_role_with_expiry` with a known end date over remembering to
  call `remove_role` manually later.

---

## Time-Bounded Verification (Issue #218)

A stolen or transferred GitHub account should not stay `verified` forever.
`config_verification`'s `expires_in` (seconds, one-time set) now drives an
actual expiry, checked against each username's last `verify()` timestamp.

- **Check effective status**: `is_verification_active --github-username
  octocat` — this is expiry-aware. `get_address --github-username octocat`
  returns the raw `ContributorRecord.verified` flag, which stays `true`
  after expiry until someone calls `revoke_verification` (lazy expiry — see
  [ABI.md](./ABI.md#time-bounded-verification-issue-218)). Payout and
  dashboard integrations should call `is_verification_active`, not read the
  raw flag, if `config_verification` has been used.
- **Renewal is automatic on re-verify**: calling `verify` again on a
  username whose prior verification has expired succeeds and refreshes the
  timestamp — it does not require an explicit `revoke_verification` first.
  A still-active verification is unaffected and still rejects a duplicate
  `verify` call with `AlreadyVerified`.
- **`get_stats` / `get_verified_count` are not expiry-aware** by design —
  they count the raw `verified` flag, not `is_verification_active`, so they
  stay an O(1) read. An operator watching for stale-but-unrevoked
  verifications should page through the registry and call
  `is_verification_active` per record, or track `get_verification_expiry`
  off-chain.
- If `config_verification` was never called, or was called with
  `expires_in = 0`, nothing in this section applies — verification behaves
  exactly as before this issue.

---

## Signed Export Attestation (Issue #223)

`export_registry.sh` produces a JSON page with no on-chain binding — a
compromised or careless export step could hand an auditor stale or edited
data with no way to detect it. `export_attestation(cursor, limit)` is the
admin-only companion read that binds the same page to a digest.

### Workflow for an air-gapped audit

1. **Online, admin-authenticated machine:** call `export_attestation` for
   each page (same `cursor`/`limit` loop as `export_registry.sh` already
   does against `get_registered_paginated`) and save both the page JSON and
   the returned `digest` / `version` / `ledger`.

   ```bash
   stellar contract invoke \
     --id $CONTRACT_ID \
     --source-account admin-identity \
     --network testnet \
     -- export_attestation --cursor 0 --limit 100
   ```

2. **Transfer** the saved output to the air-gapped machine by any channel —
   USB, printed QR, whatever the audit's threat model requires. There is
   nothing further to fetch from the network.

3. **Offline, on the air-gapped machine:** recompute the SHA-256 digest over
   the page's XDR encoding exactly as `export_attestation` does (see
   `storage::build_export_digest` in `src/storage.rs` for the reference
   implementation) and compare byte for byte against the saved `digest`. A
   mismatch means the JSON the auditor is holding does not match what the
   contract actually returned for that `cursor`/`limit` at `ledger`.

### What this is not

- **Not a threshold signature.** The digest is bound to the admin's
  authenticated read, not counter-signed by a quorum. Out of scope for
  Issue #223 by design.
- **Not a Merkle proof over the whole registry.** Each attestation covers
  one page; there is no root committing to every page at once. A
  registry-wide Merkle root can complement this later (Issue #27) without
  changing this function's shape.
- **Does not weaken the existing admin gate.** `export_attestation`,
  `get_registered_paginated`, and `get_all_registered` all still require
  admin auth — this issue only adds a binding on top of that export, it
  does not add a new unauthenticated way to read the registry.
- **An empty registry is not an error.** `page.records` is empty and
  `digest` is still a real, deterministic hash over that empty page —
  useful as a baseline attestation for a freshly deployed instance.

---

## Dual-Control `batch_remove` (Issue #219)

A single admin signature could previously delete up to `MAX_WRITE_BATCH`
(25) registrations in one call. Above a configurable size threshold, that is
no longer enough — the batch must be proposed by one admin-equivalent
address and executed by a **different** one.

### 1. Configure the threshold

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account admin-identity \
  --network testnet \
  --send=yes \
  -- set_batch_remove_threshold --threshold 10
```

`0` (the default) disables dual control entirely — every `batch_remove` call
executes directly regardless of size, identical to pre-#219 behavior. With a
threshold set, any batch **larger** than it (strictly greater; a batch
exactly at the threshold is unaffected) is rejected by `batch_remove` itself
with `DualControlRequired` and must go through the propose/execute flow
below.

### 2. Provision a second key *before* you need it

`execute_batch_remove` requires a caller that is the contract admin or holds
`Role::Admin`, and is a **different** address than whoever proposed the
batch. If the only admin-equivalent address is the contract admin itself,
a large batch can be proposed but never executed — grant `Role::Admin` to a
second, independently held key ahead of time:

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account admin-identity \
  --network testnet \
  --send=yes \
  -- set_role --target $SECOND_KEY --role 1
```

This is a deliberate design choice, not a bug: dual control that degraded
back to single-key execution whenever the second key was missing would not
be dual control. `cancel_batch_remove` remains available to abort a stuck
proposal — see [SECURITY.md](./SECURITY.md#dual-control-batch_remove-issue-219)
for the full threat-model note.

### 3. Propose, then execute from a different key

```bash
# Admin proposes
stellar contract invoke \
  --id $CONTRACT_ID --source-account admin-identity --network testnet --send=yes \
  -- propose_batch_remove --caller $ADMIN --usernames '["squatter1","squatter2", ...]'

# A DIFFERENT Role::Admin holder executes
stellar contract invoke \
  --id $CONTRACT_ID --source-account second-key-identity --network testnet --send=yes \
  -- execute_batch_remove --caller $SECOND_KEY
```

`get_pending_batch_remove` shows what is queued (works while paused). A
proposal not executed within 24 hours (`BATCH_REMOVE_PROPOSAL_TTL_SECS`) is
treated as gone the next time anyone calls `execute_batch_remove` — propose
again if you still need it removed.

### 4. Abort instead

```bash
stellar contract invoke \
  --id $CONTRACT_ID --source-account admin-identity --network testnet --send=yes \
  -- cancel_batch_remove --caller $ADMIN
```

Available even while paused, so a stuck or mistaken proposal is never
trapped behind the same freeze that might be the reason to cancel it.
