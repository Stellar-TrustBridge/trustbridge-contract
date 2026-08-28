#!/usr/bin/env python3
"""Export the TrustBridge registry through the typed operator client."""

from __future__ import annotations

import argparse
import json
import os
from datetime import datetime, timezone
from pathlib import Path

from trustbridge_client import StellarCLIError, TrustBridgeClient


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--contract", default=os.environ.get("CONTRACT_ID"), help="deployed contract ID")
    parser.add_argument("--source", default=os.environ.get("SOURCE"), help="admin Stellar CLI identity")
    parser.add_argument("--network", default=os.environ.get("NETWORK", "testnet"))
    parser.add_argument("--output", default=os.environ.get("OUTPUT_FILE"), help="output JSON path")
    parser.add_argument("--page-limit", type=int, default=int(os.environ.get("PAGE_LIMIT", "100")))
    args = parser.parse_args()

    if not args.contract:
        parser.error("--contract or CONTRACT_ID is required")
    if not args.source:
        parser.error("--source or SOURCE is required")
    if args.page_limit < 1:
        parser.error("--page-limit must be positive")

    output = Path(args.output or f"registry-export-{args.network}.json")
    client = TrustBridgeClient(args.contract, args.source, args.network)
    records = []
    cursor = 0
    for _ in range(100_000):
        page = client.get_registered_page(cursor, args.page_limit)
        records.extend(
            {
                "github_username": record.github_username,
                "stellar_address": record.stellar_address,
                "verified": record.verified,
                "registered_at": record.registered_at,
            }
            for record in page.records
        )
        if not page.has_more or page.next_cursor is None:
            break
        if page.next_cursor == cursor:
            raise RuntimeError(f"pagination stalled at cursor {cursor}")
        cursor = page.next_cursor
    else:
        raise RuntimeError("exceeded 100000 pages without exhausting the index")

    document = {
        "schema_version": 1,
        "contract_id": args.contract,
        "network": args.network,
        "exported_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "count": len(records),
        "records": records,
    }
    output.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
    print(f"Wrote {len(records)} record(s) to {output}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (StellarCLIError, OSError, RuntimeError, ValueError) as exc:
        print(f"ERROR: {exc}", file=__import__("sys").stderr)
        raise SystemExit(1) from exc
