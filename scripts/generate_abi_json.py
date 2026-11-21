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
ENTRYPOINTS_GOLDEN = Path("tests/testdata/abi/public_entrypoints.golden.json")


def is_public_entrypoint(name: str) -> bool:
    """Exclude internals, private helpers, and test-only names from the snapshot."""
    lowered = name.lower()
    if name.startswith("_"):
        return False
    if lowered.startswith("test") or lowered.endswith("_test") or "_test_" in lowered:
        return False
    return True


def arity_from_signature(signature: str) -> int:
    """Count top-level parameters in `name(type, ...) -> ret`."""
    start = signature.find("(")
    if start < 0:
        raise ValueError(f"cannot parse arity from signature: {signature}")
    depth = 0
    end = -1
    for i, ch in enumerate(signature[start:], start):
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                end = i
                break
    if end < 0:
        raise ValueError(f"cannot parse arity from signature: {signature}")
    inner = signature[start + 1 : end].strip()
    if not inner:
        return 0
    depth = 0
    parts = 1
    for ch in inner:
        if ch in "<([":
            depth += 1
        elif ch in ">)]":
            depth = max(0, depth - 1)
        elif ch == "," and depth == 0:
            parts += 1
    return parts


def public_entrypoints(functions):
    items = []
    for fn in functions:
        name = fn["name"]
        if not is_public_entrypoint(name):
            continue
        items.append({"name": name, "arity": arity_from_signature(fn["signature"])})
    items.sort(key=lambda item: (item["name"], item["arity"]))
    return items


def render_entrypoints_golden(functions):
    document = {
        "schema_version": 1,
        "source": str(OUTPUT_PATH),
        "entrypoints": public_entrypoints(functions),
    }
    return json.dumps(document, indent=2, ensure_ascii=True) + "\n"


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
        help="fail with a diff (exit 1) if docs/abi.json or the public-entrypoint golden is stale, without writing",
    )
    parser.add_argument(
        "--update-entrypoints-golden",
        action="store_true",
        help="write tests/testdata/abi/public_entrypoints.golden.json from the generated ABI",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="run snapshot helper unit tests and exit",
    )
    args = parser.parse_args(argv)

    if args.self_test:
        _self_test()
        print("abi-entrypoints self-test: OK")
        return 0

    lines = ABI_PATH.read_text(encoding="utf-8").splitlines()
    document = build_document(lines)
    rendered = render(document)
    golden = render_entrypoints_golden(document["functions"])

    if args.check:
        failed = False
        committed = OUTPUT_PATH.read_text(encoding="utf-8") if OUTPUT_PATH.exists() else ""
        if committed != rendered:
            failed = True
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
        committed_golden = (
            ENTRYPOINTS_GOLDEN.read_text(encoding="utf-8") if ENTRYPOINTS_GOLDEN.exists() else ""
        )
        if committed_golden != golden:
            failed = True
            diff = "".join(
                difflib.unified_diff(
                    committed_golden.splitlines(keepends=True),
                    golden.splitlines(keepends=True),
                    fromfile="tests/testdata/abi/public_entrypoints.golden.json (committed)",
                    tofile="public entrypoints (generated)",
                )
            )
            print(
                "abi-check: FAILED — public entrypoint golden is stale.",
                file=sys.stderr,
            )
            print(
                "Run `python3 scripts/generate_abi_json.py --update-entrypoints-golden` "
                "and commit the snapshot only for an intentional public ABI change.",
                file=sys.stderr,
            )
            print(diff, file=sys.stderr)
        if failed:
            return 1
        print("abi-check: OK — docs/abi.json and public-entrypoint golden match")
        return 0

    OUTPUT_PATH.write_text(rendered, encoding="utf-8")
    if args.update_entrypoints_golden:
        ENTRYPOINTS_GOLDEN.parent.mkdir(parents=True, exist_ok=True)
        ENTRYPOINTS_GOLDEN.write_text(golden, encoding="utf-8")
        print(f"wrote {ENTRYPOINTS_GOLDEN}")
    return 0


def _self_test() -> None:
    assert arity_from_signature("foo() -> u32") == 0
    assert arity_from_signature("bar(a: Address, b: String) -> ()") == 2
    assert arity_from_signature("page(cursor: Option<u32>, limit: Vec<Address>) -> ExportPage") == 2
    shuffled = [
        {"name": "b", "signature": "b(x: u32) -> ()"},
        {"name": "a", "signature": "a() -> ()"},
    ]
    assert public_entrypoints(shuffled) == [
        {"name": "a", "arity": 0},
        {"name": "b", "arity": 1},
    ]
    mixed = shuffled + [
        {"name": "_hidden", "signature": "_hidden() -> ()"},
        {"name": "test_only", "signature": "test_only() -> ()"},
        {"name": "foo_test_bar", "signature": "foo_test_bar() -> ()"},
    ]
    names = [item["name"] for item in public_entrypoints(mixed)]
    assert names == ["a", "b"]
    removed = [item for item in public_entrypoints(shuffled) if item["name"] != "a"]
    assert removed != public_entrypoints(shuffled)
    renamed = [{"name": "aa", "signature": "aa() -> ()"}]
    assert public_entrypoints(renamed) != public_entrypoints(shuffled[:1])
    arity_changed = [{"name": "a", "signature": "a(x: u32) -> ()"}]
    assert public_entrypoints(arity_changed) != public_entrypoints(shuffled[:1])


if __name__ == "__main__":
    raise SystemExit(main())
