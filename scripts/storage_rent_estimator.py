#!/usr/bin/env python3
"""Estimate on-chain Soroban storage rent from the versioned estimator inputs.

Consumes ``docs/storage-rent-estimator.inputs.v1.json`` (see
``docs/STORAGE_RENT_ESTIMATOR.md``) and prints the persistent/instance entry
counts for a given contributor count ``N``. Entry counts only — converting to
XLM requires operator-supplied protocol rent params and is out of scope here.

The estimator also cross-checks ``CHUNK_SIZE`` in the inputs JSON against the
value pinned in ``src/storage.rs`` and warns on any mismatch.
"""

from __future__ import annotations

import argparse
import json
import math
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_INPUTS = REPO_ROOT / "docs" / "storage-rent-estimator.inputs.v1.json"
DEFAULT_STORAGE_RS = REPO_ROOT / "src" / "storage.rs"
SUPPORTED_INPUTS_VERSION = 1

_CHUNK_SIZE_RE = re.compile(r"pub\s+const\s+CHUNK_SIZE\s*:\s*u32\s*=\s*(\d+)")


def parse_storage_chunk_size(storage_rs: Path) -> int | None:
    """Return the ``CHUNK_SIZE`` constant from ``storage.rs``, or ``None``."""
    match = _CHUNK_SIZE_RE.search(storage_rs.read_text())
    return int(match.group(1)) if match else None


def estimate(inputs: dict, n: int, roles: int, lastact: int) -> dict:
    """Pure function: entry counts for ``n`` contributors.

    ``roles`` is the number of role holders (independent of ``n``); ``lastact``
    is the number of contributors with a cooldown timestamp entry (``0..n``).
    """
    chunk_size = int(inputs["on_chain"]["layout"]["chunk_size"])
    instance_keys = inputs["on_chain"]["layout"]["instance_keys"]

    reg_entries = n
    chunk_entries = math.ceil(n / chunk_size) if n > 0 else 0
    lastact_entries = min(max(lastact, 0), n)
    role_entries = max(roles, 0)
    persistent_total = reg_entries + chunk_entries + lastact_entries + role_entries

    return {
        "users": n,
        "chunk_size": chunk_size,
        "reg_entries": reg_entries,
        "chunk_entries": chunk_entries,
        "lastact_entries": lastact_entries,
        "role_entries": role_entries,
        "persistent_entry_total": persistent_total,
        "instance_entry_count": len(instance_keys),
    }


def format_report(result: dict, mismatch: tuple[int, int] | None) -> str:
    lines = [
        "TrustBridge on-chain storage rent estimate",
        "=========================================",
        f"contributors (N)          : {result['users']}",
        f"chunk size                : {result['chunk_size']}",
        "",
        f"reg entries (N)           : {result['reg_entries']}",
        f"index chunks ceil(N/CS)   : {result['chunk_entries']}",
        f"lastact entries (0..N)    : {result['lastact_entries']}",
        f"role entries (R)          : {result['role_entries']}",
        f"persistent entry total    : {result['persistent_entry_total']}",
        f"instance entries (fixed)  : {result['instance_entry_count']}",
        "",
        "XLM conversion needs operator-supplied network_rent_params; not computed here.",
    ]
    if mismatch is not None:
        json_cs, code_cs = mismatch
        lines.append("")
        lines.append(
            f"WARNING: chunk_size in inputs JSON ({json_cs}) != CHUNK_SIZE in "
            f"src/storage.rs ({code_cs}); regenerate the inputs JSON."
        )
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--users", "-n", type=int, required=True, help="live contributor count N")
    parser.add_argument("--roles", "-r", type=int, default=0, help="role holders R (default 0)")
    parser.add_argument(
        "--lastact", type=int, default=0, help="contributors with a lastact entry, 0..N (default 0)"
    )
    parser.add_argument("--inputs", type=Path, default=DEFAULT_INPUTS, help="estimator inputs JSON")
    parser.add_argument(
        "--storage-rs", type=Path, default=DEFAULT_STORAGE_RS, help="path to src/storage.rs"
    )
    parser.add_argument("--json", action="store_true", help="emit JSON instead of a text report")
    args = parser.parse_args()

    if args.users < 0:
        parser.error("--users must be >= 0")

    inputs = json.loads(args.inputs.read_text())
    version = inputs.get("estimator_inputs_version")
    if version != SUPPORTED_INPUTS_VERSION:
        parser.error(
            f"unsupported estimator_inputs_version {version!r}; expected {SUPPORTED_INPUTS_VERSION}"
        )

    result = estimate(inputs, args.users, args.roles, args.lastact)

    mismatch = None
    code_chunk_size = parse_storage_chunk_size(args.storage_rs) if args.storage_rs.exists() else None
    if code_chunk_size is not None and code_chunk_size != result["chunk_size"]:
        mismatch = (result["chunk_size"], code_chunk_size)

    if args.json:
        payload = dict(result)
        payload["chunk_size_matches_storage_rs"] = mismatch is None
        if mismatch is not None:
            payload["storage_rs_chunk_size"] = mismatch[1]
        print(json.dumps(payload, indent=2))
    else:
        print(format_report(result, mismatch))

    if mismatch is not None:
        print(
            f"WARNING: chunk_size mismatch (inputs={mismatch[0]}, storage.rs={mismatch[1]})",
            file=sys.stderr,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
