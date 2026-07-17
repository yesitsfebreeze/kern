# splinter: src/config/mod.rs

Recovered from git (killed-agent gap):

- `anchor_dir` (why the tiers): tier 1 is the nearest ancestor holding `.git` because the git repository root is the canonical anchor — every subdirectory of a repo then shares ONE `.kern` store instead of spawning a fresh one per launch cwd. Tier 2 (`.kern`) covers a non-git project tree that already carries a store; tier 3 is `start` itself.
- `anchor_dir` (innermost wins): the first `.git` found walking up wins, so a project under a git-managed home directory still anchors to the project root rather than collapsing into `~`.
- `anchor_dir` (existence, not `is_dir()`): `.git` is a directory in a normal clone but a FILE in a worktree or submodule, so the test is existence — that catches every repo-root shape without shelling out to `git`.
- `anchor_dir` (the failure it prevents): anchoring this way keeps the daemon from booting an empty graph against a `.kern` that does not exist beside its accidental cwd.
- `validate` (why the prefixes): each sub-config validates its own ranges; the section name is prefixed onto every issue so a bad value reports where it lives.
- `validate` (history): retrieval invariants were previously orphaned — `RetrievalConfig::validate` existed but nothing aggregated it, so a retrieval-breaking value never surfaced through the top-level `validate`. Now aggregated like the other sections; a test guards it.
- `answer_url` / endpoint precedence (why the fallback chain): so a single-Ollama deployment can fill in only `[embed]` and leave the reason/answer URLs empty — they all resolve to the embed endpoint, while each section stays free to override just the model. `[answer]` omitting `url` is the common case where only the model differs.
