# AGENTS

This project runs [VOIT](https://github.com/yesitsfebreeze/voit) - four git-scoped
agent roles (Vision / Organize / Implement / Tweak) over a shared, versioned memory.

Any agent, before working here:

- **Orient:** read `.voit/memory/overview.jd` (project overview) and
  `.voit/memory/glossary.jd` (shared terms).
- **Decisions:** `.voit/memory/decisions.jd` is append-only. Consult it before
  re-deciding anything; append every durable decision you make.
- **Conventions:** `.voit/.jd/library/voit/conventions.jd`.
- Nothing reaches `main` without review.

Claude Code loads the full workflow (roles, write-scope hooks, message bus) via the
voit plugin. Other agents: the paths above are plain text - read them directly.
