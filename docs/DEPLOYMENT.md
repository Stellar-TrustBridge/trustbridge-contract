WASM size budget


### Local wasm-size Check

Before opening a PR, always run the wasm size check locally to catch oversized builds early:

```bash
make wasm-size
```

This command builds the optimized release WASM and compares its size against `WASM_SIZE_LIMIT` in the Makefile. If the build exceeds the budget, the command exits with a non-zero status and prints the current size vs. the limit.

**Important**: Always run `make wasm-size` after making changes to the contract code, especially when adding new features or dependencies. CI runs this check immediately after the release build, so a size increase beyond the budget will fail the CI job.

### Intentional Limit Bump Process

If a new feature legitimately requires increasing the WASM size budget, follow this process:

1. **Document the reason** in the PR description: explain why the size increase is necessary and reference the relevant benchmark data (see [BENCHMARK_BUDGETS.md](./BENCHMARK_BUDGETS.md)).
2. **Update the Makefile**: increase `WASM_SIZE_LIMIT` in the Makefile.
3. **Update CI**: increase the corresponding limit in `.github/workflows/ci.yml`.
4. **Update documentation**: update this section (DEPLOYMENT.md) to reflect the new budget.
5. **Get approval**: the PR must be reviewed and approved by maintainers before merging.

**Note**: Debug builds are not compared against the release limit. Only optimized release builds are checked.

### Troubleshooting

If `make wasm-size` reports a size that differs from what you expect:

- **Stellar contract build vs. cargo build**: The check uses `stellar contract build` (the same command used for deployment), not plain `cargo build`. If you see a size discrepancy, ensure you're comparing release builds, not debug builds.
- **Feature flags**: Size can vary based on enabled feature flags. Check the Makefile for any feature flags that affect the build.
- **wasm-opt**: The release build runs wasm-opt for size optimization. If you're building locally without wasm-opt, the size will be larger.
- **wasm-hash.pin**: The wasm hash pin is separate from the size check. See the [Build hash pin](#build-hash-pin) section for details.

### Related Documentation

- [BENCHMARK_BUDGETS.md](./BENCHMARK_BUDGETS.md) — Benchmark data and size budgets for different features
- [CONTRIBUTING.md](./CONTRIBUTING.md) — Contribution guidelines and quality gates
- [DEPLOYMENT.md](./DEPLOYMENT.md) — This file (deployment guide)
- 
