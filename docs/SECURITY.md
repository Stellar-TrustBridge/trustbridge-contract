# Security

Security considerations for **trustbridge-contract**.

Related docs: [README](../README.md) · [ARCHITECTURE](ARCHITECTURE.md) · [DEPLOYMENT](DEPLOYMENT.md)

---

## Threat Model

### In Scope

| Threat | Mitigation |
|--------|------------|
| Impersonation (registering someone else's GitHub username) | `stellar_address.require_auth()` — only the address owner can register |
| Unauthorized removal | `caller` must auth as registrant or admin |
| Unauthorized admin actions | `admin.require_auth()` on `verify` and `get_all_registered` |
| Double initialization | `AlreadyInitialized` error |

### Out of Scope (handled off-chain)

| Concern | Responsibility |
|---------|----------------|
| GitHub identity proof | Admin verification workflow + TrustBridge dashboard |
| Username squatting policy | Social/process layer; contract allows first-come registration |
| Admin key compromise | Operational security; use multisig for admin address |
| GitHub username changes | Off-chain mapping updates; may require re-registration |

---

## Admin Key Management

The admin address is **immutable** after `initialize`. Recommendations:

- Use a **multisig** or **smart account** as the admin G-address
- Never commit private keys or seed phrases
- Rotate operational keys via deploying a new contract instance if admin is compromised (no on-chain admin transfer in v0.1)

---

## Registration Integrity

- Registering a username requires the Stellar address owner to sign
- Re-registration with a new address resets verification status
- There is no on-chain proof of GitHub ownership at registration time — verification is a separate admin step

---

## Storage TTL

Persistent entries on Stellar mainnet have a **time-to-live (TTL)**. If entries expire, data may become unavailable until extended.

Operational teams should:

1. Monitor entry TTL via RPC
2. Run periodic TTL extension via Stellar CLI (`stellar contract extend`)
3. Document extension cadence in deployment runbooks

---

## Responsible Disclosure

If you discover a security vulnerability:

1. **Do not** open a public GitHub issue
2. Email the maintainers or use GitHub Security Advisories on the repository
3. Include steps to reproduce, impact assessment, and suggested fix if available

We aim to acknowledge reports within 72 hours.

---

## Audit Status

This contract has **not** been formally audited. Use at your own risk on mainnet until an audit is completed.

For production deployments, consider:

- Independent security audit
- Bug bounty program
- Staged rollout on testnet/futurenet first
