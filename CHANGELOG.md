# Changelog

- 2026-07-16 — Removed VOIT from the repo (the `.voit/` runtime dir + the VOIT-only `AGENTS.md`). Nothing in the build or tests referenced it; the oracle content files (VISION/FEATURES/ROADMAP/CHANGELOG/SPECIALISTS) plus the pre-commit hook are the project's process machinery, and the VOIT role/workflow files were a second, drifting set whose onboarding contract pointed at files that did not exist. Decided by: delete-superseded. Supersedes: the VOIT onboarding contract formerly in `AGENTS.md`.

- 2026-07-16 — Added `just insight` (`scripts/insight.py`): a measured repository snapshot (build, test count, code shape, oracle state, baseline presence) so project status is a run, not a recollection. Composes existing tools (cargo, nextest, tokei, git) rather than building analysis machinery. Decided by: verify-before-claiming, builtin-before-built. Supersedes: nothing.

- 2026-07-16 — Initialized the content files from the source tree: `VISION.md` (failable criteria distilled from `docs/vision.md` and `docs/aspiration.md`), `FEATURES.md` (present state, federation and the eval harness marked `building`), `ROADMAP.md` (seven open questions, eval baseline first), `SPECIALISTS.md` (seven delegation briefs by subsystem). Decided by: record-the-decision. Supersedes: nothing — first content.
- 2026-07-16 — Pinned the initial behavior set, ten from upstream `v1`, `verify-before-claiming` heaviest — measure-don't-assume is already this repo's loudest law (`docs/aspiration.md` claim standard). Decided by: the oracle. Supersedes: the empty pin list from install.
- 2026-07-16 — Installed the oracle: `ORACLE.md` is this repository's process machinery from here on. Decided by: the oracle. Supersedes: nothing.
