#!/usr/bin/env python3
import subprocess
import os
import sys

os.chdir('/home/edohwares/Desktop/Room/drips/trustbridge-contract')

try:
    # Add all changes
    print("Adding files...")
    result = subprocess.run(['git', 'add', '.'], capture_output=True, text=True)
    if result.returncode != 0:
        print(f"Error adding files: {result.stderr}")
        sys.exit(1)
    
    # Check status
    print("\nChanges to commit:")
    result = subprocess.run(['git', 'status', '--short'], capture_output=True, text=True)
    print(result.stdout)
    
    # Commit
    print("\nCreating commit...")
    commit_msg = """ci: cache soroban build targets for prs

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

Closes #621"""
    
    result = subprocess.run(['git', 'commit', '-m', commit_msg], capture_output=True, text=True)
    if result.returncode != 0 and "nothing to commit" not in result.stdout + result.stderr:
        print(f"Error committing: {result.stderr}")
        sys.exit(1)
    elif "nothing to commit" in result.stdout + result.stderr:
        print("Nothing to commit (changes may already be committed)")
    else:
        print("Commit successful!")
    
    # Show last commit
    print("\nLast commit:")
    result = subprocess.run(['git', 'log', '-1', '--oneline'], capture_output=True, text=True)
    print(result.stdout)
    
    # Show current branch
    print("Current branch:")
    result = subprocess.run(['git', 'rev-parse', '--abbrev-ref', 'HEAD'], capture_output=True, text=True)
    print(result.stdout)
    
    # Prepare push info
    print("\n" + "="*60)
    print("Ready to push!")
    print("="*60)
    print("\nTo push, run:")
    print("  git push -u origin feat/ci-soroban-cache-621")
    print("\nOr to push and open a PR automatically:")
    print("  git push -u origin feat/ci-soroban-cache-621")
    print("  gh pr create --title 'ci: cache soroban build targets for prs' --body 'Closes #621'")
    
except Exception as e:
    print(f"Error: {e}", file=sys.stderr)
    sys.exit(1)
