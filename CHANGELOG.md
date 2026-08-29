# Changelog

All notable public ABI changes are recorded here. Entries use the contract
version exposed by `version()` and follow semantic versioning: major for
breaking changes, minor for additive interface changes, and patch for
compatible corrections.

## [1.0.0] - 2026-08-28

### ABI snapshot

- Initial documented TrustBridge Contract ABI, including registry operations,
  verification and revocation, pagination, role management, upgrade
  attestation, admin transfer, challenge-period, health, and network-tagging
  interfaces.
- Initial event and public type reference captured in [docs/ABI.md](docs/ABI.md).

<!-- changelog-check: skip - Added get_verification_config() docs entry and VerificationConfiguredEvent to docs/ABI.md; both were already implemented under the 1.0.0 ABI (config observability follow-up), no version bump. -->

