# src/gnn/propagate.rs — commentary

- `DEFAULT_MIN_THOUGHTS` (128): a multi-layer GNN over a handful of nodes only overfits, and the noisy `gnn_vector` then pollutes ranking via `gnn_entity_idx` — hence training is skipped below this entity count and small graphs use the vector+BM25+PageRank+reason-edge path.

Second-pass migration:
- Defaults-block doc compressed to the "single source of truth, never re-literal" constraint (the runtime `GnnConfig::defaults` and the serde `crate::config::GnnConfig` default must both read these consts or they drift silently).
- `GnnSnapshot::weights`: deleted the `// persisted model state` trailing label (restates the field name; the `marshal_weights` / `unmarshal_weights` round trip types it).
- `tiny_snapshot` doc compressed to 2 lines. It has no persistence or LLM dependency, which is why the propagate tests run in-process.
- Undocumented behaviour worth knowing: `run_learned_propagation` swallows an `unmarshal_weights` failure (`let _ =`) and trains from fresh init instead — corrupt or version-stale persisted weights degrade to a cold start, never an error. `hidden` is `(dim/2).clamp(16, 256)`. `sample_negative_edges` is best-effort: it gives up after `want * 30` rejection-sampling attempts and may return fewer than `want`, and an empty result aborts the run.
