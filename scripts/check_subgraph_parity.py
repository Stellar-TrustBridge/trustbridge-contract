#!/usr/bin/env python3
"""Event-topic parity across the contract, indexer, docs and subgraph spec.

Issue #600: every contract topic has exactly one Graph mapping or explicit waiver
in docs/subgraph/README.md, and the Graph schema agrees.

Issue #754: every contract topic is also listed in the docs/EVENT_INDEXING.md
"Topic Classification" table with the same `event_kind` / `category` as the
indexer's TOPIC_TABLE, and a Subgraph column that matches the README
(`` `EntityName` `` for a mapped topic, `waived` for a waived one).

Usage:
  python3 scripts/check_subgraph_parity.py              # check the repo (CI)
  python3 scripts/check_subgraph_parity.py --self-test  # prove the failure paths fire
"""
from __future__ import annotations

from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parent.parent
SOURCES = {
    "events": "src/events.rs",
    "schema": "docs/subgraph/schema.graphql",
    "readme": "docs/subgraph/README.md",
    "indexer": "scripts/event_indexer.sh",
    "indexing": "docs/EVENT_INDEXING.md",
}

# Header of the EVENT_INDEXING.md classification table (Issue #754).
INDEXING_HEADER = "| Topic symbol | `event_kind` | `category` | Subgraph |"


def topic(name: str) -> str:
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


def check_unique(values, label, errors):
    if len(values) != len(set(values)):
        errors.append(f"duplicate {label}: " + ", ".join(sorted({v for v in values if values.count(v) > 1})))


def collect_errors(texts: dict[str, str]) -> tuple[list[str], dict[str, int]]:
    events, schema, readme = texts["events"], texts["schema"], texts["readme"]
    indexer, indexing = texts["indexer"], texts["indexing"]
    errors: list[str] = []

    # Match only actual declarations preceded by the macro, not comments.
    contract_names = re.findall(r"#\[contractevent\]\s*(?:#\[[^\n]+\]\s*)*pub struct (\w+)\b", events)
    contract_topics = [topic(name) for name in contract_names]
    check_unique(contract_topics, "contract topics", errors)
    if not contract_topics:
        errors.append("no contractevent declarations found")

    # Keep in step with check_event_topics.sh, which reads TOPIC_TABLE in the indexer.
    table_match = re.search(
        r'^TOPIC_TABLE="\$\(cat <<\x27TOPICS\x27\n(.*?)^TOPICS\n', indexer, flags=re.MULTILINE | re.DOTALL
    )
    indexer_rows: dict[str, tuple[str, str]] = {}
    if not table_match:
        errors.append("cannot locate TOPIC_TABLE in scripts/event_indexer.sh")
        indexer_topics: list[str] = []
    else:
        indexer_topics = []
        for line in table_match.group(1).splitlines():
            if re.match(r"^[a-z][a-z0-9_]*\|", line):
                parts = line.split("|")
                indexer_topics.append(parts[0])
                indexer_rows[parts[0]] = (parts[1] if len(parts) > 1 else "", parts[2] if len(parts) > 2 else "")
    check_unique(indexer_topics, "indexer topics", errors)
    if set(contract_topics) != set(indexer_topics):
        errors.append(
            "src/events.rs and indexer topic table disagree: missing="
            + str(sorted(set(contract_topics) - set(indexer_topics)))
            + ", stale=" + str(sorted(set(indexer_topics) - set(contract_topics)))
        )

    entity_names = re.findall(r"^type (\w+) @entity\b", schema, flags=re.MULTILINE)
    check_unique(entity_names, "Graph @entity names", errors)
    entity_topics = {topic(name): name for name in entity_names if name.endswith("Event")}

    # Read only the exact Event -> entity mapping table (not narrative backticks).
    mapping_block = re.search(
        r"^\| Contract event \| Topic symbol \|[^\n]*\n\|[-| ]+\|\n(?P<rows>(?:\|[^\n]+\n)*)",
        readme, flags=re.MULTILINE,
    )
    mappings: dict[str, tuple[str, str]] = {}
    if not mapping_block:
        errors.append("cannot locate Event -> entity mapping table in subgraph README")
    else:
        for row in mapping_block.group("rows").splitlines():
            cols = [c.strip().strip("`") for c in row.strip("|").split("|")]
            if len(cols) != 4:
                errors.append(f"malformed mapping row: {row}")
                continue
            event_name, event_topic, _, entity_name = cols
            if event_topic in mappings:
                errors.append(f"duplicate mapping for {event_topic}")
            mappings[event_topic] = (event_name, entity_name)
            if event_topic != topic(event_name):
                errors.append(f"mapping topic disagrees with event name: {event_topic}")
            if entity_name not in entity_names:
                errors.append(f"mapped Graph entity absent: {entity_name}")

    waiver_block = re.search(
        r"^## Explicit subgraph event waivers \(Issue #600\)\n"
        r".*?^\| Waived topic \| Reason \|\n\|[-| ]+\|\n(?P<rows>(?:\|[^\n]+\n)*)",
        readme, flags=re.MULTILINE | re.DOTALL,
    )
    waivers: dict[str, str] = {}
    if not waiver_block:
        errors.append("missing explicit waiver table in subgraph README")
    else:
        for row in waiver_block.group("rows").splitlines():
            cells = [v.strip().strip("`") for v in row.strip("|").split("|")]
            if len(cells) != 2 or not cells[1]:
                errors.append(f"waiver requires topic and non-empty reason: {row}")
                continue
            name, reason = cells
            if name in waivers:
                errors.append(f"duplicate waiver for {name}")
            waivers[name] = reason

    for name in contract_topics:
        mapped = name in mappings and name in entity_topics
        waived = name in waivers
        if mapped == waived:
            errors.append(f"{name}: expected exactly one Graph mapping or waiver")
        if name in mappings and name not in entity_topics:
            errors.append(f"{name}: mapping does not match a Graph @entity")
        if name in entity_topics and name not in mappings:
            errors.append(f"{name}: Graph entity requires a README mapping row")

    for name in sorted(set(mappings) | set(waivers) | set(entity_topics)):
        if name not in contract_topics:
            errors.append(f"stale mapping/entity/waiver topic: {name}")

    for name, (event_name, entity_name) in mappings.items():
        if entity_name != event_name:
            errors.append(f"{name}: entity {entity_name} differs from event {event_name}")

    # ── Issue #754: docs/EVENT_INDEXING.md classification table ──────────────
    indexing_rows: dict[str, tuple[str, str, str]] = {}
    start = indexing.find(INDEXING_HEADER)
    if start < 0:
        errors.append(f"EVENT_INDEXING.md: cannot locate topic table header `{INDEXING_HEADER}`")
    else:
        lines = indexing[start:].splitlines()[2:]  # skip header + separator
        for row in lines:
            if not row.startswith("|"):
                break
            cells = [c.strip() for c in row.strip("|").split("|")]
            if len(cells) != 4:
                errors.append(f"EVENT_INDEXING.md: malformed topic row: {row}")
                continue
            sym, kind, cat, subgraph = (cells[0].strip("`"), cells[1].strip("`"), cells[2].strip("`"), cells[3])
            if sym in indexing_rows:
                errors.append(f"EVENT_INDEXING.md: duplicate topic row: {sym}")
            indexing_rows[sym] = (kind, cat, subgraph)

        for name in contract_topics:
            if name not in indexing_rows:
                errors.append(f"EVENT_INDEXING.md: {name} is emitted by src/events.rs but missing from the topic table")
        for name in sorted(set(indexing_rows) - set(contract_topics)):
            errors.append(f"EVENT_INDEXING.md: stale topic row (not emitted by src/events.rs): {name}")

        for name, (kind, cat, subgraph) in indexing_rows.items():
            if name in indexer_rows and indexer_rows[name] != (kind, cat):
                errors.append(
                    f"EVENT_INDEXING.md: {name} classified as {kind}/{cat} but TOPIC_TABLE says "
                    f"{indexer_rows[name][0]}/{indexer_rows[name][1]}"
                )
            if name in mappings:
                expected = f"`{mappings[name][1]}`"
                if subgraph != expected:
                    errors.append(f"EVENT_INDEXING.md: {name} Subgraph column must be {expected} (mapped entity), got {subgraph!r}")
            elif name in waivers:
                if subgraph != "waived":
                    errors.append(f"EVENT_INDEXING.md: {name} Subgraph column must be 'waived' (README waiver), got {subgraph!r}")

    stats = {"topics": len(contract_topics), "mappings": len(mappings), "waivers": len(waivers),
             "indexing_rows": len(indexing_rows)}
    return errors, stats


def load_sources() -> dict[str, str]:
    return {key: (ROOT / path).read_text() for key, path in SOURCES.items()}


def self_test() -> int:
    """Happy path on the repo, plus mutated copies that must each fail."""
    base = load_sources()
    errors, _ = collect_errors(base)
    assert not errors, f"self-test: repo should pass, got {errors}"

    def mutated(fn) -> list[str]:
        texts = dict(base)
        texts["indexing"] = fn(texts["indexing"])
        return collect_errors(texts)[0]

    cases = {
        "missing row": (
            lambda t: re.sub(r"^\| `paused_event` \|[^\n]*\n", "", t, flags=re.MULTILINE),
            "paused_event is emitted by src/events.rs but missing from the topic table",
        ),
        "wrong subgraph column": (
            lambda t: t.replace("| `registered` | `registry` | `RegisteredEvent` |", "| `registered` | `registry` | waived |"),
            "registered_event Subgraph column must be `RegisteredEvent`",
        ),
        "waived topic shown as mapped": (
            lambda t: t.replace("| `paused` | `admin` | waived |", "| `paused` | `admin` | `PausedEvent` |"),
            "paused_event Subgraph column must be 'waived'",
        ),
        "category drift": (
            lambda t: t.replace("| `paused` | `admin` |", "| `paused` | `registry` |"),
            "paused_event classified as paused/registry",
        ),
        "stale row": (
            lambda t: t.replace(INDEXING_HEADER + "\n|---|---|---|---|\n",
                                INDEXING_HEADER + "\n|---|---|---|---|\n| `ghost_event` | `ghost` | `admin` | waived |\n"),
            "stale topic row (not emitted by src/events.rs): ghost_event",
        ),
        "missing table": (lambda t: t.replace(INDEXING_HEADER, "| Topic | Kind |"), "cannot locate topic table header"),
    }
    for label, (fn, needle) in cases.items():
        errs = mutated(fn)
        assert any(needle in e for e in errs), f"self-test '{label}': expected error containing {needle!r}, got {errs}"
    print(f"subgraph-parity: SELF-TEST PASS: repo passes and {len(cases)} drift cases are rejected")
    return 0


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        return self_test()
    errors, stats = collect_errors(load_sources())
    if errors:
        for message in errors:
            print("subgraph-parity: FAIL: " + message, file=sys.stderr)
        return 1
    print(f"subgraph-parity: PASS: {stats['topics']} contract topics, "
          f"{stats['mappings']} Graph mappings, {stats['waivers']} explicit waivers, "
          f"{stats['indexing_rows']} EVENT_INDEXING.md rows")
    return 0


if __name__ == "__main__":
    sys.exit(main())
