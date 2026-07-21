#!/usr/bin/env python3
"""Verify the docs' references are alive: every `src/...:line` citation points
at a real file and an existing line, every backticked repo path names a file
that exists, every relative page link points at a page that exists, and every
link into this repo's own files on GitHub names a file that is actually
committed. Exit 1 with a report if anything is dead.

Scans every documentation directory the repo has — the site, `docs/kern/`,
`docs/oracle/` and `README.md` — because a citation nobody checks is a citation
that rots. What this cannot prove: that a cited line still *says* the thing it
was cited for. Existence only.

Two escapes, because some citations are *supposed* to name a file that is gone.
A page carrying `<!-- docs-check: historical -->` is skipped whole: a changelog
entry recording a deletion must cite the deleted file, and a point-in-time note
describes the tree as it was. A single line naming a deletion is excused in
place, so a present-tense page can still say what it removed."""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PAGE_DIRS = [
    ROOT / "docs" / "site" / "content",
    ROOT / "docs" / "kern",
    ROOT / "docs" / "oracle",
]
REF = re.compile(r"`(src/[A-Za-z0-9_/.-]+\.rs)(?::(\d+)(?:-(\d+))?)?`")
REPO_PATH = re.compile(
    r"`((?:docs|scripts|e2e|\.github|\.pi)/[A-Za-z0-9_/.-]+"
    r"\.(?:md|mdx|py|toml|yml|yaml|sh|json|txt|lock))(?::(\d+)(?:-(\d+))?)?`"
)
# A bare `FILE.md:NNN` with no directory — how ROADMAP.md indexes into FEATURES.md.
SIBLING_REF = re.compile(r"`([A-Z][A-Za-z0-9_.-]*\.mdx?):(\d+)(?:-(\d+))?`")
LINK = re.compile(r"\]\((\.\.?/[^)\s#]+\.mdx?)(?:#[^)\s]*)?\)")
SELF_URL = re.compile(
    r"https://(?:raw\.githubusercontent\.com/yesitsfebreeze/kern"
    r"|github\.com/yesitsfebreeze/kern/(?:blob|raw))/master/([^)\s#\"']+)"
)
HISTORICAL = "<!-- docs-check: historical -->"
GONE = re.compile(r"\b(deleted|removed|withdrawn|absorbed|superseded by)\b", re.I)

line_counts: dict[Path, int] = {}


def lines_of(path: Path) -> int:
    if path not in line_counts:
        line_counts[path] = sum(1 for _ in path.open(encoding="utf-8", errors="replace"))
    return line_counts[path]


def pages() -> list[Path]:
    found = [ROOT / "README.md"]
    for d in PAGE_DIRS:
        found += sorted(d.rglob("*.md")) + sorted(d.rglob("*.mdx"))
    return found


# Standalone line only: a page that merely *describes* the marker must not exempt
# itself, which is how FEATURES.md and ROADMAP.md first went dark.
def is_historical(lines: list[str]) -> bool:
    return any(l.strip() == HISTORICAL for l in lines)


def check_page(page: Path, failures: list[str]) -> int:
    rel = page.relative_to(ROOT)
    lines = page.read_text(encoding="utf-8").splitlines()
    if is_historical(lines):
        return 0
    total = 0
    for lineno, text in enumerate(lines, 1):
        if GONE.search(text):
            continue
        for m in REF.finditer(text):
            total += 1
            target = ROOT / m.group(1)
            cited = m.group(3) or m.group(2)
            if not target.is_file():
                failures.append(f"{rel}:{lineno}: missing file {m.group(1)}")
            elif cited and int(cited) > lines_of(target):
                failures.append(
                    f"{rel}:{lineno}: {m.group(1)}:{cited} beyond EOF "
                    f"({lines_of(target)} lines)"
                )
        for m in REPO_PATH.finditer(text):
            total += 1
            target = ROOT / m.group(1)
            cited = m.group(3) or m.group(2)
            if not target.is_file():
                failures.append(f"{rel}:{lineno}: missing file {m.group(1)}")
            elif cited and int(cited) > lines_of(target):
                failures.append(
                    f"{rel}:{lineno}: {m.group(1)}:{cited} beyond EOF "
                    f"({lines_of(target)} lines)"
                )
        for m in SIBLING_REF.finditer(text):
            total += 1
            # Beside the citing page first, then the repo root — `README.md:159` in
            # docs/oracle/ROADMAP.md means the top-level README.
            target = page.parent / m.group(1)
            if not target.is_file():
                target = ROOT / m.group(1)
            cited = m.group(3) or m.group(2)
            if not target.is_file():
                failures.append(f"{rel}:{lineno}: missing sibling {m.group(1)}")
            elif int(cited) > lines_of(target):
                failures.append(
                    f"{rel}:{lineno}: {m.group(1)}:{cited} beyond EOF "
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
    return total


def main() -> int:
    failures: list[str] = []
    total = sum(check_page(p, failures) for p in pages())
    if failures:
        print(f"{len(failures)}/{total} dead references:")
        print("\n".join(failures))
        return 1
    print(f"docs-check: {total} references exist (existence only — not that they still say it)")
    return 0


def selftest() -> None:
    assert REF.findall("see `src/base/merge.rs:20` and `src/crdt.rs`") == [
        ("src/base/merge.rs", "20", ""),
        ("src/crdt.rs", "", ""),
    ]
    assert REF.findall("`src/base/types.rs:291-296`") == [
        ("src/base/types.rs", "291", "296")
    ]
    assert REF.findall("src/base/merge.rs:20 unquoted") == []
    assert [m[0] for m in REPO_PATH.findall(
        "`docs/kern/vllm.md:17-20` and `scripts/docs_check.py`"
    )] == ["docs/kern/vllm.md", "scripts/docs_check.py"]
    assert SIBLING_REF.findall("see `FEATURES.md:733-736` and `ROADMAP.md:12`") == [
        ("FEATURES.md", "733", "736"),
        ("ROADMAP.md", "12", ""),
    ]
    assert SIBLING_REF.findall("`FEATURES.md`") == [], "a bare name has no line to check"
    assert REPO_PATH.findall("`docs/site/out/`") == [], "a directory is not a citation"
    assert REPO_PATH.findall("`src/base/merge.rs`") == [], "src is REF's job"
    assert LINK.findall("[a](./federation.mdx) [b](../howto/mcp.mdx#gossip)") == [
        "./federation.mdx",
        "../howto/mcp.mdx",
    ]
    assert LINK.findall("[v](../oracle/VISION.md)") == ["../oracle/VISION.md"]
    assert LINK.findall("[x](https://example.com/a.mdx)") == []
    assert SELF_URL.findall(
        "curl https://raw.githubusercontent.com/yesitsfebreeze/kern/master/install.sh | sh"
    ) == ["install.sh"]
    assert SELF_URL.findall(
        "[v](https://github.com/yesitsfebreeze/kern/blob/master/docs/oracle/VISION.md)"
    ) == ["docs/oracle/VISION.md"]
    assert SELF_URL.findall("https://github.com/yesitsfebreeze/kern/releases") == []
    assert (ROOT / "docs" / "oracle" / "ROADMAP.md") in pages(), "docs/oracle is scanned"
    assert (ROOT / "docs" / "kern" / "README.md") in pages(), "docs/kern is scanned"
    assert GONE.search("`docs/kern/x.md`, deleted 2026-07-20"), "a deletion excuses its line"
    assert not GONE.search("see `src/base/merge.rs:20`"), "a live citation is not excused"
    assert is_historical(["# Changelog", "", HISTORICAL, ""])
    assert not is_historical(
        ["a page holding `" + HISTORICAL + "` is skipped whole (`CHANGELOG.md`)"]
    ), "describing the marker inline must not exempt the page"
    print("selftest OK")


if __name__ == "__main__":
    if "--selftest" in sys.argv:
        selftest()
        sys.exit(0)
    sys.exit(main())
