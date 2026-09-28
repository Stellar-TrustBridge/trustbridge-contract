#!/usr/bin/env python3
"""Generate the machine-readable ABI summary from docs/ABI.md."""

import argparse
import difflib
import json
import re
import sys
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


def build_document(lines):
    return {
        "schema_version": 1,
        "source": str(ABI_PATH),
        "functions": parse_functions(lines),
        "errors": parse_errors(lines),
        "events": parse_events(lines),
    }


def render(document):
    return json.dumps(document, indent=2, ensure_ascii=True) + "\n"


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="fail with a diff (exit 1) if docs/abi.json is stale vs docs/ABI.md, without writing",
    )
    args = parser.parse_args(argv)

    lines = ABI_PATH.read_text(encoding="utf-8").splitlines()
    rendered = render(build_document(lines))
    if args.check:
        committed = OUTPUT_PATH.read_text(encoding="utf-8") if OUTPUT_PATH.exists() else ""
        if committed == rendered:
            print("abi-check: OK — docs/abi.json matches docs/ABI.md")
            return 0
        diff = "".join(
            difflib.unified_diff(
                committed.splitlines(keepends=True),
                rendered.splitlines(keepends=True),
                fromfile="docs/abi.json (committed)",
                tofile="docs/abi.json (generated)",
            )
        )
        print("abi-check: FAILED — docs/abi.json is stale vs docs/ABI.md.", file=sys.stderr)
        print("Run `make abi` and commit the regenerated docs/abi.json.", file=sys.stderr)
        print(diff, file=sys.stderr)
        return 1
    OUTPUT_PATH.write_text(rendered, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
