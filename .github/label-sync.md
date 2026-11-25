# GitHub Label Sync Guide

This document describes how to sync labels defined in `labels.json` to the GitHub repository.

## Automated Sync (Recommended)

Use `github-label-sync` to keep labels in sync:

```bash
npm install -g github-label-sync
github-label-sync \
  --access-token $GITHUB_TOKEN \
  --labels .github/labels.json \
  owner/trustbridge-contract
```

## Manual Sync

If `github-label-sync` is not available, use GitHub CLI:

```bash
# Requires: gh cli (https://cli.github.com/)

# Create labels
gh label create dx --description "Developer experience improvements" --color "0366d6"
gh label create documentation --description "Documentation updates and improvements" --color "0075ca"
gh label create "complexity: high" --description "High complexity task requiring careful implementation" --color "b60205"
gh label create "points: 200" --description "Estimated at 200 story points" --color "fbca04"
gh label create wave --description "Related to Wave registry system" --color "5319e7"
gh label create "repo: trustbridge-contract" --description "Specific to trustbridge-contract repository" --color "1d76db"
gh label create "category: Events" --description "Related to event handling and indexing" --color "cccccc"
```

## Label Definitions

See `.github/labels.json` for the authoritative label definitions.

## CI/CD Integration

Consider adding a GitHub Action to automatically sync labels on each push:

```yaml
# .github/workflows/sync-labels.yml
name: Sync Labels
on: [push]
jobs:
  sync:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: Financial-Times/github-label-sync@v2
        with:
          labels: .github/labels.json
          prune: false
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

## References

- [github-label-sync](https://github.com/Financial-Times/github-label-sync)
- [GitHub CLI label docs](https://cli.github.com/manual/gh_label)
- [GitHub REST API - Labels](https://docs.github.com/en/rest/issues/labels)
