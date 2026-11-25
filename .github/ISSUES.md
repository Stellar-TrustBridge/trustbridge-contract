# Trustbridge Contract Issues

This document tracks high-priority Wave registry issues.

---

## Issue 1: Event Fixture Refresh Documentation

**Labels:** `dx`, `documentation`, `complexity: high`, `points: 200`, `wave`, `repo: trustbridge-contract`, `category: Events`

**Title:** Add documented, deterministic fixture refresh process for event replay tests

**Description:**

Contributors need a documented, deterministic way to refresh `tests/testdata/event_replay_fixture.json` after intentional event changes.

### Why this matters
Manual fixture edits cause flaky Wave CI and review thrash.

### Requirements
- Add make/script target for deterministic fixture generation
- Document the fixture refresh process in CONTRIBUTING.md
- Include failure modes and troubleshooting steps
- Ensure test automation validates fixture consistency

### Acceptance Criteria
- Fixture refresh is fully automated and deterministic
- CONTRIBUTING.md contains clear step-by-step guide
- CI validates fixture consistency on every test run
- Zero manual fixture editing required

---

## Issue 2: TTL Extension Budget Documentation

**Labels:** `documentation`, `complexity: high`, `points: 200`, `wave`, `repo: trustbridge-contract`

**Title:** Document max batch sizes for TTL extension with measured fixtures

**Description:**

Document max batch sizes for TTL extension, link BENCHMARK_BUDGETS, and ensure tests/scripts match. Include failure mode when budget exceeded.

### Why this matters
TTL ops that OOG (Out of Gas) leave Wave registries partially extended, compromising safety guarantees.

### Requirements
- Document max batch sizes for TTL extension in docs/BENCHMARK_BUDGETS.md
- Link BENCHMARK_BUDGETS.md from docs/STORAGE_RENT.md
- Ensure tests/extend_registry_ttl.rs matches documented limits
- Match src/contract/maintenance.rs implementation with documented behavior
- Update docs/STORAGE_RENT.md for user/operator-facing changes

### Acceptance Criteria
- docs/BENCHMARK_BUDGETS.md contains measured batch size limits
- tests/extend_registry_ttl.rs covers happy path + 2 failure paths (OOG scenarios)
- docs/STORAGE_RENT.md references batch size limits
- All three files (docs, tests, implementation) are in sync
- Failure path tests demonstrate safe partial-extension handling

---

## Issue 3: Registry Init and Health Probes

**Labels:** `complexity: high`, `points: 200`, `wave`, `repo: trustbridge-contract`

**Title:** Add registry initialization state tracking and health probe consistency

**Description:**

After init, probes show `admin_set` true; double init is still rejected. Add integration test to verify init state tracking and health probe consistency.

### Why this matters
Probes must reflect Wave init state accurately to ensure operators can monitor registry health.

### Requirements
- Add integration test: initialize → probes → initialize (should be rejected)
- Keep tests/integration.rs, src/lib.rs, and docs/CONTRACT_HEALTH.md consistent
- Update docs/ABI.md when user/operator-facing surfaces change
- Preserve existing auth, pause/freeze, dry-run, and happy-path semantics
- Match existing patterns; do not invent parallel subsystems

### Acceptance Criteria
- Integration test covers happy path: init → probes show admin_set true
- Double init is rejected with appropriate error
- docs/CONTRACT_HEALTH.md documents init state and probe behavior
- docs/ABI.md documents any new ABI surfaces
- Tests cover happy path + 2 distinct failure paths
- All existing semantics preserved and documented

---

## Recommended Implementation Order

1. **Issue 1 (Event Fixture Refresh)** — Improves DX immediately, unblocks other work
2. **Issue 2 (TTL Budget Documentation)** — Ensures safety for Wave operators
3. **Issue 3 (Registry Init & Health)** — Completes monitoring and correctness story

