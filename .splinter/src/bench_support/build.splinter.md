# src/bench_support/build.rs — commentary

Kept separate from the replay/scoring loop (`replay.rs`) so each module owns a single responsibility — build vs measure.

- `SIMILARITY_EDGE_FLOOR`: a real graph's reason-edges connect genuinely related entities; a near-orthogonal pair (cosine ~0.1) is not "related". A loose floor wires almost every pair together, and that dense graph lets graph expansion's corroboration boost promote well-connected central nodes over the direct best match. Measured on synthetic.json: NDCG@10 was 0.54 with floor 0.1 vs ~1.0 at 0.5.
- `seed_similarity_edges`: the rayon pair scan borrows each vector once and runs on all cores, so 10k-doc traces build in seconds instead of the former per-pair double-clone crawl. ANN top-k edge seeding was evaluated and rejected — byte-identical edge set at 1k docs but a net timing regression; see the `pairwise_seeding_matches_ann_top_k_1k` test and `traces/edge-seeding-equivalence-1k.md`.

Second-pass migration:
- `build_graph` / `insert_docs` (docs deleted): build_graph inserts each trace document as a Claim entity carrying the deterministic bench embedding of its text, seeds pairwise similarity edges, then builds the ANN index and populates BM25. The BM25 line stays inline — an empty lexical index makes "hybrid" queries silently fall back to dense-only, which is a real bug that shipped once.
- `pairwise_edge_ids` (doc deleted): reads the reason ids the pairwise seeder actually produced off the built graph.
- `pairwise_seeding_matches_ann_top_k_1k`: the doc is compressed to the decision + the "do not optimize the O(n^2) scan away" oracle. Full record: at 1k docs the ANN top-k edge set is byte-identical to the pairwise set (120 edges), and timings are in `traces/edge-seeding-equivalence-1k.md`. The test also prints an `EDGE-ARTIFACT` fingerprint line so the edge set is diffable across runs.
- `thousand_doc_clustered_trace` keeps its fixture doc inline: 40 clusters of 3 near-identical docs (siblings cosine ~0.89 > FLOOR) among 880 unrelated singles, so the only >FLOOR pairs are the 120 intra-cluster edges — that is what justifies the `120` assertion.
