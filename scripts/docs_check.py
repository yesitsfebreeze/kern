#!/usr/bin/env python3
"""Verify the docs' references are alive: every `src/...:line` citation points
at a real file and an existing line, every backticked repo path names a file
that exists, every relative page link points at a page that exists, and every
link into this repo's own files on GitHub names a file that is actually
committed. Exit 1 with a report if anything is dead.

Scans every documentation directory the repo has — the site, `docs/kern/`,
`docs/oracle/` and `README.md` — because a citation nobody checks is a citation
that rots. Existence is the only thing it can *fail* on.

Two escapes, because some citations are *supposed* to name a file that is gone.
A page carrying `<!-- docs-check: historical -->` is skipped whole: a changelog
entry recording a deletion must cite the deleted file, and a point-in-time note
describes the tree as it was. A single line naming a deletion is excused in
place, so a present-tense page can still say what it removed.

Beyond existence there is a second, weaker question: does the cited line still
*say* the thing it was cited for? A line anchor is a bet that nothing is ever
inserted above it, and appending to a growing file loses that bet silently —
the line still exists, it just says something else now. So every anchor with a
line number also gets a content check: the words of the citing bullet against
the words of the cited line(s). Near-zero overlap is *nominated*, not failed.
Short targets legitimately score low, and a checker that cries wolf gets turned
off, so nominations are printed under their own heading and leave the exit code
alone. `--strict-anchors` makes them fatal for a CI that has decided to trust
them. A nomination a human has adjudicated is silenced in place with a trailing
`docs-check: anchor-ok` comment, in the same idiom as the historical marker."""

import re
import sys
import tempfile
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

# A nomination a human has looked at and kept. Inline, on the citing line, so the
# verdict lives next to the thing it acquits.
ANCHOR_OK = "docs-check: anchor-ok"
CODE_SPAN = re.compile(r"`[^`]*`")
BULLET = re.compile(r"^\s*(?:[-*+]\s|\d+[.)]\s|#{1,6}\s|\|)")
WORD = re.compile(r"[A-Za-z0-9]+")
# `ReasonKind` has to reach a sentence that says "reason kind", or every doc line
# naming a type in prose scores zero against the line declaring it. Measured, this
# is a near-wash and not the win it looks like: against the real tree it silenced
# two false positives and one true one (`bayesian-belief.md:16`, whose target now
# matches on a stray "entity"). Kept because prose and code should tokenise alike,
# not because the numbers demanded it.
CAMEL = re.compile(r"[a-z0-9]+|[A-Z][a-z0-9]*")

# Four characters was too coarse a floor. It threw away exactly the tokens that
# distinguish a target: `acl`, `rrf`, `hub`, `run`, `dim`, `git`. Measured on the
# real tree, six of eleven false positives were nothing but a three-letter name
# the tokeniser refused to look at. Three is the floor now; two would admit `id`,
# `fn`, `to` and every other piece of syntax.
MIN_TOKEN = 3
# A stripped stem shorter than this is not a word, it is a fragment: `uses` must
# not become `us`, so the `es` rule declines and the `s` rule takes it to `use`.
STEM_MIN = 3
# Longest first, so `ies`/`ers` win over `es`/`s` and `edly` over `ed`.
SUFFIXES = ("ations", "ation", "ings", "ing", "edly", "ers", "ies", "ied", "er", "es", "ed", "ly", "s")
DOUBLE_KEEP = frozenset("lsz")


def _undouble(w: str) -> str:
    """`stemmer` → `stemm` → `stem`, `running` → `runn` → `run`. English doubles
    the final consonant before a vowel suffix; leaving the double in place is what
    kept `fn stem` from ever reaching the sentence that says "stemmer". `ll`/`ss`
    are real (`call`, `class`), so they stay."""
    if len(w) > STEM_MIN and w[-1] == w[-2] and w[-1].isalpha() and w[-1] not in DOUBLE_KEEP:
        return w[:-1]
    return w


def stem(w: str) -> str:
    """A light suffix stripper, not a linguist. Both sides of every comparison go
    through it, so consistency matters more than correctness: `stemmers`, `stemmer`
    and `stem` must agree, and it is fine that `status` lands on `statu` as long as
    it lands there from both directions."""
    for suf in SUFFIXES:
        if not w.endswith(suf) or len(w) - len(suf) < STEM_MIN:
            continue
        if suf == "s" and w.endswith("ss"):  # `class`, `less` — the s is the word
            continue
        base = w[: -len(suf)]
        if suf in ("ies", "ied"):
            base += "y"
        return _undouble(base)
    return w


# With a three-character floor the connective tissue is back in play, so the list
# has to carry it: articles, pronouns and prepositions manufacture agreement out
# of nothing. Rust's boilerplate keywords are here for the same reason — `let`,
# `pub`, `self` and `new` appear on nearly every line of the tree, so a match on
# one says only that the target is Rust. Stored stemmed, and looked up stemmed,
# because `files` and `file` must both be dropped.
STOPWORDS = frozenset(
    stem(w)
    for w in """about also because been before both cannot could does done each else even
    every from have here into itself just like made make many more most much must
    none only other over same should some still such than that their them then
    there these they this those through under until very what when where which
    while will with would your line lines file files
    and are but for its not the was who you all any can few had has her him his
    how may nor now off one our out own per see she too two use via way why yes
    did non yet had say get set new
    let mut pub ref mod self impl crate dyn
    """.split()
)
# The bar depends on what is being cited, because prose and code disagree by
# construction. Two documents describing the same thing reuse its words; two
# shared words is the bar the prototype cleared, catching eleven of eleven real
# FEATURES.md breakages. Prose citing *code* shares almost nothing on purpose —
# the sentence explains, the line implements — so measured against the real tree
# a two-word bar nominates 117 anchors, nearly all of them correct. For code the
# only believable signal is total silence: a target that shares no content word
# at all is a brace, a fragment, or drift.
PROSE_OVERLAP = 2
CODE_OVERLAP = 1
PROSE_SUFFIXES = (".md", ".mdx", ".txt")

line_counts: dict[Path, int] = {}
file_lines: dict[Path, list[str]] = {}


def lines_of(path: Path) -> int:
    if path not in line_counts:
        line_counts[path] = sum(1 for _ in path.open(encoding="utf-8", errors="replace"))
    return line_counts[path]


def text_of(path: Path) -> list[str]:
    if path not in file_lines:
        file_lines[path] = path.read_text(encoding="utf-8", errors="replace").splitlines()
    return file_lines[path]


def tokens(text: str) -> set[str]:
    """Content words: lowercase, stemmed, three characters or more, connectives
    dropped. `merge_claims` and `mergeClaims` both reach a sentence saying "merge
    claims", because snake_case splits on the underscore and camelCase on the
    case; `fn stem` reaches "stemmer" because both stem to `stem`."""
    out: set[str] = set()
    for raw in WORD.findall(text):
        for w in CAMEL.findall(raw) + [raw]:
            w = stem(w.lower())
            if len(w) >= MIN_TOKEN and w not in STOPWORDS:
                out.add(w)
    return out


def blocks_of(lines: list[str]) -> list[tuple[int, int]]:
    """Split a page into citing contexts: a bullet, a table row, a heading, or a
    paragraph. A citation is judged against the whole block it sits in, not its
    own line, because the docs wrap prose at eighty columns and the sentence that
    explains an anchor routinely starts two lines above it."""
    spans: list[tuple[int, int]] = []
    start = None
    for i, text in enumerate(lines):
        if not text.strip():
            if start is not None:
                spans.append((start, i))
                start = None
            continue
        if start is not None and BULLET.match(text):
            spans.append((start, i))
            start = i
        elif start is None:
            start = i
    if start is not None:
        spans.append((start, len(lines)))
    return spans


def block_at(spans: list[tuple[int, int]], idx: int) -> tuple[int, int]:
    for lo, hi in spans:
        if lo <= idx < hi:
            return lo, hi
    return idx, idx + 1


def acquitted(text: str) -> bool:
    """The marker counts only outside backticks, so a page may quote it while
    explaining it — the same discipline that keeps `is_historical` honest."""
    return ANCHOR_OK in CODE_SPAN.sub("", text)


def nominate(
    context: str, citation: str, target: Path, start: int, end: int
) -> tuple[int, set[str]] | None:
    """Does the cited line still relate to the sentence citing it? Compare content
    words. Returns the shared words when they are too few to believe, else None."""
    body = text_of(target)[start - 1 : end]
    # The citation itself is metadata, not argument: `src/retrieval/walk.rs:12`
    # would otherwise match any target line containing the word "walk".
    prose = tokens(context.replace(citation, " "))
    said = tokens("\n".join(body))
    shared = prose & said
    bar = PROSE_OVERLAP if target.suffix in PROSE_SUFFIXES else CODE_OVERLAP
    return (len(shared), shared) if len(shared) < bar else None


def pages() -> list[Path]:
    found = [ROOT / "README.md"]
    for d in PAGE_DIRS:
        found += sorted(d.rglob("*.md")) + sorted(d.rglob("*.mdx"))
    return found


# Standalone line only: a page that merely *describes* the marker must not exempt
# itself, which is how FEATURES.md and ROADMAP.md first went dark.
def is_historical(lines: list[str]) -> bool:
    return any(l.strip() == HISTORICAL for l in lines)


def check_page(page: Path, failures: list[str], nominations: list[str] | None = None) -> int:
    try:
        rel: Path | str = page.relative_to(ROOT)
    except ValueError:  # a selftest fixture living outside the repo
        rel = page
    lines = page.read_text(encoding="utf-8").splitlines()
    if is_historical(lines):
        return 0
    spans = blocks_of(lines)
    total = 0

    def anchor(m: re.Match[str], target: Path, lineno: int, label: str) -> None:
        """Existence is settled by the caller; this asks whether the words agree."""
        if nominations is None or not m.group(2):
            return
        lo, hi = block_at(spans, lineno - 1)
        # The citing line is inside its own block, so one sweep acquits both.
        if any(acquitted(l) for l in lines[lo:hi]):
            return
        start, end = int(m.group(2)), int(m.group(3) or m.group(2))
        if start < 1 or start > lines_of(target):
            return
        verdict = nominate("\n".join(lines[lo:hi]), m.group(0), target, start, end)
        if verdict is not None:
            count, shared = verdict
            witness = ", ".join(sorted(shared)) if shared else "nothing"
            nominations.append(
                f"{rel}:{lineno}: {label} shares {count} word(s) with its target "
                f"({witness}) — {text_of(target)[start - 1].strip()[:60]!r}"
            )

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
            else:
                anchor(m, target, lineno, f"{m.group(1)}:{m.group(2)}")
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
            else:
                anchor(m, target, lineno, f"{m.group(1)}:{m.group(2)}")
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
            else:
                anchor(m, target, lineno, f"{m.group(1)}:{m.group(2)}")
        for m in LINK.finditer(text):
            total += 1
            if not (page.parent / m.group(1)).resolve().is_file():
                failures.append(f"{rel}:{lineno}: dead page link {m.group(1)}")
        for m in SELF_URL.finditer(text):
            total += 1
            if not (ROOT / m.group(1)).is_file():
                failures.append(f"{rel}:{lineno}: dead self link {m.group(1)}")
    return total


def main(strict: bool = False) -> int:
    failures: list[str] = []
    nominations: list[str] = []
    total = sum(check_page(p, failures, nominations) for p in pages())
    code = 0
    if failures:
        print(f"{len(failures)}/{total} dead references:")
        print("\n".join(failures))
        code = 1
    else:
        print(f"docs-check: {total} references exist")
    if nominations:
        print()
        print(f"nominated for review — {len(nominations)} anchor(s) whose target no longer")
        print("reads like the sentence citing it. These are SUSPICIONS, not failures:")
        print("a short or terse target legitimately shares few words. A human decides.")
        print("Keep one by appending `<!-- docs-check: anchor-ok -->` to its line.")
        print("\n".join("  " + n for n in nominations))
        if strict:
            print()
            print("--strict-anchors: failing on the nominations above.")
            code = 1
    elif code == 0:
        print("docs-check: no anchors nominated for review")
    return code


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
    anchor_selftest()
    print("selftest OK")


# A page that cites its sibling twice: once at a line that says what the sentence
# says, once at a line that says something else entirely — the exact shape a merge
# produces when it appends above an anchor. Written to disk rather than asserted in
# the abstract, so the fixture exercises the same code path `main` does.
FIXTURE_TARGET = """\
# Features

Retention expires a fact out of query results once its horizon passes.

The gossip transport batches deltas before it hands them to the wire.
"""
FIXTURE_PAGE = """\
# Notes

- Retention expires the fact out of query results once the horizon
  passes (`FEATURES.md:3`).
- The gossip transport batches deltas before the wire sees them
  (`FEATURES.md:5`).
- Retention expires the fact out of query results once the horizon
  passes (`FEATURES.md:5`).
- Retention expires the fact out of query results once the horizon
  passes (`FEATURES.md:5`). <!-- docs-check: anchor-ok -->
"""


def stem_selftest() -> None:
    """Each rule gets the pair it exists for: two spellings that must agree, and a
    neighbour that must NOT be dragged along with them."""
    for a, b, why in [
        ("stemmer", "stem", "the case item 93 named: `fn stem` vs the word 'stemmer'"),
        ("stemmers", "stem", "plural of the same"),
        ("running", "run", "doubled consonant before -ing"),
        ("panics", "panic", "plural verb"),
        ("expires", "expir", "-es off a verb"),
        ("evicted", "evict", "-ed off a verb"),
        ("entities", "entity", "-ies restores the y"),
        ("proportionally", "proportional", "-ly is not part of the word"),
    ]:
        assert stem(a) == stem(b), f"{a!r} must reach {b!r} — {why}: {stem(a)} != {stem(b)}"
    # The other half of every rule: words the stripper must leave alone, because a
    # stemmer that over-strips manufactures agreement instead of finding it.
    for w, why in [
        ("class", "a trailing -ss is the word, not a plural"),
        ("less", "same"),
        ("uses", "-es would leave 'us'; the -s rule takes it to 'use'"),
        ("call", "-ll survives undoubling"),
        ("need", "-ed would leave 'ne', under the floor"),
    ]:
        assert stem(w) in (w, w[:-1]), f"{w!r} over-stripped to {stem(w)!r} — {why}"
    assert stem("uses") == "use", "the -s rule catches what the -es rule declined"
    assert stem("class") == "class" and stem("less") == "less"
    # Fires: the words agree only after stemming.
    assert tokens("the stemmer is hand-rolled") & tokens("fn stem(t: &str) -> String {"), (
        "'stemmer' must reach `fn stem` — the false positive item 93 named"
    )
    # Does not fire: two unrelated words that merely share a prefix.
    assert not (tokens("the stemmer is hand-rolled") & tokens("fn stencil(t: &str) {")), (
        "a shared prefix is not a shared stem"
    )


def short_token_selftest() -> None:
    """The three-character floor, both ways."""
    assert "acl" in tokens("pub acl: Acl,"), "three letters is a word now"
    assert "rrf" in tokens("pub fn rrf(lists: &[&[EntityHit]]) {}")
    assert "hub" in tokens("pub async fn run_hub(idle_unload_secs: u64) {")
    assert tokens("carries an acl") & tokens("pub acl: Acl,"), (
        "`acl` must reach the field it names — a false positive on the real tree"
    )
    assert tokens("`run_hub` at serve.rs") & tokens("pub async fn run_hub(u64) {")
    # Two characters stays out: `id`, `fn`, `to` are syntax, not subject matter.
    assert "id" not in tokens("pub id: String,"), "two characters is still noise"
    assert "fn" not in tokens("pub fn go() {}")
    # And the floor must not let boilerplate manufacture agreement.
    assert not (tokens("a new claim arrives") & tokens("GCounter::new()")), (
        "`new` is on every Rust line and in every sentence — it proves nothing"
    )
    assert not (tokens("the daemon pins itself") & tokens("let mut cfg = self.pub_ref();")), (
        "Rust boilerplate keywords are stopwords"
    )


def anchor_selftest() -> None:
    assert tokens("Retention expires the fact") == {"retention", "expir", "fact"}, (
        "short words and stopwords drop out; what is left is stemmed"
    )
    assert tokens("merge_claims") == {"merge", "claim"}, "snake_case splits, then stems"
    assert tokens("ReasonKind") == {"reason", "kind", "reasonkind"}, "camelCase splits"
    stem_selftest()
    short_token_selftest()
    assert blocks_of(["- a", "  cont", "- b", "", "para"]) == [(0, 2), (2, 3), (4, 5)]
    assert acquitted("cited (`X.md:3`) <!-- docs-check: anchor-ok -->")
    assert not acquitted("silence one with `<!-- docs-check: anchor-ok -->`"), (
        "quoting the marker must not silence the page explaining it"
    )

    with tempfile.TemporaryDirectory() as tmp:
        d = Path(tmp)
        (d / "FEATURES.md").write_text(FIXTURE_TARGET, encoding="utf-8")
        page = d / "NOTES.md"
        page.write_text(FIXTURE_PAGE, encoding="utf-8")
        failures: list[str] = []
        nominations: list[str] = []
        check_page(page, failures, nominations)
        assert failures == [], f"every fixture line exists: {failures}"
        # Line 8 is the mismatch — retention prose pointed at the gossip line.
        assert len(nominations) == 1, f"exactly one nomination expected, got {nominations}"
        assert ":8:" in nominations[0] and "FEATURES.md:5" in nominations[0], nominations[0]
        assert "gossip transport batches" in nominations[0], (
            "the nomination must quote the line it doubts"
        )
        # Line 3 cites the line that says what it says; line 5 cites gossip prose at
        # the gossip line; line 10 is the same breakage as line 8, adjudicated.
        for good in (":3:", ":5:", ":10:"):
            assert not any(good in n for n in nominations), f"{good} must not be nominated"

        # Code targets ride a lower bar, so prove both sides of it separately.
        rs = d / "accept.rs"
        rs.write_text("pub fn accept(g: &mut GraphGnn) {\n}\n", encoding="utf-8")
        drifted = nominate("The accept path stamps the reason.", "`x`", rs, 2, 2)
        assert drifted == (0, set()), f"a closing brace shares nothing: {drifted}"
        assert nominate("The accept path stamps the reason.", "`x`", rs, 1, 1) is None, (
            "one shared word acquits a code target — prose and code agree by name only"
        )
        assert nominate("Retention expires the fact.", "`x`", rs, 1, 1) == (0, set()), (
            "an unrelated sentence over the same line is still nominated"
        )

        # The two fixes item 93 named, each proved on a real target line and each
        # given a near neighbour it must still nominate. A rule only observed
        # staying quiet has not been observed at all.
        drift = d / "drift.rs"
        drift.write_text(
            "fn stem(t: &str) -> String {\n"  # 1 — reached only by stemming
            "\tpub acl: Acl,\n"  # 2 — reached only by the 3-char floor
            "\tpub async fn run_hub(idle_unload_secs: u64) {\n"  # 3 — same
            "\tlet n = a + b;\n",  # 4 — nothing but boilerplate
            encoding="utf-8",
        )
        cases = [
            ("swap the hand-rolled stemmer for rust-stemmers", 1, None, "'stemmer' reaches `fn stem`"),
            ("swap the hand-rolled parser for a real one", 1, (0, set()), "an unrelated swap still fires"),
            ("Entity also carries an acl", 2, None, "`acl` reaches the field it names"),
            ("Entity also carries a scope list", 2, (0, set()), "a sibling word does not"),
            ("`run_hub` at serve.rs is the accept loop", 3, None, "`run`/`hub` are words now"),
            ("the reaper unloads a dead node", 3, None, "`unload` still matches on its own"),
            ("a new claim arrives", 4, (0, set()), "`let`/`new` prove only that the target is Rust"),
        ]
        for prose, line, want, why in cases:
            got = nominate(prose, "`x`", drift, line, line)
            assert got == want, f"drift.rs:{line} — {why}: wanted {want}, got {got}"

        line_counts.clear()
        file_lines.clear()


if __name__ == "__main__":
    if "--selftest" in sys.argv:
        selftest()
        sys.exit(0)
    sys.exit(main(strict="--strict-anchors" in sys.argv))
