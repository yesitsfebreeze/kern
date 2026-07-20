#!/usr/bin/env python3
"""Verify the docs' references are alive: every `src/...:line` citation points
at a real file and an existing line, every relative page link points at a page
that exists, and every link into this repo's own files on GitHub names a file
that is actually committed. Exit 1 with a report if anything is dead."""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONTENT = ROOT / "docs" / "site" / "content"
REF = re.compile(r"`(src/[A-Za-z0-9_/.-]+\.rs)(?::(\d+))?`")
LINK = re.compile(r"\]\((\.\.?/[^)\s#]+\.mdx)(?:#[^)\s]*)?\)")
SELF_URL = re.compile(
    r"https://(?:raw\.githubusercontent\.com/yesitsfebreeze/kern"
    r"|github\.com/yesitsfebreeze/kern/(?:blob|raw))/master/([^)\s#\"']+)"
)

line_counts: dict[Path, int] = {}


def lines_of(path: Path) -> int:
    if path not in line_counts:
        line_counts[path] = sum(1 for _ in path.open(encoding="utf-8", errors="replace"))
    return line_counts[path]


def main() -> int:
    failures = []
    total = 0
    pages = sorted(CONTENT.rglob("*.mdx")) + [ROOT / "README.md"]
    for page in pages:
        rel = page.relative_to(ROOT)
        for lineno, text in enumerate(page.read_text(encoding="utf-8").splitlines(), 1):
            for m in REF.finditer(text):
                total += 1
                target = ROOT / m.group(1)
                if not target.is_file():
                    failures.append(f"{rel}:{lineno}: missing file {m.group(1)}")
                elif m.group(2) and int(m.group(2)) > lines_of(target):
                    failures.append(
                        f"{rel}:{lineno}: {m.group(1)}:{m.group(2)} beyond EOF "
                        f"({lines_of(target)} lines)"
                    )
            for m in LINK.finditer(text):
                total += 1
                if not (page.parent / m.group(1)).resolve().is_file():
                    failures.append(f"{rel}:{lineno}: dead page link {m.group(1)}")
            for m in SELF_URL.finditer(text):
                total += 1
                if not (ROOT / m.group(1)).is_file():
                    failures.append(f"{rel}:{lineno}: dead self link {m.group(1)}")
    if failures:
        print(f"{len(failures)}/{total} dead references:")
        print("\n".join(failures))
        return 1
    print(f"docs-check: {total} references OK")
    return 0


def selftest() -> None:
    assert REF.findall("see `src/base/merge.rs:20` and `src/crdt.rs`") == [
        ("src/base/merge.rs", "20"),
        ("src/crdt.rs", ""),
    ]
    assert REF.findall("src/base/merge.rs:20 unquoted") == []
    assert LINK.findall("[a](./federation.mdx) [b](../howto/mcp.mdx#gossip)") == [
        "./federation.mdx",
        "../howto/mcp.mdx",
    ]
    assert LINK.findall("[x](https://example.com/a.mdx)") == []
    assert SELF_URL.findall(
        "curl https://raw.githubusercontent.com/yesitsfebreeze/kern/master/install.sh | sh"
    ) == ["install.sh"]
    assert SELF_URL.findall(
        "[v](https://github.com/yesitsfebreeze/kern/blob/master/docs/oracle/VISION.md)"
    ) == ["docs/oracle/VISION.md"]
    assert SELF_URL.findall("https://github.com/yesitsfebreeze/kern/releases") == []
    print("selftest OK")


if __name__ == "__main__":
    if "--selftest" in sys.argv:
        selftest()
        sys.exit(0)
    sys.exit(main())
