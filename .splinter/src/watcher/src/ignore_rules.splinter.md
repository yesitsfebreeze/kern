# src/watcher/src/ignore_rules.rs — commentary

- `IgnoreRules`: reuses the `ignore` crate (already a dep of `shared/search`) rather than duplicating gitignore semantics.
Second-pass migration:
- `is_ignored` `is_dir = false` full argument (compressed inline to 2 lines): the `ignore` crate uses `is_dir` only to classify the matched path itself for trailing-slash patterns; a notify file event is never a directory listing, so `false` avoids a `stat` per match and still matches file patterns (`*.log`, `secret*`) as intended.
- Matchers are evaluated per-root: a path is ignored iff it strips to a prefix of some root and that root's matcher reports ignore.
- `Gitignore::matched` checks only the event path — no parent-dir walking — so a trailing-slash dir pattern will not catch nested files; `.git/**` is the one recursive prune, hard-coded ahead of the matchers.
- `empty()` (no rules) exists for tests.
