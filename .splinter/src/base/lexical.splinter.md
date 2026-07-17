# src/base/lexical.rs — commentary

- `set_bm25_params`: wired from `config.retrieval.bm25_k1`/`bm25_b` at daemon load — before this method existed the configured values were dead and BM25 always ran on the hardcoded construction defaults.
- `rebuild_from_graph`: the previous version dropped the write lock after clearing and re-acquired it once per entity via `self.insert()` — thousands of lock round-trips on a large graph, plus a window where the index was visibly empty to concurrent readers. Hence the single-guard rebuild.
- `stem`: deliberately crude — exists only to collapse common regular English inflections (plurals, -ing/-ed/-ly) enough to lift lexical recall; a precise Porter/Snowball stemmer would add a dependency for marginal gain at this layer.
- `tokenize`/`stem`: no stopword removal anywhere — BM25's idf already down-weights common words, so a query for a rare term isn't diluted.
Second-pass migration (2026-07-17):
- `search_filtered` rationale (was in the doc): BM25 scores every doc containing a query token, so filtering pre-truncation returns a full k matching hits — no over-fetch and none of the post-filtering fewer-than-k loss. `keep` is built at the retrieval layer from a `QueryOptions` filter (`score::matches_filter`), keeping this base-layer index free of any retrieval dependency.
- `inner_insert` is shared by `insert` (locks, single doc) and `rebuild_from_graph` (one lock, every doc).
- `stem` failure examples (doc now keeps only the pattern): irregular forms "mice"/"ran"/"better" are left as-is; "ties" -> "t" via the `ies` suffix.
- `rebuild_from_graph` skips empty-statement entities rather than indexing zero-length docs (asserted by `rebuild_from_graph_indexes_every_nonempty_entity`).
