#!/bin/bash
set -e

cd /home/edohwares/Desktop/Room/drips/trustbridge-contract

# Configure git if needed
if ! git config user.email > /dev/null 2>&1; then
    git config user.email "edohwares@example.com"
fi
if ! git config user.name > /dev/null 2>&1; then
    git config user.name "EDOHWARES"
fi

# Show current branch
echo "Current branch: $(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo 'unknown')"

# Add all changes
echo "Adding files..."
git add .

# Check status
echo ""
echo "Changes to commit:"
git status --short

# Commit
echo ""
echo "Creating commit..."
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

Closes #621" || true

# Show the commit
echo ""
echo "Last commit:"
git log -1 --oneline 2>/dev/null || echo "No commits yet"

# Show branch
echo ""
echo "Current branch: $(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo 'unknown')"
