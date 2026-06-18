# Research & rationale

The notes in this directory are the design rationale behind kern's
self-organizing and federated behavior — the models, proofs, and trade-off
analyses that the implementation is built on. Several are referenced directly
from source-code doc comments, so they double as the canonical "why" for the
mechanisms they describe.

These are reference material, not a tutorial. For how to *use* kern, start with
the [README](../../README.md) and the [Memory Bank guide](../book/src/guides/memory-bank.md);
for what it is and why it exists, read the [Vision](../vision.md).

## Self-organization

- **[Stigmergy for self-improving memory](stigmergy-self-improving.md)** — the
  heat / decay / evict / cluster loop that keeps the hot graph small without
  manual gardening. Implemented by `tick::stigmergy`.
- **[Safety architecture](safety-architecture.md)** — the guardrails on
  autonomous mutation: confidence bounds, typed kinds, and what is never
  auto-forgotten. Referenced by the mutation tools.
- **[PageRank for authority](pagerank-authority.md)** — how graph centrality
  weights retrieval and federated trust.
- **[Bayesian belief](bayesian-belief.md)** — multi-observer truth convergence:
  how confidence updates as independent observations accumulate.
- **[Wikipedia edit-convergence](wikipedia-edit-convergence.md)** — the
  NPOV-style model for converging on neutral, durable thoughts.

## Federation

- **[CRDTs for federated state](crdts-federation.md)** — the content-addressed,
  conflict-free merge that lets nodes converge with no coordinator.
- **[Federated learning vs. kern federation](fl-vs-knids-federation.md)** — why
  kern gossips knowledge rather than gradients.

## Retrieval & indexing

- **[DiskANN-style disk-resident index](diskann-disk-index.md)** — the
  design for the on-disk ANN index (built and tested; see `base::diskann`).
- **[Benchmark results](bench-retrieval.md)** — recorded retrieval benchmark
  numbers.

## Planning

- **[Board unblock plan](board-unblock-plan.md)** — what each open work item
  needs to finish.
</content>
