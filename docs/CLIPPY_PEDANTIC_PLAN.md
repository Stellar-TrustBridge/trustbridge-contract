# Clippy Pedantic Cleanup — Design Plan

**Issue:** #109  
**Status:** Design phase — phased rollout in progress  
**Related files:** `Makefile`, `src/lib.rs`, `src/storage.rs`, `src/error.rs`, `src/utils.rs`

---

## Background

The current Clippy invocation in `Makefile` and CI is:

```makefile
lint:
    cargo clippy --all-targets -- -D warnings
```

This runs only the default Clippy lints. Pedantic and nursery lint groups
(`clippy::pedantic`, `clippy::nursery`) are not enabled. Those groups surface
auth/storage footguns (e.g. integer casts that silently truncate, needlessly
permissive function signatures, missing error-handling docs) that are
particularly relevant for a Soroban smart contract where bugs are expensive
to fix post-deploy.

The goal is a **phased rollout** — not a big-bang `#![deny(clippy::pedantic)]`
that breaks the build — so that each batch of fixes can be reviewed in isolation,
rationale is documented, and exceptions are explained.

---

## Current State Inventory

### Clippy invocation

| Location | Current flags |
|----------|---------------|
| `Makefile` → `lint` target | `cargo clippy --all-targets -- -D warnings` |
| `.github/workflows/ci.yml` → `cargo clippy` step | `cargo clippy --all-targets -- -D warnings` |

No `#![allow(clippy::*)]` attributes exist anywhere in `src/`.

### Source modules

| Module | Notes |
|--------|-------|
| `src/lib.rs` | Large (~2 700 lines); most pedantic warnings will originate here |
| `src/storage.rs` | Many `u32`/`u64` casts; `must_use` candidates |
| `src/error.rs` | Clean; `#[contracterror]` proc-macro output |
| `src/events.rs` | Struct-only; low lint surface |
| `src/utils.rs` | Username validation; `must_use` candidates |
| `src/version.rs` | Version struct helpers |
| `src/audit.rs` | Audit log helpers |
| `src/batch.rs` | Batch helpers |
| `src/error_context.rs` | Error context wrappers |

---

## Proposed Lint Groups and Phasing

### Phase 1 — Low-risk, high-signal (land first)

Enable these lints immediately; fixes are mechanical and low-risk:

| Lint | Rationale |
|------|-----------|
| `clippy::must_use_candidate` | Flag pure functions whose return value is routinely discarded; critical for error-propagation correctness |
| `clippy::missing_errors_doc` | Enforce `# Errors` rustdoc sections on `Result`-returning public functions |
| `clippy::missing_panics_doc` | Enforce `# Panics` sections where `unwrap`/`expect` are used |
| `clippy::redundant_closure_for_method_calls` | Pure style; zero-risk |
| `clippy::cloned_instead_of_copied` | Minor performance improvement for `Copy` types |

**Implementation:** Add the following to the top of `src/lib.rs`:

```rust
#![warn(
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::redundant_closure_for_method_calls,
    clippy::cloned_instead_of_copied,
)]
```

Using `warn` (not `deny`) so the build remains green while fixes land incrementally.

### Phase 2 — Medium-risk, high-value

Enable after Phase 1 fixes are merged:

| Lint | Rationale | Known allow-list candidates |
|------|-----------|----------------------------|
| `clippy::cast_possible_truncation` | Catches `u64 as u32` that silently truncate — critical for ledger timestamp arithmetic | Soroban SDK conversions that are provably safe by protocol bounds |
| `clippy::cast_sign_loss` | Catches `i64 as u64` sign-loss | Same |
| `clippy::integer_division` | Catches integer division that drops remainder unintentionally | Benchmark/stats rounding that is explicitly floor-division |
| `clippy::wildcard_imports` | Bans `use foo::*`; forces explicit imports for readability | None — no wildcard imports currently exist |
| `clippy::items_after_statements` | Keeps item declarations at the top of blocks | Isolated occurrences in test helper closures |

### Phase 3 — Selective nursery lints

Enable only after Phases 1–2 are complete and lint output is clean:

| Lint | Rationale | Notes |
|------|-----------|-------|
| `clippy::cognitive_complexity` | Flag functions that are too complex to reason about (auth, register) | Requires setting a threshold; start at 30 |
| `clippy::option_if_let_else` | Encourages `Option::map_or_else` over `if let` chains | Review each site — sometimes the `if let` form is clearer |
| `clippy::manual_let_else` | Encourages `let…else` in Rust 1.65+ | Soroban MSRV is 1.84; safe to enable |

### Lint groups explicitly excluded

| Group | Reason |
|-------|--------|
| `clippy::restriction` | Contains contradictory lints (e.g. `else_if_without_else` conflicts with idiomatic Rust); not suitable for blanket enable |
| `clippy::pedantic` (full group) | Too many lints with legitimate exceptions in `#![no_std]` / Soroban context; enable individually instead |
| `clippy::nursery` (full group) | Unstable; lint set changes across Rust releases; enable individually |

---

## End-State CI / Make Commands

Once all phases are complete, the `lint` target and CI step will be:

**Makefile:**

```makefile
lint: ## Run clippy (default + phase-1 lints promoted to deny)
	cargo clippy --all-targets -- \
	  -D warnings \
	  -D clippy::must_use_candidate \
	  -D clippy::missing_errors_doc \
	  -D clippy::missing_panics_doc \
	  -D clippy::cast_possible_truncation \
	  -D clippy::cast_sign_loss \
	  -D clippy::wildcard_imports
```

**CI (`.github/workflows/ci.yml`):**

```yaml
- name: cargo clippy
  run: |
    cargo clippy --all-targets -- \
      -D warnings \
      -D clippy::must_use_candidate \
      -D clippy::missing_errors_doc \
      -D clippy::missing_panics_doc \
      -D clippy::cast_possible_truncation \
      -D clippy::cast_sign_loss \
      -D clippy::wildcard_imports
```

---

## Allow-List Policy

Any `#[allow(clippy::*)]` attribute added to source code **must** include an
inline comment explaining why the exception is intentional:

```rust
// SAFETY: Soroban ledger timestamps are u64 values guaranteed by the
// protocol to fit in u32 for the current epoch; this cast is safe.
#[allow(clippy::cast_possible_truncation)]
let ts = env.ledger().timestamp() as u32;
```

Undocumented `allow` attributes will be rejected in code review. This policy
is enforced by convention; a future PR may add a CI check via `grep` for
bare `#[allow(clippy::` without an adjacent comment.

---

## Phase 1 — First Batch (landed with this PR)

The following low-risk `warn` attributes are added to `src/lib.rs` to begin
surfacing lint output without breaking the build:

```rust
#![warn(
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::redundant_closure_for_method_calls,
    clippy::cloned_instead_of_copied,
)]
```

These produce `warning` output (not errors), keeping CI green while the
follow-up fix PRs work through the results.

---

## References

- [Clippy lint list](https://rust-lang.github.io/rust-clippy/master/)
- [Clippy pedantic group](https://rust-lang.github.io/rust-clippy/master/#/?groups=pedantic)
- [Clippy nursery group](https://rust-lang.github.io/rust-clippy/master/#/?groups=nursery)
- Soroban MSRV: 1.84 (see `rust-toolchain.toml`)
