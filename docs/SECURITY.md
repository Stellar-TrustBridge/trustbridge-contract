# Security

Security considerations for **trustbridge-contract**.

Related docs: [README](../README.md) · [ARCHITECTURE](ARCHITECTURE.md) · [DEPLOYMENT](DEPLOYMENT.md) · [ADMIN_RUNBOOK](ADMIN_RUNBOOK.md)

---

## Threat Model

### In Scope

| Threat | Mitigation |
|--------|------------|
| Impersonation (registering someone else's GitHub username) | `stellar_address.require_auth()` — only the address owner can register |
| Unauthorized removal | `caller` must auth as registrant or admin |
| Unauthorized admin-only actions | `admin.require_auth()` on `get_all_registered`, `get_registered_paginated`, `set_role`, `pause`/`unpause` |
| Unauthorized verification actions | `caller.require_auth()` on `verify` / `revoke_verification`; `caller` must be the admin or hold `Role::Verifier` (checked via `has_role_or_admin`) — any other caller gets `NotAuthorized` |
| Double initialization | `AlreadyInitialized` error |
| Admin storage mutated after init | No public setter writes `ADMIN_KEY` — only `initialize` does, gated by `AlreadyInitialized` (Issue #97) |
| Malformed or oversized username input | `InvalidUsername` error, checked before auth and before any write |
| Unicode / homoglyph username spoofing | Byte-wise ASCII validation rejects all non-ASCII bytes; see **Unicode Rejection Policy** section |
| Consecutive-hyphen username bypass | `InvalidUsername` error — consecutive hyphens now enforced on-chain |
| Counter drift from rejected calls | Invariant property fuzzing, see [REGISTRY_INVARIANTS](REGISTRY_INVARIANTS.md) |
| Stale trust surviving a remove → re-register cycle (a new registrant inheriting the previous owner's verified status or address binding) | `remove` unconditionally clears the stored record; `register` on a removed username always starts a fresh, unverified record — see [Re-registration After Remove](#re-registration-after-remove) (Issue #93) |
| Compromised or unpinned RPC client dependency | Crate validation checklist below |
| Compromised Verifier key silently revoking payout eligibility | `Role::Verifier` and `Role::Revoker` are distinct — Issue #212 |
| Admin force-removing a name without a grace period | Challenge flow enforces on-chain `resolve_after` delay — Issue #214 |

### Out of Scope (handled off-chain)

| Concern | Responsibility |
|---------|----------------|
| GitHub identity proof | Admin verification workflow + TrustBridge dashboard |
| Username squatting policy | Social/process layer; contract allows first-come registration |
| Admin key compromise | Operational security; use multisig for admin address |
| GitHub username changes | Off-chain mapping updates; may require re-registration |

---

## Admin Key Management

The admin address is **immutable** after `initialize` unless rotated via the
two-step admin transfer flow introduced in Issue #195.

### Two-step admin rotation (Issue #195)

Admin rotation is a **propose → delay → accept** flow. There is never a window
with two live admins:

1. **Propose** — the current admin calls `propose_admin_transfer(new_admin, delay_seconds)`.
   The proposal is written to chain and `AdminTransferProposedEvent` is emitted.
   During the delay the current admin remains the only admin.
2. **Observe / cancel** — watchers have `delay_seconds` to detect and respond.
   The current admin may call `cancel_admin_transfer()` at any point before
   execution to abort the proposal (emits `AdminTransferCancelledEvent`).
   A second `propose_admin_transfer` call overwrites the first — useful for
   correcting a typo during the window.
3. **Accept** — after the delay elapses, the **proposed new admin** calls
   `execute_admin_transfer(caller)` and signs. `ADMIN_KEY` is atomically
   rotated: the old admin's `Role::Admin` entry is removed and the new admin
   receives it. `AdminTransferExecutedEvent` is emitted.

Threat-model properties:
- A compromised admin key can only propose a transfer, not execute it — the
  candidate address must also sign `execute_admin_transfer`.
- Self-transfer is not explicitly blocked (the admin may rotate to themselves
  with a delay), but produces no effective change.
- The zero/burn address is rejected at proposal time (`ZeroAddress`).
- Pause during a pending proposal does not affect the proposal storage — the
  delay timer continues. However, `execute_admin_transfer` checks
  `require_not_paused`, so execution is blocked while paused.
- `get_admin_transfer()` exposes the pending proposal for monitoring.

### Legacy note (pre-#195)

Before Issue #195, `ADMIN_KEY` was set once in `initialize` with no
transfer API. Rotation meant redeploying a new instance. The immutable-admin
design and its regression tests are preserved: `test_double_initialize_rejected_after_successful_init`,
`test_issue_97_second_initialize_rejected_with_different_admin`, and
`test_issue_97_admin_unchanged_across_unrelated_operations` continue to pass.

Recommendations:

- Use a **multisig** or **smart account** as the admin G-address
- Set a meaningful `delay_seconds` (e.g. 86400 = 24 h) to give watchers time
  to detect unexpected proposals
- Never commit private keys or seed phrases
- Monitor `AdminTransferProposedEvent` in your indexer

### Two-step WASM upgrade (Issue #198)

The attestation flow (`attest_upgrade` → `upgrade`) is optionally enforceable
on-chain via `set_attestation_required(true)`.

| `attestation_required` | No attestation published | Attestation matches | Expired / mismatch |
|------------------------|--------------------------|--------------------|--------------------|
| `false` (default) | Upgrade proceeds (unattested) | Upgrade proceeds (attested) | `AttestationExpired` / `UnattestedWasm` |
| `true` | `AttestationRequired` (code 20) | Upgrade proceeds (attested) | `AttestationExpired` / `UnattestedWasm` |

Setting `required = true` means:
- A hot admin key cannot swap the WASM binary in a single step — it must first publish the hash via `attest_upgrade`, wait for watchers, then call `upgrade` with the same hash.
- Clearing attestation (`clear_attestation`) and then calling `upgrade` fails with `AttestationRequired`.
- The cooldown still applies independently.

The `is_attestation_required()` read exposes the current config for monitoring and client-side enforcement.

**Threat scenarios:**
- *Compromised admin key attempts silent upgrade*: `upgrade` fails with `AttestationRequired` unless an attestation for that exact hash was published first (observable on-chain by watchers).
- *Attacker publishes a forged attestation*: They still need admin auth on `attest_upgrade`, so a compromise of the admin key is the prerequisite — the same threat as before, but now visible before the swap.
- *Attestation expired before upgrade*: `AttestationExpired` is returned; the stale record is cleared. The admin must re-attest with a new `expires_at`.



When pause mode is active, guarded entry points fail with
`ContractError::Paused` (code `7`). This avoids partial-wave behavior where
some state writes continue while others are frozen.

Paused function matrix:

| Function | Behavior while paused |
|---------|------------------------|
| `register` | Rejected with `Paused` |
| `remove` | Rejected with `Paused` |
| `verify` | Rejected with `Paused` |
| `revoke_verification` | Rejected with `Paused` |
| `upgrade` | Rejected with `Paused` |
| `migrate` | Rejected with `Paused` |
| `set_role` / `remove_role` | Rejected with `Paused` |
| `get_public_paginated` | Rejected with `Paused` |

Allowed while paused:

| Function | Behavior while paused |
|---------|------------------------|
| `get_address`, `has_record`, `get_stats` | Allowed read-only lookups |
| `get_all_registered`, `get_registered_page`, `get_registered_paginated` | Allowed for admin/export workflows |
| `is_paused`, `is_contract_paused`, `version`, `is_compatible` | Allowed status and compatibility reads |
| `pause`, `unpause`, `set_paused` | Allowed admin controls for freeze lifecycle |

---

## Registration Integrity

- Registering a username requires the Stellar address owner to sign
- Re-registration with a new address resets verification status
- There is no on-chain proof of GitHub ownership at registration time — verification is a separate admin step
- Wave #49 locks the address-update invariant: after a verified username is
  re-registered to a different Stellar address, the record becomes unverified,
  the verified count decreases, and any later `verify()` applies to the new
  address only.

### Registration cooldown enforcement

The contract enforces per-username action rate-limiting during `register()`:

- When `cooldown` is non-zero, `register()` checks whether `is_in_cooldown()` is true for `github_username`.
- If the configured cooldown window has not elapsed since the username's last mutating action, `register()` fails with `CooldownActive` (code 8).
- Upon a successful `register()`, the username's last action timestamp is updated via `set_last_action()`.
- First-time registrations have no recorded prior action timestamp (0), allowing initial registration to succeed immediately.

---

## Username Squatting Mitigations

Because Soroban handles GitHub registrations permissionlessly (first-come, first-registered), there is a risk of username squatting (someone registering another contributor's GitHub username to redirect their rewards). TrustBridge uses a multi-layered security model to mitigate this risk.

### 1. Mandatory Admin Verification Gate
Registration alone does **not** grant payout readiness. Payout systems and the TrustBridge dashboard require a contributor record to be **verified** before rewards can be disbursed.
- Verification is performed by the contract admin or a designated verifier after confirming ownership of the GitHub account off-chain (e.g., via OAuth or a cryptographic proof).
- The verifier validates that the registered Stellar address matches the authenticated GitHub user.
- If a squatter registers a name, they cannot pass this verification gate since they cannot prove ownership of the corresponding GitHub account.

### 2. Double-Auth Transfer Protection (Self-Auth)
If a user registers a username and later needs to transfer it to a different Stellar address, the contract requires **both** of the following to authorize the transaction:
1. The new Stellar address.
2. The currently registered Stellar address.
This prevents a third party from maliciously taking over a registered username.

### 3. Contributor Dispute & Resolution Flow
If a rightful owner discovers that their GitHub username has been squatted on-chain:
1. **Report**: The owner reports the dispute to the TrustBridge administrators (off-chain).
2. **Revocation/Removal**: The admin verifies the owner's identity, then calls `remove` to delete the squatter's record from the contract registry.
3. **Re-registration**: The rightful owner registers their correct Stellar address.
4. **Re-verification**: The admin verifies the new record.

### FAQ: "Someone registered my GitHub name, what should I do?"
- **Will they receive my payouts?** No. Payouts require the record to be verified. The squatter cannot pass the admin verification check.
- **How do I reclaim my username?** Open a support ticket / dispute with the TrustBridge administrators. They will remove the squatter's record so you can register your address.
- **Does the contract verify my GitHub handle automatically?** No. There is **no on-chain verification proof** of GitHub identity at registration time. Verification is entirely off-chain/administrative.

---

## Input Validation

`register` validates the username **before** `require_auth()` and before any
storage write. The order matters: a malformed call is rejected at the cheapest
point, no signature is spent on it, and no counter or index entry moves.

| Rule | Value |
|------|-------|
| Length | 1 to 39 characters (GitHub's own cap) |
| Allowed characters | `a-z`, `A-Z`, `0-9`, `-`, `_` (ASCII only) |
| First and last character | Must be alphanumeric |
| Consecutive hyphens | Not allowed (`foo--bar` is rejected) |
| Unicode / non-ASCII | Rejected — see **Unicode Rejection Policy** below |

Rejection returns `InvalidUsername` (code 7).

Validation lives in `src/utils.rs` and works entirely on a fixed 64-byte stack
buffer. The contract is `#![no_std]`, so the validation path never allocates
and the copy length is bounded before the copy happens.

Deliberate non-goals:

- **Underscores are accepted** even though GitHub disallows them, so any
  registration made before validation existed stays readable and removable.
  Tightening this later would strand those records.
- **Case is not normalized on-chain.** `Alice` and `alice` are distinct keys.
  Off-chain workflows should match with `eq_ignore_ascii_case` from
  `src/utils.rs` when comparing a registration against a GitHub identity.
- **No on-chain proof the username exists on GitHub.** Validation checks shape,
  not ownership. Ownership remains the admin verification step.

---

## Unicode Rejection Policy

**GitHub usernames are ASCII-only.** Any username containing a non-ASCII byte —
including multi-byte UTF-8 sequences for accented letters (é, ü, ñ), emoji,
CJK characters, or Cyrillic/Arabic/Hebrew script — is rejected with
`InvalidUsername`.

### Why this matters

Unicode homoglyph attacks are a recognized impersonation vector. An attacker
registers a username that **looks** visually identical to a legitimate user's
name but uses different Unicode codepoints:

- Cyrillic 'а' (U+0430) looks like ASCII 'a' (U+0061)
- Greek 'ο' (U+03BF) looks like ASCII 'o' (U+006F)
- Cyrillic 'с' (U+0441) looks like ASCII 'c' (U+0063)

A username like `аlice` (Cyrillic 'а' + ASCII 'lice') appears indistinguishable
from `alice` in most fonts, but encodes as `[0xD0, 0xB0, 0x6C, 0x69, 0x63, 0x65]`
instead of `[0x61, 0x6C, 0x69, 0x63, 0x65]`. Without byte-level validation,
this becomes a credential spoofing attack.

### How the check works

Validation is byte-wise, not glyph-wise:

1. Every username is copied into a fixed stack buffer (64 bytes).
2. Every byte is checked with `.is_ascii()` (returns false for bytes > 0x7F).
3. Any multi-byte UTF-8 sequence has a leading byte ≥ 0x80, which fails the
   ASCII check and is immediately rejected.

This makes the homoglyph attack impossible: even if the rendered glyphs look
identical, the byte sequences differ and only the ASCII form is accepted.

### Covered cases

The following are all rejected (see comprehensive tests in `src/utils.rs`):

| Category | Example | Codepoint | UTF-8 Encoding |
|----------|---------|-----------|----------------|
| Latin-extended | `café` | U+00E9 é | `[0xC3, 0xA9]` |
| Emoji | `user😀` | U+1F600 | `[0xF0, 0x9F, 0x98, 0x80]` |
| CJK (Chinese/Japanese/Korean) | `中user` | U+4E2D 中 | `[0xE4, 0xB8, 0xAD]` |
| Arabic | `مuser` | U+0645 م | `[0xD9, 0x85]` |
| Hebrew | `אuser` | U+05D0 א | `[0xD7, 0x90]` |
| Cyrillic homoglyph | `аlice` | U+0430 а | `[0xD0, 0xB0]` |
| Greek homoglyph | `bοb` | U+03BF ο | `[0xCF, 0xBF]` |

### Test coverage

`src/utils.rs` includes a dedicated test suite for the Unicode rejection policy
(Wave #69 / Issue #70):

- `test_unicode_latin_extended_rejected`
- `test_unicode_emoji_rejected`
- `test_unicode_cjk_rejected`
- `test_unicode_arabic_and_rtl_rejected`
- `test_unicode_homoglyph_attack_rejected`
- `test_unicode_all_non_ascii_rejected`
- `test_unicode_embedded_at_any_position_rejected`
- `test_raw_high_byte_rejected`
- `test_valid_ascii_still_accepted_after_unicode_hardening`

These tests confirm that every form of non-ASCII input — whether a visually
distinct character like an emoji or a deceptive homoglyph like Cyrillic 'а' —
is caught and rejected, while every valid ASCII username shape remains accepted.

### Performance

The check adds no allocations and no UTF-8 decoding overhead. It is a
per-byte scan over a stack buffer, the same cost profile as the existing
alphanumeric and hyphen checks.

### Future considerations

- Off-chain tooling (dashboard, indexers) should **canonicalize and validate**
  usernames against the GitHub API before submitting them for registration.
  The on-chain check is a last line of defense, not a substitute for
  pre-submission validation.
- If GitHub's own username policy changes (e.g. to allow certain Unicode
  ranges), this validation will need to be relaxed via a contract upgrade and
  a corresponding audit of the new attack surface.

---

---

## Validating the Rust RPC Client Crate

Every off-chain component that talks to this contract, including the deploy
scripts, the dashboard sync job, and any indexer, reaches the network through an
RPC client crate. That crate sits between operator keys and the network, so it
is in the trust boundary and gets reviewed like contract code.

### Before adding or bumping an RPC client dependency

| Check | How |
|-------|-----|
| Version is pinned exactly | `soroban-client = "=x.y.z"` in `Cargo.toml`, `Cargo.lock` committed for binaries |
| No known advisories | `cargo audit` and `cargo deny check advisories` |
| License is acceptable | `cargo deny check licenses` |
| No unexpected transitive additions | `cargo tree --duplicates` and review the lockfile diff |
| Source is the official crate | Confirm the repository field points at the upstream Stellar org, not a fork |
| Registry integrity | `cargo verify-project`; do not use `[patch]` or git dependencies for release builds |
| Maintenance signal | Recent releases, open advisories, and responsiveness on upstream issues |

A dependency bump that changes the transitive graph needs the lockfile diff in
the PR. Reviewers should be able to see every crate that was added.

### Runtime expectations for any RPC client

- **TLS enforced.** Reject plain `http://` RPC URLs outside of local development.
- **No secret logging.** Secret keys, seed phrases, and signed transaction
  envelopes must never reach logs, error strings, or telemetry.
- **Bounded retries.** Retry with exponential backoff and a hard attempt cap, so
  an outage degrades instead of turning into a self-inflicted flood.
- **Explicit timeouts.** A client with no timeout turns an RPC stall into a hung
  deploy job holding an operator key in memory.
- **Response validation.** Treat RPC responses as untrusted input: check the
  contract ID, network passphrase, and ledger sequence before acting on them.
- **Simulation before submission.** Simulate state-changing calls first so a
  malformed username or an auth failure surfaces without spending fees.

---

## Operational Failure Modes

| Failure | Expected behavior | Operator action |
|---------|-------------------|-----------------|
| Horizon or RPC outage | Client retries with backoff, then fails loudly. Contract state is unaffected: nothing was submitted. | Fail the job, alert, retry later. Never fall back to an unverified RPC endpoint. |
| RPC rate limiting (HTTP 429) | Backoff honors `Retry-After` where present. | Reduce poll frequency, batch reads, or move to a dedicated RPC provider. |
| Invalid env configuration | `scripts/deploy.sh` refuses to run without `ADMIN`. Every `invoke-*` and `bindings` Makefile target refuses to run without `CONTRACT_ID`, and `invoke-init` also requires `ADMIN`. | Fix the value rather than exporting a placeholder. `NETWORK` defaults to `testnet`, so a mainnet job must state `NETWORK=mainnet` explicitly. |
| Auth or permission failure | `require_auth()` panics the invocation and the whole transaction rolls back. Admin-only calls by a non-admin return `NotAuthorized`. | Confirm the signing key matches the registrant or the admin address. |
| Partial write during failure | Not possible. Soroban transactions are atomic, and validation runs before the first write. | None. |
| 100+ contributor scale | `get_all_registered` is a linear full-index scan and grows with registry size. | Prefer event indexing (see [EVENT_INDEXING.md](EVENT_INDEXING.md)) over repeated full exports. Watch the export benchmark in [ABI.md](ABI.md#cost-and-benchmarks) for regressions. |

## Register Budget Guard

Contributor onboarding depends on `register`, so fee spikes or budget
exhaustion are treated as availability risks.

Budget thresholds (current defaults in `Makefile`):

- CPU instructions: `25_000_000` max
- Memory bytes: `300_000` max

The guard measures two inputs:

1. Baseline username (`octocat`)
2. Stressed username (maximum allowed username length)

Run locally:

```bash
make bench-register-budget
```

Override thresholds when updating the baseline:

```bash
make bench-register-budget REGISTER_BUDGET_CPU_MAX=26000000 REGISTER_BUDGET_MEM_MAX=320000
```

Failure output identifies which sample exceeded the budget using
`input=baseline` or `input=max_username_len`.

### Remediation when budget is exceeded

1. Reduce writes in `register` (avoid unnecessary index/counter touches).
2. Keep username handling bounded (`MAX_USERNAME_LEN`) and avoid extra string copying.
3. Re-run `make bench-register-budget` and compare against prior output before
  raising thresholds.
4. If threshold changes are unavoidable, document rationale in PR notes and
  update deployment/operator docs accordingly.

### Environment configuration

Copy `.env.example` and fill every value explicitly. Configuration rules:

- No implicit network default in production scripts. `NETWORK` must be stated.
- `ADMIN` is required for mainnet deploys and is not inferred from the local
  keystore.
- Never commit `.env`. Only `.env.example` is tracked.

---

## Index-Length Invariant

The registry maintains two parallel state values that must always agree:

| State | Storage key | Updated by |
|-------|-------------|------------|
| `COUNT_KEY` — registration counter (`u32`) | instance storage | `register` (increment), `remove` (decrement) |
| `INDEX_KEY` — ordered username vec (`Vec<String>`) | instance storage | `add_to_index` (append), `remove_from_index` (filter) |

**Invariant:** `get_count(env) == get_index(env).len()` at every quiescent point between transactions.

### Why this matters

Both values are read by different callers for different purposes:

- Paginated export endpoints (`get_registered_page`, `get_registered_paginated`, `get_public_paginated`) walk `INDEX_KEY` for the actual usernames but expose `COUNT_KEY` as the `total` field of the response. If they diverge, a client that uses `total` to compute page counts will request the wrong number of pages.
- `get_stats` returns `COUNT_KEY` directly. Monitoring and dashboard tooling that reads `get_stats` to show a contributor count will display a wrong number if the counter has drifted.
- An index longer than the counter indicates **phantom entries** — the index holds usernames that the contract believes do not exist. An index shorter than the counter indicates **invisible entries** — the counter says more contributors exist than are reachable by any export. Both are security-relevant for an audit.

### How the invariant is maintained

`register` and `remove` always update both values in the same transaction:

```
register (new username):
    set_count(get_count + 1)
    add_to_index(username)        ← appends to INDEX_KEY

remove:
    remove_record(username)
    remove_from_index(username)   ← filters INDEX_KEY
    set_count(get_count - 1)
```

Soroban transactions are atomic, so a partial write that updates one side but not the other cannot leave the invariant broken at rest — either both updates land or neither does.

### Test coverage (Issue #59 / Wave #60)

`tests/integration.rs` includes a dedicated invariant test suite:

| Test | What it checks |
|------|----------------|
| `test_index_invariant_holds_on_empty_registry` | Invariant holds at genesis (count=0, index.len()=0) |
| `test_index_invariant_holds_after_single_register` | Invariant holds after the first registration |
| `test_index_invariant_holds_after_register_and_remove` | Invariant holds after removing first, middle, and last entries |
| `test_index_invariant_holds_after_same_address_reregister` | Re-register to same address does not double-increment counter |
| `test_index_invariant_holds_after_address_change_reregister` | Re-register to different address does not alter total |
| `test_index_invariant_holds_at_scale` | Register 10, remove 5 interleaved — check after each removal |
| `test_index_invariant_unchanged_on_failed_remove` | **Failure path**: `remove` on unknown username returns `NotRegistered` and does not mutate state |
| `test_index_invariant_unchanged_on_invalid_register` | **Failure path**: invalid username returns `InvalidUsername` and does not mutate state |
| `test_index_invariant_holds_after_remove_then_reregister` | Remove then re-register restores count=1, index.len()=1 |
| `test_index_invariant_unchanged_by_pause_unpause` | Pause/unpause does not touch count or index |

The helper `storage::index_length_invariant_holds(env)` encodes `get_count == get_index().len()` in one place so every test asserts the same invariant without repeating the definition inline.

### Edge cases

- **Removal of a non-existent username** returns `NotRegistered` before any write, so count and index are never touched on a failed remove.
- **Invalid username on register** is caught before `require_auth` and before any write, so count and index are never touched on a rejected registration.
- **Re-registration** (same username, same or different address) follows the `existing.is_some()` branch in `register`, which does not call `add_to_index` or increment the counter, preserving the invariant.
- **100+ contributors**: both `COUNT_KEY` and `INDEX_KEY` live in instance storage. At very large registry sizes the `get_all_registered` export hits the 100-ledger-entry footprint limit; use paginated endpoints instead, but the invariant is unaffected by which export endpoint is used.

---

## Storage TTL

Persistent entries on Stellar mainnet have a **time-to-live (TTL)**. If entries expire, data may become unavailable until extended.

Operational teams should:

1. Monitor entry TTL via RPC
2. Run periodic TTL extension via Stellar CLI (`stellar contract extend`)
3. Document extension cadence in deployment runbooks

---

## Batch Remove Semantics

The `batch_remove` function provides an efficient way to clean up multiple registrations in one transaction. It introduces specific security considerations:

### 1. Admin-Only Auth
Unlike single `remove`, which allows a registrant to self-remove, `batch_remove` is strictly admin-only. A registrant attempting to remove their own record via `batch_remove` will receive `NotAuthorized`. The `batch_remove` surface is for administrative cleanup, not self-service.

### 2. Partial Failure by Design
A batch call does not revert if a single username fails to be removed (e.g., if it was already removed or was never registered). Instead, the failure is tallied in the returned `BatchSummary`, and the transaction continues. This ensures that one disputed or stale record does not grief an entire cleanup batch. The transaction only aborts if the caller lacks authorization, the contract is paused, or the batch size limit is exceeded.

### 3. Griefing Size Cap
To prevent a malicious or erroneous caller from exhausting the network CPU/memory budget in a single transaction (and causing an out-of-gas panic that masks partial success), `batch_remove` enforces a strict maximum batch size (configured via `BatchConfig`). Submitting a list of usernames larger than this cap immediately reverts the transaction with `InvalidBatchSize`.

---

## Verify and Revoke_Verification Auth Negative Matrix

Dashboard operators and auditors need the full failure surface of `verify` and `revoke_verification` spelled out.
The matrix below covers every unauthorized and invalid state transition.  Each cell maps to an automated
unit test in `src/lib.rs` (search for `#114`).

Cross-reference: [remove auth negative matrix](#remove-auth-negative-matrix) (Issue #113) · [ABI reference](ABI.md#verifycaller-address-github_username-string---resultcontracterror)

### `verify` — negative matrix

| # | Scenario | Expected error | Code | Test |
|---|----------|---------------|------|------|
| V1 | Contract not yet initialized | `NotInitialized` | 2 | `test_verify_negative_not_initialized` |
| V2 | Username not registered | `NotRegistered` | 4 | `test_verify_negative_username_not_registered` |
| V3 | Username already verified (double-verify) | `AlreadyVerified` | 5 | `test_verify_negative_already_verified` |
| V4 | Caller has no role | `NotAuthorized` | 3 | `test_verify_negative_no_role_caller` |
| V5 | `Role::Upgrader` holder | `NotAuthorized` | 3 | `test_verify_negative_upgrader_cannot_verify` |
| V6 | **Admin caller** _(happy path)_ | `Ok(())` | — | `test_verify_positive_admin_can_verify` |
| V7 | **`Role::Verifier` holder** _(happy path)_ | `Ok(())` | — | `test_verify_positive_verifier_role_can_verify` |
| V8 | Contract is paused | `Paused` | 7 | `test_verify_negative_paused` |

### `revoke_verification` — negative matrix

| # | Scenario | Expected error | Code | Test |
|---|----------|---------------|------|------|
| R1 | Contract not yet initialized | `NotInitialized` | 2 | `test_revoke_negative_not_initialized` |
| R2 | Username not registered | `NotRegistered` | 4 | `test_revoke_negative_username_not_registered` |
| R3 | Record not yet verified | `NotVerified` | 6 | `test_revoke_negative_not_verified` |
| R4 | Caller has no role | `NotAuthorized` | 3 | `test_revoke_negative_no_role_caller` |
| R5 | `Role::Upgrader` holder | `NotAuthorized` | 3 | `test_revoke_negative_upgrader_cannot_revoke` |
| R6 | **Admin caller** _(happy path)_ | `Ok(())` | — | `test_revoke_positive_admin_can_revoke` |
| R7 | **`Role::Verifier` holder** _(happy path)_ | `Ok(())` | — | `test_revoke_positive_verifier_role_can_revoke` |
| R8 | Contract is paused | `Paused` | 7 | `test_revoke_negative_paused` |

### Auth rules for `verify` and `revoke_verification`

```
caller == admin                   →  allowed
caller has Role::Verifier         →  allowed
caller has Role::Upgrader         →  NotAuthorized (code 3)
caller has no role                →  NotAuthorized (code 3)
```

Both functions require a `caller: Address` argument so the contract can call
`caller.require_auth()` and enforce the role check in a single auditable step.
Only the admin and any address granted `Role::Verifier` via `set_role` may
call these functions.

The `verify` function additionally guards against illegal state transitions:
- Verifying an unregistered username → `NotRegistered` (code 4)
- Re-verifying an already-verified username → `AlreadyVerified` (code 5)

The `revoke_verification` function guards:
- Revoking from an unregistered username → `NotRegistered` (code 4)
- Revoking from a username that was never verified → `NotVerified` (code 6)

---

## Responsible Disclosure

If you discover a security vulnerability:

1. **Do not** open a public GitHub issue
2. Email the maintainers or use GitHub Security Advisories on the repository
3. Include steps to reproduce, impact assessment, and suggested fix if available

We aim to acknowledge reports within 72 hours.

---

## Futurenet Deploy Smoke Workflow

Wave #39: before an audit or a testnet/mainnet promotion, validate a fresh
deploy against Futurenet to catch threat-model regressions early (e.g. an
`initialize` gate that silently no-ops, or a lookup that leaks state before
verification).

1. Deploy to Futurenet: `ADMIN=G... ./scripts/futurenet_smoke_test.sh`
2. Confirm `get_stats` reports `{total: 0, verified: 0}` on the fresh instance
   — a nonzero result means the deploy reused stale storage.
3. Confirm `has_record` returns `false` for an unregistered username — this
   guards the "no on-chain proof of GitHub ownership" boundary called out
   above by verifying reads don't fabricate positive results.
4. Re-run after any change to `initialize`, `register`, or storage key
   layout, since those are the surfaces the threat model above depends on.

The script is a deploy sanity check, not a substitute for `cargo test`
(see `src/lib.rs` and `tests/integration.rs` for functional coverage).

---

## Verify and Revoke Verification CLI Usage

The `verify` and `revoke_verification` functions are admin-only in the CLI documentation. The authoritative examples below use `--source = admin`. A non-admin caller (including a registrant) receives `NotAuthorized` and the transaction reverts.

### Authoritative examples (admin path)

```bash
# Verify a contributor (admin must sign)
stellar contract invoke --id $ID --source admin --network testnet --send=yes \
  -- verify --caller G... --github-username octocat

# Revoke verification (admin must sign)
stellar contract invoke --id $ID --source admin --network testnet --send=yes \
  -- revoke_verification --caller G... --github-username octocat
```

### Unauthorized failure (non-admin)

If a non-admin address attempts either call, the transaction fails with `NotAuthorized`:

```bash
# This will fail — registrant cannot self-verify
stellar contract invoke --id $ID --source registrant --network testnet --send=yes \
  -- verify --caller G... --github-username octocat
# Error: NotAuthorized (code 3)
```

Do not construct CLI examples that imply a registrant can self-verify. The contract rejects such calls at the auth layer.

---

## Verifier / Revoker Role Separation (Issue #212)

Prior to this change, `Role::Verifier` could both `verify` and
`revoke_verification`. A single compromised key could therefore silently strip
payout eligibility from any contributor.

`Role::Verifier` — may only call `verify`.  
`Role::Revoker` — may only call `revoke_verification`.  
`Admin` — can still do both.

**Migration for live deployments:** Existing `Role::Verifier` holders keep their
verify permission unchanged. If an operator previously relied on a Verifier to
also revoke, assign that address `Role::Revoker` via `set_role`.

---

## Challenge-Period Flow (Issue #214)

Admin force-remove was previously instant and irreversible. A legitimate
registrant could lose their name with no recourse.

`start_challenge(caller, github_username)` places the name in a locked state for
`DEFAULT_CHALLENGE_DELAY_SECS` (48 hours). During this window:

- Re-registration by anyone other than the current owner is blocked.
- The current registrant may still `remove` their own record, which clears the
  challenge atomically — they proved ownership by signing.
- `complete_challenge` is gated behind the delay. Calling it before `resolve_after`
  returns `ChallengeNotResolvable`.

After the delay, the admin calls `complete_challenge`, which removes the record
and emits both `RemovedEvent` and `ChallengeCompletedEvent`.

`cancel_challenge` is the escape hatch: if the registrant proves ownership off-chain
during the window, the admin cancels the challenge and the registration is preserved.

---

## On-Chain Audit Logging

The contract records structured audit log entries into contract storage upon state mutations (`initialize`, `register`, `remove`, `verify`, `batch_verify`, `pause`, `unpause`, `config_verification`, `set_role`).

### What IS an On-Chain Audit Log

- **Structured compliance record**: An on-chain log entry (`AuditLogEntry`) persisted in instance storage recording event type (`AuditEventType`), timestamp, actor address, target username/address, and details.
- **Operator query surface**: Callable on-chain via `get_audit_logs()` and `get_audit_stats()`.
- **Bounded ring buffer**: Maintained up to a maximum cap (100 entries) per contract instance to stay within Soroban memory and footprint boundaries.

### What IS NOT an On-Chain Audit Log

- **Domain events replacement**: Audit log entries complement, but do not replace, Soroban domain events (`RegisteredEvent`, `VerifiedEvent`, `RemovedEvent`, etc.). Off-chain indexers still rely on domain events for event stream monitoring.
- **Unbounded historical store**: Audit entries are capped on-chain. Complete long-term history across all ledgers should be collected by off-chain indexers from event topics or block archives.

---

## Audit Status

This contract has **not** been formally audited. Use at your own risk on mainnet until an audit is completed.

For production deployments, consider:

- Independent security audit
- Bug bounty program
- Staged rollout on testnet/futurenet first


### What IS an On-Chain Audit Log

- **Structured compliance record**: An on-chain log entry (`AuditLogEntry`) persisted in instance storage recording event type (`AuditEventType`), timestamp, actor address, target username/address, and details.
- **Operator query surface**: Callable on-chain via `get_audit_logs()` and `get_audit_stats()`.
- **Bounded ring buffer**: Maintained up to a maximum cap (100 entries) per contract instance to stay within Soroban memory and footprint boundaries.

### What IS NOT an On-Chain Audit Log

- **Domain events replacement**: Audit log entries complement, but do not replace, Soroban domain events (`RegisteredEvent`, `VerifiedEvent`, `RemovedEvent`, etc.). Off-chain indexers still rely on domain events for event stream monitoring.
- **Unbounded historical store**: Audit entries are capped on-chain. Complete long-term history across all ledgers should be collected by off-chain indexers from event topics or block archives.

---

## Audit Status

This contract has **not** been formally audited. Use at your own risk on mainnet until an audit is completed.

For production deployments, consider:

- Independent security audit
- Bug bounty program
- Staged rollout on testnet/futurenet first

