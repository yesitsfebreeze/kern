# splinter: src/config/mod.rs

Recovered from git (killed-agent gap):

- `anchor_dir` (why the tiers): tier 1 is the nearest ancestor holding `.git` because the git repository root is the canonical anchor — every subdirectory of a repo then shares ONE `.kern` store instead of spawning a fresh one per launch cwd. Tier 2 (`.kern`) covers a non-git project tree that already carries a store; tier 3 is `start` itself.
- `anchor_dir` (innermost wins): the first `.git` found walking up wins, so a project under a git-managed home directory still anchors to the project root rather than collapsing into `~`.
- `anchor_dir` (existence, not `is_dir()`): `.git` is a directory in a normal clone but a FILE in a worktree or submodule, so the test is existence — that catches every repo-root shape without shelling out to `git`.
- `anchor_dir` (the failure it prevents): anchoring this way keeps the daemon from booting an empty graph against a `.kern` that does not exist beside its accidental cwd.
- `validate` (why the prefixes): each sub-config validates its own ranges; the section name is prefixed onto every issue so a bad value reports where it lives.
- `validate` (history): retrieval invariants were previously orphaned — `RetrievalConfig::validate` existed but nothing aggregated it, so a retrieval-breaking value never surfaced through the top-level `validate`. Now aggregated like the other sections; a test guards it.
- `answer_url` / endpoint precedence (why the fallback chain): so a single-Ollama deployment can fill in only `[embed]` and leave the reason/answer URLs empty — they all resolve to the embed endpoint, while each section stays free to override just the model. `[answer]` omitting `url` is the common case where only the model differs.

# Ratings — scope: src/config/mod.rs

Scope rating: 8/10 — layered TOML config (user + project), zero-config defaults, WSL loopback rewrite, data-dir anchoring, root resolution (walks up to nearest .kern or git root). The `dirs::config_dir()` fallback path had a doubled `kern/` segment; fixed.

## Function ratings

- `Config::load` — 7/10→9/10: was `dirs::config_dir().unwrap_or_else(|| cwd.join(".kern")).join("kern").join("kern.toml")` — the `None` fallback produced `cwd/.kern/kern/kern.toml` (doubled `kern/`). Fixed to `.map(|d| d.join("kern").join("kern.toml")).unwrap_or_else(|| cwd.join(".kern").join("kern.toml"))` so the fallback hits `cwd/.kern/kern.toml`. The WSL rewrite and data-dir anchor are correct.
- `Config::default_in` — 9/10: clean defaults, all sub-configs constructed.
- `anchor_data_dir` — 9/10: correct relative-to-cwd anchoring.
- `resolve_root` — 9/10: walks up to nearest .kern or git root. Good test coverage.
- `validate` — 8/10: surfaces sub-config invariants.
- `redirect_loopback_to_wsl_host` — 9/10: probe-first, only-default-URL, explicit-config-never-second-guessed. Correct WSL2 NAT handling.
Config merge model: user <XDG_CONFIG>/kern/kern.toml overlaid by project <cwd>/.kern/kern.toml. Section-level merge; missing fields fall to Default.

default_in(cwd) is Default with an explicit cwd (deterministic for tests) — only data_dir depends on cwd; every other field is a fixed baseline.

redirect_loopback_to_wsl_host rationale: repoints loopback embed/reason/answer endpoints at the Windows host under WSL2 NAT networking where localhost cannot reach a host-side Ollama. Only fires when loopback is dead and the gateway is live, so mirrored-mode WSL2 and in-distro Ollama are untouched (see wsl::resolve_loopback). Without it a stock WSL install fails silently forever: embeds return transient connect errors, ingest re-spools every job, and the graph stays empty with nothing surfaced to the user.

resolve_root tiers: nearest ancestor with .git (innermost wins), else nearest with .kern, else start. .git-may-be-a-file hazard kept in source.

Endpoint precedence accessors: reason_* falls back to [embed]; answer_* falls back to the resolved reason_*.
