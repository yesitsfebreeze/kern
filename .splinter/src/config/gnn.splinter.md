# src/config/gnn.rs — commentary

- `GnnConfig` is a field-identical twin of the runtime `gnn::propagate::GnnConfig`, kept separate only so the serde derives don't leak into the hot runtime type; `From<GnnConfig>` bridges the two. This struct exists purely so the config can be (de)serialized from TOML.
Field semantics removed from source doc comments:
- self_weight: residual self-weight [0,1]; blend = self_weight*own + (1-self_weight)*neighbour. Higher keeps more own signal (less smoothing).
- min_weight: edge-weight floor; propagation ignores weaker neighbour edges.
- min_thoughts: minimum entity count before GNN training runs. Below it a multi-layer GNN overfits, so retrieval falls back to vector + BM25 + PageRank + reason edges.
- train_epochs / train_learning_rate: Adam epochs and learning rate per re-embed pass.
Both this serde view and runtime gnn::propagate::GnnConfig draw defaults from the same DEFAULT_* consts; the From impl and the two drift-guard tests keep them aligned.
