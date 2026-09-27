#!/usr/bin/env python3
"""Generate a payout allowlist CSV from on-chain registry state (Issue #285).

Treasury teams need a CSV of payout recipients sourced directly from the
contract rather than a possibly-stale dashboard cache. This reads the
unauthenticated ``get_public_paginated`` endpoint (no admin key required),
keeps verified records only by default, and writes one row per contributor.

Read-only: no mutating call is ever made. Payment submission is out of scope.

Payout mutation surface (Issue #407): the only entry point that changes a
payout address is ``set_payout_address``. It is admin-gated (the caller must
hold the contract admin role) and rejects malformed addresses with the stable
error codes ``InvalidPayoutAddress`` and ``Unauthorized``. This script never
invokes it; it only reads the resulting state, so the allowlist stays aligned
with the documented ABI mutation surface.
"""

from __future__ import annotations

import argparse
import csv
import os
import re
import sys
from pathlib import Path

from trustbridge_client import StellarCLIError, TrustBridgeClient

# Column order is aligned with the JSON export in export_registry.py, with the
# payout destination added as the leading operational field.
COLUMNS = ["github_username", "payout_address", "stellar_address", "verified", "registered_at"]

# Stable error codes emitted by the contract's set_payout_address mutation
# (Issue #407). Kept here so tooling and the ABI doc agree on the surface.
PAYOUT_MUTATION = "set_payout_address"
PAYOUT_ERROR_CODES = ("Unauthorized", "InvalidPayoutAddress")

# Payout addresses are Stellar account IDs (ed25519 public keys, StrKey "G...").
# Mirrors the on-chain validation so the allowlist never emits a row the
# contract would reject on a subsequent set_payout_address call.
PAYOUT_ADDRESS_RE = re.compile(r"^G[A-Z2-7]{55}$")


def is_valid_payout_address(address: str) -> bool:
    """Return True when *address* matches the contract's payout address format."""
    return bool(address) and PAYOUT_ADDRESS_RE.match(address) is not None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--contract", default=os.environ.get("CONTRACT_ID"), help="deployed contract ID")
    parser.add_argument(
        "--source",
        default=os.environ.get("SOURCE", "default"),
        help="Stellar CLI identity used to sign the read (any funded identity; no admin role needed)",
    )
    parser.add_argument("--network", default=os.environ.get("NETWORK", "testnet"))
    parser.add_argument("--output", default=os.environ.get("OUTPUT_FILE"), help="output CSV path")
    parser.add_argument("--page-limit", type=int, default=int(os.environ.get("PAGE_LIMIT", "100")))
    parser.add_argument(
        "--include-unverified",
        action="store_true",
        help="opt in to unverified rows (default: verified-only, so squatter registrations are never paid)",
    )
    parser.add_argument(
        "--include-bots",
        action="store_true",
        help="keep records flagged as CI bots (default: excluded from payout allowlists)",
    )
    parser.add_argument(
        "--include-invalid-addresses",
        action="store_true",
        help="keep rows whose payout_address fails format validation (default: excluded)",
    )
    args = parser.parse_args()

    if not args.contract:
        parser.error("--contract or CONTRACT_ID is required")
    if args.page_limit < 1:
        parser.error("--page-limit must be positive")

    output = Path(args.output or f"payout-allowlist-{args.network}.csv")
    client = TrustBridgeClient(args.contract, args.source, args.network)

    rows = []
    total = 0
    skipped_unverified = 0
    skipped_bots = 0
    skipped_invalid = 0
    for record in client.iter_public_records(args.page_limit):
        total += 1
        if not args.include_unverified and not record.verified:
            skipped_unverified += 1
            continue
        if not args.include_bots and record.is_bot:
            skipped_bots += 1
            continue
        if not args.include_invalid_addresses and not is_valid_payout_address(record.payout_address):
            skipped_invalid += 1
            continue
        rows.append(
            {
                "github_username": record.github_username,
                "payout_address": record.payout_address,
                "stellar_address": record.stellar_address,
                "verified": str(record.verified).lower(),
                "registered_at": record.registered_at,
            }
        )

    with output.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=COLUMNS)
        writer.writeheader()
        writer.writerows(rows)

    print(
        f"Wrote {len(rows)} row(s) to {output} "
        f"(scanned {total}, skipped {skipped_unverified} unverified, "
        f"{skipped_bots} bot, {skipped_invalid} invalid-address)"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (StellarCLIError, OSError, RuntimeError, ValueError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        raise SystemExit(1) from exc
