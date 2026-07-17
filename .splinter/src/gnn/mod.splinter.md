# src/gnn/mod.rs — commentary

Module map: `gcn` implements the `GraphLayer` / `BackwardGraphLayer` traits; `loss` and `optim` drive the training step (run inline by `propagate`); `tensor` is the minimal dense-matrix backbone — deliberately no external BLAS. Operation errors surface as `GnnError`.


Second-pass migration:
- Module doc compressed. Moved here: a small from-scratch GNN periodically re-embeds the entity graph so `gnn_entity_idx` (on `crate::base::graph`) captures structural/relational signal the raw content embeddings miss; the tick loop writes back per-node `gnn_vector`s, fused with content similarity in `base::search::merge_hits`.
- `GnnError` variant docs deleted (the `#[error]` messages carry them). One nuance kept here: `MissingForwardState` also fires when cached state was reset after a successful forward, not only when forward was never called.

## Preserved from stripped comments (2026-07-17)
- Subsystem role: the GNN is kern's learned re-embedder. The tick loop trains it on the live graph; retrieval fuses its `gnn_vector`s with content similarity.
- `GnnError` is the shared error enum across all gnn submodules by design — extend this enum rather than adding per-site error types.
