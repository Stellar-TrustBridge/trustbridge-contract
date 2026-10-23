#!/usr/bin/env python3
"""Issue #600: assert subgraph entity or explicit waiver for every contract topic."""
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parent.parent
events = (ROOT / "src/events.rs").read_text()
schema = (ROOT / "docs/subgraph/schema.graphql").read_text()
readme = (ROOT / "docs/subgraph/README.md").read_text()
indexer = (ROOT / "scripts/event_indexer.sh").read_text()

def topic(name):
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()

def check_unique(values, label, errors):
    if len(values) != len(set(values)):
        errors.append(f"duplicate {label}: " + ", ".join(sorted(v for v in values if values.count(v) > 1)))

errors = []
# Match only actual declarations preceded by the macro, not comments.
contract_names = re.findall(
    r"#\[contractevent\]\s*(?:#\[[^\n]+\]\s*)*pub struct (\w+)\b", events
)
contract_topics = [topic(name) for name in contract_names]
check_unique(contract_topics, "contract topics", errors)
if not contract_topics:
    errors.append("no contractevent declarations found")

# Keep in step with check_event_topics.sh, which reads TOPIC_TABLE in the indexer.
table_match = re.search(
    r'^TOPIC_TABLE="\$\(cat <<\x27TOPICS\x27\n(.*?)^TOPICS\n',
    indexer, flags=re.MULTILINE | re.DOTALL,
)
if not table_match:
    errors.append("cannot locate TOPIC_TABLE in scripts/event_indexer.sh")
    indexer_topics = []
else:
    indexer_topics = [
        line.split("|", 1)[0]
        for line in table_match.group(1).splitlines()
        if re.match(r"^[a-z][a-z0-9_]*\|", line)
    ]
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
    r"^\| Contract event \| Topic symbol \|[^\n]*\n"
    r"\|[-| ]+\|\n(?P<rows>(?:\|[^\n]+\n)*)",
    readme, flags=re.MULTILINE,
)
mappings = {}
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
    r".*?^\| Waived topic \| Reason \|\n\|[-| ]+\|\n"
    r"(?P<rows>(?:\|[^\n]+\n)*)",
    readme, flags=re.MULTILINE | re.DOTALL,
)
waivers = {}
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

if errors:
    for message in errors:
        print("subgraph-parity: FAIL: " + message, file=sys.stderr)
    sys.exit(1)

print(f"subgraph-parity: PASS: {len(contract_topics)} contract topics, "
      f"{len(mappings)} Graph mappings, {len(waivers)} explicit waivers")
