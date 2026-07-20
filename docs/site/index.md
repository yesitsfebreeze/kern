# kern

**A self-learning memory substrate for AI agents.** One long-running daemon per
working directory owns a knowledge graph that takes in durable facts from your
sessions, keeps itself small without manual gardening, and serves the right
context back when you need it.

```
session text → intake → distill (LLM) → typed claims → graph → recall
```

## Where to start

- **New here?** [Why kern exists](./concepts/why-kern.md) — the problem, and why
  chunk-embed-retrieve does not solve it.
- **Want it running?** [Install & run the daemon](./howto/install-run.md), then
  [The memory bank](./howto/memory-bank.md).
- **Want the whole system in one diagram?** [Architecture](./concepts/architecture.md).

**Concepts** explain the mental model — read these to understand what kern is
doing and why. **How-to** pages are task-shaped: you have a goal, they get you
there.

!!! warning "Maturity"

    Federation is `building`, not shipped, and its transport is currently
    unauthenticated and unencrypted. See [Federation](./concepts/federation.md)
    before enabling it.

## Beyond these pages

- **[Vision](https://github.com/yesitsfebreeze/kern/blob/master/docs/oracle/VISION.md)** — what kern is and why it exists.
- **[README](https://github.com/yesitsfebreeze/kern/blob/master/README.md)** — install, configure, and run it.
- **[Research & rationale](https://github.com/yesitsfebreeze/kern/blob/master/docs/kern/README.md)** — the models and proofs behind
  the self-organizing, federated design.
- **[Roadmap](https://github.com/yesitsfebreeze/kern/blob/master/docs/oracle/ROADMAP.md)** — the north star, the measured baseline, and every open work item.
- **API** — rustdoc for every crate (`cargo doc --open`).
