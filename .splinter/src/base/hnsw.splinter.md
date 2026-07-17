# src/base/hnsw.rs — commentary

Hot path is pure `u32`: contiguous node arena (`nodes`), `Vec<u32>` adjacency per layer, beam heaps carrying slot ids. Strings only cross at the public API via `slot_of`/`id_of`. The u32-arena rewrite preserved exact ranking vs brute force (asserted by `search_order_matches_brute_force_on_separated_corpus`), not just set overlap.

- `Candidate`: made `Copy` (u32 + f64) deliberately so the beam heaps never clone a `String` per hop.
- `level_for`: pure-function-of-id design chosen over an RNG keyed on insert position — an RNG cannot give id-stability under arrival-order changes (HashMap iteration upstream, delete/insert churn), which would reshape the whole graph.
- `search_filtered`: exists as the alternative to post-filtering (search k then drop non-matches), which under-returns whenever matches are sparse in the top-k. This is the filtered-vector-search guarantee a dedicated vector DB provides.
- `HeapOrder`: zero-sized type parameter so the comparison monomorphizes with no per-operation branch.
- `recall_matches_brute_force` (test): this recall number is the one kern must beat Qdrant on — without it the index is unmeasured.
- `int8_recall_tracks_f64` (test): int8 quantization cuts vector memory 8x (the Qdrant-parity move); the test proves the quantized path is usable, not just present.
- `binary_recall_tracks_f64` (test): 1-bit sign quantization cuts vector memory ~64x but is far coarser than int8; the bar (0.30) is deliberately lower than int8's 0.75 — the test measures the recall floor of pure Hamming candidate-gen with no rescore, so the tradeoff is a number, not a guess. (The measured-floor justification stays inline at the assert.)
Second-pass migration (2026-07-17):
- `search_filtered` cost detail (was inline): with a sparse filter the result set never fills to `ef`, so the frontier stays open longer and the worst case approaches a full graph walk; the `visited` set bounds it to O(nodes).
- `beam_search_filtered` expansion rule: the frontier keeps expanding while the result set is under `ef` OR a neighbour is closer than the current worst match — non-matching nodes are still pushed to the frontier, which is how matches behind them are reached. Termination argument: once the matching set is full and the nearest frontier node is farther than the worst match, no closer match can exist.
- `level_for` mechanics: FNV-1a hash of the id → uniform (0,1] → the standard exponential level draw, capped at 16.
- `structure_digest` composition: entry point, max layer, then every live node in id order with its per-layer adjacency (as ids).
- `binary_recall_tracks_f64` numbers (moved from the assert; supersedes "stays inline at the assert" above): measured ~0.33 agreement at dim=32 (run 2026-06-12) — pure 1-bit Hamming without rescore loses ~2/3 of the f64 top-k, proving int8-rescore is required before Binary is usable (hence out of `QuantizationMode::parse`). The 0.30 floor locks that; the rescore phase must raise it, then the assert should lock the lifted floor.
- `alloc_slot` reuses freed slots so the arena stays compact under insert/delete churn (asserted by `delete_then_insert_reuses_slot_and_search_stays_correct`).
