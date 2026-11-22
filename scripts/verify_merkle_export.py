#!/usr/bin/env python3
"""Verify Merkle roots captured for each page in a registry export JSON."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path
from typing import Any

LEAF_DOMAIN = b"trustbridge/export-leaf/v1:"
NODE_DOMAIN = b"trustbridge/export-node/v1:"


def decode_root(value: Any) -> bytes:
    if isinstance(value, str):
        encoded = value.removeprefix("0x")
        try:
            root = bytes.fromhex(encoded)
        except ValueError as exc:
            raise ValueError("merkle_root must be a 32-byte hexadecimal value") from exc
    elif isinstance(value, list) and all(type(byte) is int and 0 <= byte <= 255 for byte in value):
        root = bytes(value)
    else:
        raise ValueError("merkle_root must be a 32-byte hexadecimal value or byte array")
    if len(root) != 32:
        raise ValueError("merkle_root must contain exactly 32 bytes")
    return root


def leaf_hash(record: dict[str, Any]) -> bytes:
    username = record.get("github_username")
    address = record.get("stellar_address")
    verified = record.get("verified")
    if not isinstance(username, str) or not isinstance(address, str) or type(verified) is not bool:
        raise ValueError("each record needs github_username, stellar_address, and boolean verified")
    payload = LEAF_DOMAIN + username.encode() + b"\0" + address.encode() + bytes([verified])
    return hashlib.sha256(payload).digest()


def page_root(records: list[dict[str, Any]]) -> bytes:
    level = [leaf_hash(record) for record in records]
    while len(level) > 1:
        level = [
            hashlib.sha256(NODE_DOMAIN + level[i] + level[i + 1]).digest()
            if i + 1 < len(level)
            else level[i]
            for i in range(0, len(level), 2)
        ]
    return level[0] if level else bytes(32)


def verify_export(path: Path) -> int:
    document = json.loads(path.read_text(encoding="utf-8"))
    pages = document.get("pages") if isinstance(document, dict) else None
    if not isinstance(pages, list):
        raise ValueError("export must contain a pages array with captured merkle_root values")
    total = 0
    for index, page in enumerate(pages):
        if not isinstance(page, dict) or not isinstance(page.get("records"), list):
            raise ValueError(f"page {index} must contain a records array")
        if any(not isinstance(record, dict) for record in page["records"]):
            raise ValueError(f"page {index} records must be JSON objects")
        expected = decode_root(page.get("merkle_root"))
        actual = page_root(page["records"])
        if actual != expected:
            raise ValueError(f"page {index} (cursor {page.get('cursor')}) Merkle root mismatch")
        total += len(page["records"])
    print(f"Verified {len(pages)} page(s), {total} record(s): Merkle roots match")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("export", type=Path, help="registry-export JSON from export_registry.py")
    args = parser.parse_args()
    try:
        return verify_export(args.export)
    except (OSError, json.JSONDecodeError, ValueError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
