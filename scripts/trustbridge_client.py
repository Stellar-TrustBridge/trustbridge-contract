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


class StellarCLIError(RuntimeError):
    """A Stellar CLI invocation failed."""

    def __init__(self, command: Sequence[str], returncode: int, stderr: str) -> None:
        self.command = tuple(command)
        self.returncode = returncode
        self.stderr = stderr.strip()
        detail = self.stderr or "no error output"
        super().__init__(f"stellar CLI exited with {returncode}: {detail}")


@dataclass(frozen=True)
class RegistryRecord:
    github_username: str
    stellar_address: str
    verified: bool
    registered_at: int


@dataclass(frozen=True)
class RegistryPage:
    records: list[RegistryRecord]
    next_cursor: int | None
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
        completed = subprocess.run(command, text=True, capture_output=True, check=False)
        if completed.returncode:
            raise StellarCLIError(command, completed.returncode, completed.stderr or completed.stdout)
        try:
            return json.loads(completed.stdout)
        except json.JSONDecodeError as exc:
            raise StellarCLIError(command, 0, f"invalid JSON response: {exc}: {completed.stdout!r}") from exc

    @staticmethod
    def _record(username: str, value: dict[str, Any]) -> RegistryRecord:
        return RegistryRecord(
            github_username=username,
            stellar_address=value["stellar_address"],
            verified=bool(value["verified"]),
            registered_at=int(value["registered_at"]),
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

    def batch_verify(self, usernames: Sequence[str]) -> int:
        value = self._invoke("batch_verify", ("--caller", self.source, "--usernames", json.dumps(list(usernames))), send=True)
        return int(value["successful"] if isinstance(value, dict) else value)

    def batch_remove(self, usernames: Sequence[str]) -> int:
        value = self._invoke("batch_remove", ("--caller", self.source, "--usernames", json.dumps(list(usernames))), send=True)
        return int(value["successful"] if isinstance(value, dict) else value)

    def extend_registry_ttl(self, usernames: Sequence[str]) -> int:
        value = self._invoke("extend_registry_ttl", ("--usernames", json.dumps(list(usernames))), send=True)
        return int(value)
