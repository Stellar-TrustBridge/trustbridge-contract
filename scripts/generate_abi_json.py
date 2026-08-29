#!/usr/bin/env python3
"""Generate the machine-readable ABI summary from docs/ABI.md."""

import json
import re
from pathlib import Path

ABI_PATH = Path("docs/ABI.md")
OUTPUT_PATH = Path("docs/abi.json")


def parse_functions(lines):
    functions = []
    seen = set()
    in_functions = False
    for line in lines:
        if line == "## Functions":
            in_functions = True
            continue
        if in_functions and line.startswith("## "):
            break
        match = re.match(r"^### `([^`]+)`$", line)
        if match and "(" in match.group(1) and " -> " in match.group(1):
            signature = match.group(1)
            name = signature.split("(", 1)[0]
            if signature not in seen:
                functions.append({"name": name, "signature": signature})
                seen.add(signature)
    return functions


def parse_errors(lines):
    errors = []
    in_errors = False
    for line in lines:
        if line == "### ContractError (u32 discriminant)":
            in_errors = True
            continue
        if in_errors and line.startswith("### "):
            break
        match = re.match(r"^\| ([0-9]+) \| `([^`]+)` \| (.+) \|$", line)
        if match:
            errors.append(
                {
                    "code": int(match.group(1)),
                    "name": match.group(2),
                    "description": match.group(3),
                }
            )
    return errors


def parse_events(lines):
    events = []
    in_events = False
    current = None
    code_lines = []

    def finish_event():
        nonlocal current, code_lines
        if current is None:
            return
        code = " ".join(line.strip() for line in code_lines).strip()
        topics = []
        data = None
        topics_match = re.search(r'topics:\s*(\[[^]]*\])', code)
        data_match = re.search(r"data:\s*(\{[^}]*\})", code)
        if topics_match:
            topics = [item.strip() for item in topics_match.group(1)[1:-1].split(",")]
            topics = [item for item in topics if item]
        if data_match:
            data = data_match.group(1)
        for name in re.findall(r"[A-Z][A-Za-z0-9]*Event", current):
            events.append({"name": name, "topics": topics, "data": data})
        current = None
        code_lines = []

    for line in lines:
        if line == "## Events":
            in_events = True
            continue
        if in_events and line.startswith("## "):
            finish_event()
            break
        if not in_events:
            continue
        heading = re.match(r"^### (.+)$", line)
        if heading:
            finish_event()
            current = heading.group(1)
            continue
        if current is not None and line.startswith("topics:"):
            code_lines.append(line)
        elif current is not None and line.startswith("data:"):
            code_lines.append(line)
    finish_event()
    return events


def main():
    lines = ABI_PATH.read_text(encoding="utf-8").splitlines()
    document = {
        "schema_version": 1,
        "source": str(ABI_PATH),
        "functions": parse_functions(lines),
        "errors": parse_errors(lines),
        "events": parse_events(lines),
    }
    OUTPUT_PATH.write_text(
        json.dumps(document, indent=2, ensure_ascii=True) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    main()
