# Registry Invariants

These invariants help reviewers reason about safe contract changes.

## Identity mapping

- A GitHub username maps to at most one Stellar address at a time.
- Registration changes should keep the total count in sync with stored records.
- Removal should clear lookup state and update export indexes consistently.

## Verification

- Only the configured admin can mark a contributor as verified.
- Verification should never replace address ownership checks.
- Verified count must only change when a record crosses the unverified/verified boundary.
