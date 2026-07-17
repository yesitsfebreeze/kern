# src/tick/gnn_propagate.rs — commentary

- `build_gnn_snapshot`: node feature vectors are stored on the `Graph` and the feature matrix is materialized once via `gg.feature_matrix()` — no separate feat_data buffer. Cross-kern edge skip ("local model, local edges"): `gnn_vector` is explicitly excluded from CRDT replication per docs/kern/crdts-federation.md §7. Commit a29ea34 stamps `to_kern_id` more aggressively on `move_entity`, which increases the count of skipped reasons here — the intended outcome, not a regression.
- `cosine_align`: delegates the dot/norm math to `base::math::cosine` (its SIMD path) instead of a second hand-rolled copy.

## Second-pass migration:
- Superseded-entity exclusion (moved from inline on build_gnn_snapshot): superseded entities are never searchable (excluded from entity_idx / gnn_entity_idx), so propagating over them is wasted work that would ALSO re-insert them into gnn_entity_idx via `apply_gnn_updates`, undoing the supersede index-removal. Guard: `superseded_entities_excluded_from_gnn_snapshot`.
- f64/f32 boundary: the GNN tensor core stays f64 internally; entity vectors are f32. Widen once when building node features, narrow once in `apply_gnn_updates` when writing `gnn_vector` back.
