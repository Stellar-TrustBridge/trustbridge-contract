# Public entrypoint golden (Issue #631)

`public_entrypoints.golden.json` is a sorted snapshot of **public contract
function names and arity** extracted from `docs/abi.json`.

Error discriminants are **not** recorded here. Those stay in
`abi/contract_error_codes.golden` and `tests/contract_error_codes.rs`.

## When to update

Commit a golden change only when a public entrypoint is intentionally added,
removed, renamed, or changes arity. Unrelated `docs/ABI.md` wording must not
rewrite this file.

## Regeneration

```bash
python3 scripts/generate_abi_json.py                 # refresh docs/abi.json from docs/ABI.md
python3 scripts/generate_abi_json.py --update-entrypoints-golden
make abi-check
cargo test --test abi_entrypoints_golden
```

`UPDATE_GOLDEN=1` is **not** wired for this snapshot: the file is written only
by `--update-entrypoints-golden` so the diff is reviewable in Git.
