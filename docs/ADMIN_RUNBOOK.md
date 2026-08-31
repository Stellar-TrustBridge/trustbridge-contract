# Admin Runbook

The admin role exists for off-chain GitHub verification and operational recovery.

Related docs: [SECURITY](SECURITY.md) · [ABI](ABI.md) · [DEPLOYMENT](DEPLOYMENT.md) · [EVENT_INDEXING](EVENT_INDEXING.md)

## Contents

- [Routine actions](#routine-actions)
- [Stellar Lab & CLI invoke recipes](#stellar-lab--cli-invoke-recipes) — one paste-ready
  `stellar contract invoke` + Lab note for **every admin op**
  - [Conventions & auth gotchas](#conventions--auth-gotchas)
  - [Pause & freeze](#pause--freeze-recipes) · [Guardian](#guardian-recipes) ·
    [Roles](#role-recipes) · [Verification](#verification-recipes) ·
    [Registry maintenance](#registry-maintenance-recipes) ·
    [Upgrade & attestation](#upgrade--attestation-recipes) ·
    [Admin transfer](#admin-transfer-recipes) · [Export](#export-recipes)
- [Storage TTL Maintenance (Keeper)](#storage-ttl-maintenance-keeper)
- [Emergency Pause Lifecycle](#emergency-pause-lifecycle)
- [Recovery notes](#recovery-notes)
- [Guardian Circuit Breaker (Issue #196)](#guardian-circuit-breaker-issue-196)
- [Wave Pause Checklist](#wave-pause-checklist)

## Routine actions

- Verify contributors only after confirming the GitHub identity off-chain.
- Revoke verification cleanly when contributor identities change or a registration is invalidated.
- Export registered records before large dashboard migrations.
- Keep the admin account in a secure wallet or multisig flow.

## Python operator client

The client in `scripts/trustbridge_client.py` provides typed wrappers around
the Stellar CLI for `get_address`, `get_stats`, paginated registry reads, and
batch operations. It uses only Python's standard library, so no package
installation is required beyond Python 3.10+ and the Stellar CLI.

The existing export command delegates to the Python implementation while
keeping its environment-variable interface:

```bash
CONTRACT_ID=C... SOURCE=admin NETWORK=testnet \
  ./scripts/export_registry.sh
```

Equivalent direct invocation:

```bash
PYTHONPATH=scripts python3 scripts/export_registry.py \
  --contract C... --source admin --network testnet \
  --output registry-export-testnet.json
```

The client passes `--send=yes` only for mutating batch methods. CLI failures,
non-JSON responses, pagination stalls, and malformed response shapes stop the
operation with an actionable error instead of being inferred with `grep` or
`jq`. Keep the Stellar CLI identity and network explicit for every operation;
the WASM hash remains independently checked by `make wasm-hash-pin`.

---

## Stellar Lab & CLI invoke recipes

Every admin operation, with a paste-ready `stellar contract invoke` line and the
equivalent [Stellar Lab](https://lab.stellar.org) → **Invoke Contract** note.
Operators hit incidents by hand-assembling XDR in Lab and pasting it broken —
these recipes remove that step.

Signatures here track [`docs/ABI.md`](ABI.md). If the CLI rejects an argument
name, print the live spec for the deployed build and match it exactly:

```bash
stellar contract invoke --id "$CONTRACT_ID" --network "$NETWORK" -- --help
stellar contract invoke --id "$CONTRACT_ID" --network "$NETWORK" -- <fn> --help
```

### Conventions & auth gotchas

| Item | Rule |
|---|---|
| **Env** | `CONTRACT_ID=C…`, `NETWORK=testnet\|futurenet\|mainnet`, `SOURCE=<CLI identity>` (must be **funded** on that network). |
| **Network passphrase** | The CLI derives it from `--network`. In **Lab**, set it explicitly — testnet: `Test SDF Network ; September 2015`; mainnet: `Public Global Stellar Network ; September 2015`; futurenet: `Test SDF Future Network ; October 2022`. A wrong passphrase produces a valid-looking XDR that every validator rejects. |
| **`--source-account` vs `caller` arg** | Two independent things. `--source-account` signs and pays the transaction. Functions like `verify`, `revoke_verification`, `batch_verify`, `batch_remove`, `set_bot_status`, `execute_admin_transfer` **also** take a `caller: Address` argument that the contract checks against the admin / role holder. They must normally be the **same** identity: pass `--caller "$ADMIN"` and sign with the admin identity. |
| **Admin is immutable** | Set once by `initialize`. It is never rotated in place — use the [admin-transfer](#admin-transfer-recipes) flow, which redeploys admin rights atomically. |
| **Role holders** | `verify` / `batch_verify` accept the admin **or** a `Role::Verifier` holder. `revoke_verification` accepts the admin **or** a `Role::Revoker` holder. A `Verifier` cannot revoke; a `Revoker` cannot verify. Everything else is admin-only. |
| **Pause interaction** | Most mutating admin ops return `Paused` (code 7) while the contract is paused. Exceptions that work while paused: `pause`, `unpause`, `set_paused`, `emergency_pause`, `clear_emergency_pause`, `attest_upgrade`, `clear_attestation`, `set_attestation_required`, `upgrade`, `migrate`, and all reads. |
| **Simulate first** | Drop `--send=yes` to simulate only — the CLI prints the result and diagnostic events without submitting. In Lab, use **Simulate** before **Sign & Submit**. Always simulate a role, pause, upgrade, or admin-transfer call before sending. |
| **WASM hash** | `upgrade` / `attest_upgrade` take the **hex** SHA-256 of the `.wasm` (`stellar contract install` prints it; `make wasm-hash-pin` checks it against `wasm-hash.pin`). Not the contract ID, not base64. |
| **No secrets in examples** | Never paste a secret seed into Lab's request body or a shell history. Use a named CLI identity (`stellar keys …`) or a hardware/multisig signer. |
| **Enums / tuples in CLI** | `Role` is passed by variant name: `--role Verifier`. Version tuples are JSON arrays: `--new-version '[1,1,0]'`. `Vec<String>` is a JSON array: `--usernames '["octocat","alice"]'`. |

---

### Pause & freeze recipes

See [Emergency Pause Lifecycle](#emergency-pause-lifecycle) for the full
procedure; these are the bare invoke lines.

#### `pause` — halt all mutations

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- pause
```

- **Auth:** admin only. **Works while paused:** yes (idempotent).
- **Lab:** Invoke Contract → `pause`, no args → Simulate → Sign with admin → Submit.
- Emits `PausedEvent`. Confirm with `-- is_paused` (expect `true`).

#### `unpause` — resume mutations

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- unpause
```

- **Auth:** admin only. Emits `UnpausedEvent`.
- **Gotcha:** if the emergency pause is *also* set, `unpause` alone does not
  resume writes — you must also `clear_emergency_pause`. Check both:
  `-- is_paused` and `-- is_emergency_paused`.

#### `set_paused` — idempotent pause toggle (indexer-friendly)

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- set_paused --paused true      # or --paused false
```

- **Auth:** admin only. Emits `PausedEvent` / `UnpausedEvent` **only on a state
  change** (Issue #197) — a no-op call is silent, which is what makes it safe
  to script.

#### `emergency_pause` — guardian-capable circuit breaker

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account guardian --network "$NETWORK" --send=yes \
  -- emergency_pause --caller "$GUARDIAN_ADDRESS"
```

- **Auth:** admin **or** the designated guardian. `--caller` must match the
  signing identity. Emits `EmergencyPausedEvent`; idempotent.

#### `clear_emergency_pause` — lift the circuit breaker

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- clear_emergency_pause
```

- **Auth:** admin **only** — the guardian deliberately cannot clear it.
  Emits `EmergencyClearedEvent`.

---

### Guardian recipes

#### `set_guardian`

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- set_guardian --guardian "$GUARDIAN_ADDRESS"
```

- **Auth:** admin only. Replaces any existing guardian.
- **Lab:** the `guardian` arg is a `G…` address string; no XDR wrapping needed
  in the Lab form.

#### `remove_guardian`

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- remove_guardian
```

- **Auth:** admin only. After this, only the admin can `emergency_pause`.

---

### Role recipes

`Role` values: `Admin`, `Upgrader`, `Verifier`, `Revoker`. Granting `Admin`
through `set_role` does **not** change `ADMIN_KEY` — it only adds the address to
role checks; use [admin transfer](#admin-transfer-recipes) to move the admin.

#### `set_role` — grant a role

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- set_role --target "$TARGET_ADDRESS" --role Verifier
```

- **Auth:** admin only. Overwrites any existing role for `target`.
- **Lab gotcha:** enter the role as the plain variant name `Verifier` (the Lab
  form renders a dropdown / string field for `contracttype` enums) — not
  `{"Verifier":{}}` and not a number.
- Verify with `-- get_role --address "$TARGET_ADDRESS"` and
  `-- get_role_holders --offset 0 --limit 50`.

#### `remove_role` — revoke a role

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- remove_role --target "$TARGET_ADDRESS"
```

- **Auth:** admin only. No-op if the address holds no role. Compacts the role
  index (later holders shift down one — restart any offset-based page walk).

---

### Verification recipes

#### `verify` — mark one contributor verified

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- verify --caller "$ADMIN" --github-username octocat
```

- **Auth:** admin **or** `Role::Verifier` holder. `--caller` must equal the
  signing identity and hold the right. Confirm GitHub identity off-chain first.

#### `batch_verify` — verify a page of contributors

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- batch_verify --caller "$ADMIN" --usernames '["octocat","alice","bob-smith"]'
```

- **Auth:** admin **or** `Role::Verifier`. Capped at `MAX_WRITE_BATCH` = 25.
- **Partial success:** unknown / already-verified entries are counted as
  `failed` and skipped; the batch does **not** abort. Inspect the returned
  `BatchSummary` — a `success_rate < 100` is informational, not an error.
- For large lists use `scripts/bulk_verify.sh` (handles paging + RPC pacing).

#### `revoke_verification` — withdraw verification

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- revoke_verification --caller "$ADMIN" --github-username octocat --reason-code 1
```

- **Auth:** admin **or** `Role::Revoker` holder (a `Verifier` cannot revoke).
- `reason_code` must be a `RevokeReason` code: `1` IdentityFraud, `2`
  CompromisedKey, `3` Regulatory, `4` DuplicateRegistration, `5` OperatorError,
  `6` GdprErasure, `99` Other. An unknown code fails `InvalidReasonCode`.
- Incident flow: detect → revoke → notify → audit export. Prefer this over
  `remove` when the goal is to stop trust fast without deleting the record.

#### `config_verification` — one-time verification config

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- config_verification --caller "$ADMIN" --attestation github_att --expires-in 3600 --threshold 2
```

- **Auth:** admin only, and callable **once** — a second call fails
  `AlreadyInitialized`. `attestation` is a `Symbol` (≤ 9 chars, `[a-z0-9_]`).

---

### Registry maintenance recipes

#### `batch_remove` — admin-only bulk de-registration

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- batch_remove --caller "$ADMIN" --usernames '["octocat","alice"]'
```

- **Auth:** strictly admin (unlike single `remove`, registrants cannot use it).
  Capped at 25. Returns a `BatchSummary`. Decrements total and verified counters
  for each removed record.

#### `set_bot_status` — flag / unflag a CI account

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- set_bot_status --caller "$ADMIN" --github-username ci-bot --is_bot true
```

- **Auth:** admin only. Dashboards exclude `is_bot == true` records from human
  contributor stats. Note the arg is `--is_bot` (underscore), matching the ABI.

#### `adopt_network_tag` — tag a pre-network-tagging instance

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- adopt_network_tag
```

- **Auth:** admin only. Records `env.ledger().network_id()` (SHA-256 of the
  passphrase) on an instance deployed before Issue #231. Fails
  `NetworkMismatch` if a different tag is already recorded. Read with
  `-- get_network_tag`.

---

### Upgrade & attestation recipes

Full upgrade procedure: [DEPLOYMENT.md § Upgrade Window](DEPLOYMENT.md#upgrade-window-read-only-mode).
Order: `set_paused true` → (optional `attest_upgrade`) → `upgrade` → verify →
`set_paused false`.

#### `set_cooldown` — upgrade timelock

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- set_cooldown --cooldown-seconds 86400      # 0 disables the timelock
```

- **Auth:** admin only. A non-zero cooldown makes `upgrade` fail
  `CooldownActive` until the interval since the last upgrade elapses.

#### `set_attestation_required` — require pre-declared hashes

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- set_attestation_required --required true
```

- **Auth:** admin only. When `true`, `upgrade` without a live matching
  attestation fails `AttestationRequired` (code 20). Default `false`.

#### `attest_upgrade` — declare the next hash in advance

```bash
HASH=$(stellar contract install --wasm target/wasm32v1-none/release/trustbridge_contract.wasm \
  --source-account admin --network "$NETWORK")     # prints the hex hash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- attest_upgrade --wasm-hash "$HASH" --expires-at 1893456000
```

- **Auth:** admin only. `expires_at` is a **Unix timestamp** and must be in the
  future (`AttestationExpired` otherwise). Single-use; publishing a new one
  replaces the old. While live, `upgrade` accepts only this hash.

#### `clear_attestation` — withdraw a pending attestation

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- clear_attestation
```

- **Auth:** admin only. No-op if none pending. Read state with `-- get_attestation`.

#### `upgrade` — swap the executable

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- upgrade --new-wasm-hash "$HASH"
```

- **Auth:** admin only. Fails `CooldownActive`, `UnattestedWasm`,
  `AttestationExpired`, or `AttestationRequired` per the rules above.
  Records a `WasmProvenance` entry (read with `-- get_provenance`), emits
  `UpgradedEvent`. **Simulate first** — a bad hash bricks upgrades until fixed.

#### `migrate` — bump the stored schema version post-upgrade

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- migrate --new-version '[1,1,0]'
```

- **Auth:** admin only. `new_version` must be strictly greater than the current
  (`InvalidVersion` otherwise). Runs any registered migration steps. Confirm
  with `-- get_version`.

---

### Admin transfer recipes

Two-step, time-delayed handover (Issue #195). The current admin stays sole admin
for the whole window.

#### `propose_admin_transfer`

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- propose_admin_transfer --new-admin "$NEW_ADMIN" --delay-seconds 172800
```

- **Auth:** current admin. Re-calling overwrites the pending proposal (fix a
  typo'd address or delay during the window). `new_admin` cannot be the zero
  address.

#### `cancel_admin_transfer`

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" --send=yes \
  -- cancel_admin_transfer
```

- **Auth:** current admin. No-op if nothing pending.

#### `execute_admin_transfer`

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account new-admin --network "$NETWORK" --send=yes \
  -- execute_admin_transfer --caller "$NEW_ADMIN"
```

- **Auth:** must be signed by **the proposed new admin**, and only after the
  delay elapses (`AdminTransferDelayActive` otherwise). Atomically rotates
  `ADMIN_KEY`, drops the old admin's `Role::Admin`, and grants it to the new
  admin. Check the pending proposal any time with `-- get_admin_transfer`.

---

### Export recipes

#### `get_all_registered` — full mapping in one call

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" \
  -- get_all_registered
```

- **Auth:** admin only (read). No `--send` needed. Does not scale — use the
  paginated form or the script below for a large registry.

#### `get_registered_paginated` — cursor export with the `verified` bit

```bash
stellar contract invoke --id "$CONTRACT_ID" --source-account admin --network "$NETWORK" \
  -- get_registered_paginated --cursor 0 --limit 100
```

- **Auth:** admin only (read). `limit` clamps to `MAX_PAGE_LIMIT` = 100. Loop
  until `has_more == false` / `next_cursor == null`.

#### `scripts/export_registry.sh` — assembled JSON snapshot

```bash
CONTRACT_ID="$CONTRACT_ID" SOURCE=admin NETWORK="$NETWORK" ./scripts/export_registry.sh
```

- Pages `get_registered_paginated` and writes a single
  `registry-export-<network>.json`. `SOURCE` must sign as the admin. Take an
  export **before** any bulk verify/remove or dashboard migration.

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

---

## Staged WASM Slot (Issue #300)

The staged WASM slot lets observers see what binary is coming **before** the
upgrade transaction lands.  It is independent of the existing attestation
(`attest_upgrade`) — you may use both together or either alone.

### How it works

| Step | Who | Call |
|------|-----|------|
| Stage the binary | admin or `Role::Upgrader` | `stage_wasm(caller, wasm_hash)` |
| Anyone reads the intent | public (no auth) | `get_staged()` |
| Upgrade (must match if staged) | admin | `upgrade(wasm_hash)` |
| Retract the slot | admin | `clear_staged(caller)` |

When a hash is staged, `upgrade` **rejects** any hash that differs from it
with `StagedWasmMismatch` (code 34).  If nothing is staged, `upgrade` is
unaffected.

### CLI recipes

```bash
# Stage a WASM hash (admin or Upgrader)
stellar contract invoke \
  --id $CONTRACT_ID --source-account admin-identity --network testnet --send=yes \
  -- stage_wasm --caller $ADMIN --wasm-hash <HEX_HASH>

# Public read — no auth needed
stellar contract invoke \
  --id $CONTRACT_ID --network testnet \
  -- get_staged

# Clear the slot
stellar contract invoke \
  --id $CONTRACT_ID --source-account admin-identity --network testnet --send=yes \
  -- clear_staged --caller $ADMIN
```

### Events

| Event | Published by |
|-------|-------------|
| `WasmStagedEvent { wasm_hash, staged_by, timestamp }` | `stage_wasm` |
| `StagedWasmClearedEvent { wasm_hash, cleared_by, timestamp }` | `clear_staged` |

---

## Multisig Upgrade Flow (Issue #301)

Replaces the single-key `upgrade` with an on-chain **propose → approve (×N)
→ execute after delay** governance flow.  The regular `upgrade` path is
unchanged for deployments where `get_upgrade_threshold()` returns `1` (the
default).

### Threshold configuration

```bash
# Require 2-of-N approvals before any upgrade executes
stellar contract invoke \
  --id $CONTRACT_ID --source-account admin-identity --network testnet --send=yes \
  -- set_upgrade_threshold --caller $ADMIN --threshold 2

# Read current threshold (default 1)
stellar contract invoke --id $CONTRACT_ID --network testnet -- get_upgrade_threshold
```

### Full multisig upgrade flow

```bash
# 1. Propose (admin or Upgrader; counts as first approval)
stellar contract invoke \
  --id $CONTRACT_ID --source-account proposer-identity --network testnet --send=yes \
  -- propose_multisig_upgrade \
     --caller $PROPOSER \
     --wasm-hash <HEX_HASH> \
     --delay-secs 86400

# 2. Additional approvers sign (each must be admin or Upgrader, each distinct)
stellar contract invoke \
  --id $CONTRACT_ID --source-account signer2-identity --network testnet --send=yes \
  -- approve_upgrade --caller $SIGNER2 --proposal-id 0

# 3. Read current state
stellar contract invoke --id $CONTRACT_ID --network testnet -- get_upgrade_proposal

# 4. Execute once delay elapsed and approvals >= threshold
stellar contract invoke \
  --id $CONTRACT_ID --source-account admin-identity --network testnet --send=yes \
  -- execute_upgrade --caller $ADMIN --proposal-id 0

# Cancel at any time (admin-only)
stellar contract invoke \
  --id $CONTRACT_ID --source-account admin-identity --network testnet --send=yes \
  -- cancel_upgrade_proposal --caller $ADMIN --proposal-id 0
```

### Error reference

| Error | Code | Meaning |
|-------|------|---------|
| `UpgradeProposalAlreadyPending` | 35 | Cancel or execute the existing proposal first |
| `NoUpgradeProposalPending` | 36 | No proposal to approve/execute/cancel |
| `UpgradeProposalAlreadyApproved` | 37 | This address already approved — use a different key |
| `UpgradeProposalDelayActive` | 38 | Delay has not elapsed; retry later |
| `UpgradeProposalInsufficientApprovals` | 39 | Gather more approvals before executing |
| `StagedWasmMismatch` | 34 | Proposal hash conflicts with staged slot |

### Events

| Event | Published by |
|-------|-------------|
| `UpgradeProposedEvent { proposal_id, wasm_hash, proposed_by, executable_at, timestamp }` | `propose_multisig_upgrade` |
| `UpgradeApprovedEvent { proposal_id, approved_by, approval_count, timestamp }` | `approve_upgrade` |
| `UpgradeProposalExecutedEvent { proposal_id, wasm_hash, executed_by, approval_count, timestamp }` | `execute_upgrade` |
| `UpgradeProposalCancelledEvent { proposal_id, wasm_hash, cancelled_by, timestamp }` | `cancel_upgrade_proposal` |
| `UpgradedEvent { … }` | `execute_upgrade` (same as regular `upgrade`) |

### Scope note

The multisig flow covers **upgrade proposals only**.  Other admin functions
(`set_role`, `pause`, `verify`, etc.) remain single-admin.  Use a Stellar
multisig account as the admin address to add multi-party control over all
admin actions at the account layer rather than the contract layer.
