"""Small typed client for TrustBridge operator scripts.

The client deliberately shells out to the pinned Stellar CLI instead of
embedding an RPC implementation. CLI failures are raised as structured
exceptions and successful return values are decoded as JSON.
"""

from __future__ import annotations

import json
import os
import subprocess
from dataclasses import dataclass
from typing import Any, Sequence

#: Exit status contract for operator scripts built on this client:
#: ``0`` on success, non-zero on any failure. Argument/config errors exit
#: via ``argparse`` (status 2); CLI/RPC failures, malformed responses, and
#: pagination stalls exit with status 1 after printing an actionable
#: ``ERROR:`` line to stderr.
EXIT_OK = 0
EXIT_OPERATOR_ERROR = 1

#: Common Stellar CLI / RPC failure substrings mapped to operator actions.
#: Matched case-insensitively against CLI stderr; the first match wins.
#: Keep needles specific enough to avoid mislabelling unrelated output.
ERROR_HINTS: tuple[tuple[str, str], ...] = (
    (
        "failed to connect",
        "HINT: RPC endpoint unreachable — verify --network/NETWORK and RPC connectivity, then retry; simulate first without --send=yes.",
    ),
    (
        "connection refused",
        "HINT: RPC endpoint refused the connection — verify --network/NETWORK and RPC connectivity, then retry.",
    ),
    (
        "connection reset",
        "HINT: RPC connection dropped mid-call — retry the read; if persistent, switch RPC endpoint and re-run.",
    ),
    (
        "timed out",
        "HINT: RPC request timed out — retry with backoff; for bulk walks, lower --page-limit and add pacing between calls.",
    ),
    (
        "timeout",
        "HINT: RPC request timed out — retry with backoff; for bulk walks, lower --page-limit and add pacing between calls.",
    ),
    (
        "name resolution",
        "HINT: DNS failure resolving the RPC host — check network connectivity and the --network/NETWORK value, then retry.",
    ),
    (
        "network is unreachable",
        "HINT: Network unreachable — check connectivity and the --network/NETWORK value, then retry.",
    ),
    (
        "contract not found",
        "HINT: Contract ID not found on this network — verify CONTRACT_ID and that --network matches the deployment.",
    ),
    (
        "contract does not exist",
        "HINT: Contract ID not found on this network — verify CONTRACT_ID and that --network matches the deployment.",
    ),
    (
        "account not found",
        "HINT: Signing account missing on this network — fund SOURCE on --network or pick the funded identity for that network.",
    ),
    (
        "unfunded",
        "HINT: Signing account is unfunded — fund SOURCE on --network before submitting.",
    ),
    (
        "insufficient",
        "HINT: Insufficient balance for fees — fund SOURCE on --network before submitting.",
    ),
    (
        "require_auth",
        "HINT: Auth failed — sign with the identity the contract checks (--caller must equal the signing --source-account for admin-gated calls).",
    ),
    (
        "not authorized",
        "HINT: Caller lacks permission — sign as admin (or the required role holder) and pass the same identity as --caller.",
    ),
    (
        "signature",
        "HINT: Signature/auth rejected — sign with the expected identity and keep --caller identical to the signer.",
    ),
    (
        "paused",
        "HINT: Contract is paused — reads still work; for writes, confirm pause state with `is_paused` / `is_emergency_paused` before retrying after unpause.",
    ),
    (
        "rate limit",
        "HINT: RPC rate-limited the call — retry with backoff and slower pacing between pages.",
    ),
    (
        "overloaded",
        "HINT: RPC endpoint overloaded — retry with backoff; lower --page-limit if paging a large registry.",
    ),
    (
        "ledger",
        "HINT: Ledger/RPC state issue — retry the read; if the cursor stalls repeatedly, re-run the export from cursor 0.",
    ),
)


def actionable_hint(stderr: str) -> str | None:
    """Return the operator hint for known CLI/RPC failure text, if any."""
    lowered = stderr.lower()
    for needle, hint in ERROR_HINTS:
        if needle in lowered:
            return hint
    return None


class StellarCLIError(RuntimeError):
    """A Stellar CLI invocation failed."""

    def __init__(
        self,
        command: Sequence[str],
        returncode: int,
        stderr: str,
        hint: str | None = None,
    ) -> None:
        self.command = tuple(command)
        self.returncode = returncode
        self.stderr = stderr.strip()
        self.hint = hint if hint is not None else actionable_hint(self.stderr)
        detail = self.stderr or "no error output"
        message = f"stellar CLI exited with {returncode}: {detail}"
        if self.hint:
            message += f"\n{self.hint}"
        super().__init__(message)


@dataclass(frozen=True)
class RegistryRecord:
    github_username: str
    stellar_address: str
    verified: bool
    registered_at: int
    payout_address: str = ""
    is_bot: bool = False


@dataclass(frozen=True)
class RegistryPage:
    records: list[RegistryRecord]
    next_cursor: object | None
    total: int
    has_more: bool


class TrustBridgeClient:
    """Typed operator client backed by ``stellar contract invoke``."""

    def __init__(
        self,
        contract_id: str,
        source: str,
        network: str = "testnet",
        stellar: str | None = None,
    ) -> None:
        if not contract_id:
            raise ValueError("contract_id is required")
        if not source:
            raise ValueError("source is required")
        self.contract_id = contract_id
        self.source = source
        self.network = network
        self.stellar = stellar or os.environ.get("STELLAR", "stellar")

    def _invoke(self, method: str, args: Sequence[str] = (), send: bool = False) -> Any:
        command = [
            self.stellar,
            "contract",
            "invoke",
            "--id",
            self.contract_id,
            "--source-account",
            self.source,
            "--network",
            self.network,
        ]
        if send:
            command.append("--send=yes")
        command.extend(["--", method, *args])
        try:
            completed = subprocess.run(command, text=True, capture_output=True, check=False)
        except FileNotFoundError as exc:
            raise StellarCLIError(
                command,
                127,
                f"stellar CLI not found ({self.stellar}): {exc}",
                hint=f"HINT: Stellar CLI binary '{self.stellar}' is missing — install stellar-cli >= 26.x "
                "and ensure it is on PATH, or set STELLAR=<path-to-binary>.",
            ) from exc
        except OSError as exc:
            raise StellarCLIError(
                command,
                127,
                f"failed to execute stellar CLI ({self.stellar}): {exc}",
                hint=f"HINT: Could not execute '{self.stellar}' — check the STELLAR env var and file permissions, then retry.",
            ) from exc
        if completed.returncode:
            raise StellarCLIError(command, completed.returncode, completed.stderr or completed.stdout)
        try:
            return json.loads(completed.stdout)
        except json.JSONDecodeError as exc:
            raise StellarCLIError(
                command,
                0,
                f"invalid JSON response: {exc}: {completed.stdout!r}",
                hint="HINT: The CLI printed non-JSON output — check the Stellar CLI version (>= 26.x) and "
                "re-run with `-- <fn> --help` to confirm the deployed contract exposes this method.",
            ) from exc

    @staticmethod
    def _record(username: str, value: dict[str, Any]) -> RegistryRecord:
        return RegistryRecord(
            github_username=username,
            stellar_address=value["stellar_address"],
            verified=bool(value["verified"]),
            registered_at=int(value["registered_at"]),
            payout_address=value.get("payout_address") or value["stellar_address"],
            is_bot=bool(value.get("is_bot", False)),
        )

    def get_address(self, username: str) -> RegistryRecord | None:
        value = self._invoke("get_address", ("--github-username", username))
        if value is None:
            return None
        if not isinstance(value, dict):
            raise ValueError(f"get_address returned unexpected value: {value!r}")
        return self._record(username, value)

    def get_stats(self) -> dict[str, int]:
        value = self._invoke("get_stats")
        if not isinstance(value, dict):
            raise ValueError(f"get_stats returned unexpected value: {value!r}")
        return {key: int(value[key]) for key in ("total", "verified")}

    def get_registered_page(self, cursor: int = 0, limit: int = 100) -> RegistryPage:
        value = self._invoke("get_registered_paginated", ("--cursor", str(cursor), "--limit", str(limit)))
        if not isinstance(value, dict):
            raise ValueError(f"get_registered_paginated returned unexpected value: {value!r}")
        records = [self._record(item[0], item[1]) for item in value["records"]]
        next_cursor = value.get("next_cursor")
        return RegistryPage(
            records=records,
            next_cursor=None if next_cursor is None else int(next_cursor),
            total=int(value["total"]),
            has_more=bool(value["has_more"]),
        )

    def get_public_page(self, cursor: object = 0, limit: int = 100) -> RegistryPage:
        """One page of the unauthenticated ``get_public_paginated`` read.

        ``next_cursor`` is an opaque token — it is passed straight back to the
        next call and never interpreted here.
        """
        value = self._invoke("get_public_paginated", ("--cursor", str(cursor), "--limit", str(limit)))
        if not isinstance(value, dict):
            raise ValueError(f"get_public_paginated returned unexpected value: {value!r}")
        records = [self._record(item[0], item[1]) for item in value["records"]]
        next_cursor = value.get("next_cursor")
        return RegistryPage(
            records=records,
            next_cursor=None if next_cursor is None else next_cursor,
            total=int(value["total"]),
            has_more=bool(value["has_more"]),
        )

    def iter_public_records(self, page_limit: int = 100):
        """Yield every registry record via ``get_public_paginated``, guarding
        against a stalled cursor echoed by an unreliable RPC node."""
        cursor: object = 0
        seen: set[str] = set()
        while True:
            page = self.get_public_page(cursor, page_limit)
            yield from page.records
            if not page.has_more or page.next_cursor is None:
                return
            token = str(page.next_cursor)
            if token in seen:
                raise RuntimeError(f"pagination stalled at cursor {token}")
            seen.add(token)
            cursor = page.next_cursor

    def batch_verify(self, usernames: Sequence[str]) -> int:
        value = self._invoke("batch_verify", ("--caller", self.source, "--usernames", json.dumps(list(usernames))), send=True)
        return int(value["successful"] if isinstance(value, dict) else value)

    def batch_remove(self, usernames: Sequence[str]) -> int:
        value = self._invoke("batch_remove", ("--caller", self.source, "--usernames", json.dumps(list(usernames))), send=True)
        return int(value["successful"] if isinstance(value, dict) else value)

    def extend_registry_ttl(self, usernames: Sequence[str]) -> int:
        value = self._invoke("extend_registry_ttl", ("--usernames", json.dumps(list(usernames))), send=True)
        return int(value)
