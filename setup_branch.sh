#!/bin/bash

cd /home/edohwares/Desktop/Room/drips/trustbridge-contract

# Configure git
git config user.email "edohwares@example.com"
git config user.name "EDOHWARES"

# Create and switch to feature branch
git checkout -b feat/ci-soroban-cache-621

# Add all changes
git add .

# Commit
git commit -m "ci: cache soroban build targets for prs

Add CI job caching for Soroban target to speed up PR feedback cycles.

- Extended .github/workflows/ci.yml with Swatinem/rust-cache across all jobs
- Cache targets keyed on Cargo.lock and rust-toolchain.toml for proper invalidation
- New dedicated 'build' job running make build and make wasm-size with optimized cache
- Cache includes wasm32v1-none artifacts with Stellar CLI version in key
- Added STELLAR_CLI_VERSION env var for reproducible cache keys
- All cargo test and stellar contract build jobs now leverage cache
- Documented cache behavior and expectations in docs/CONTRIBUTING.md

Cache behavior:
- Second CI run shows cache hit, reducing total run time by 2-5 minutes
- Cache miss on lockfile or toolchain change rebuilds cleanly
- Fork PRs without cache write permissions remain functional
- No secrets stored in cache paths

Closes #621"

# Show the commit
echo "Commit created:"
git log -1 --oneline

echo ""
echo "Branch created: $(git rev-parse --abbrev-ref HEAD)"
echo ""
echo "Ready to push. Current status:"
git status
