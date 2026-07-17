# kern

**A self-learning memory substrate for AI agents.** One long-running daemon per
working directory owns a knowledge graph that captures durable facts from your
sessions, keeps itself small without manual gardening, and serves the right
context back when you need it.

```
session text → intake → distill (LLM) → typed claims → graph → digest → recall
```

If you're new, start with [The Memory Bank](./guides/memory-bank.md) — what kern
does and how to turn it on. For the whole system in one diagram, read
[Architecture](./guides/architecture.md). If you know which unit you need, jump
straight to its crate page.

## This book

- **Guides** walk an end-to-end flow across crates — read these first.
- **Crates** are per-crate reference: the public surface of each unit in the
  workspace.

## Beyond the book

- **[Vision](../../vision.md)** — what kern is and why it exists.
- **[README](../../../README.md)** — install, configure, and run it.
- **[Research & rationale](../../kern/README.md)** — the models and proofs behind
  the self-organizing, federated design.
- **[Aspiration](../../../aspiration.md)** — the north star and roadmap.
- **API** — rustdoc for every crate (`cargo doc --open`).
