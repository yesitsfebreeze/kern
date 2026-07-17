# src/retrieval.rs — commentary

Stage order composed by `answer::query`: `hyde` (optional LLM HyDE expansion of the query) → `seed` (dense ANN over the query vector + BM25 lexical index) → `expand` (graph expansion from seeds; personalized-PageRank / HippoRAG-style multi-hop association, `pagerank` is the core kernel) → `fuse` (weighted Reciprocal Rank Fusion of candidate lists) → `score`/`rerank` (fold heat / confidence / graph signals into the final score) → `merge` (combine overlapping/duplicate hits) → `diversify` (MMR so near-duplicates don't crowd out) → `answer` (glue surviving context into prose via the answer LLM). `cache` memoises whole results keyed on the raw query embedding, skipping the ~30 s LLM path on repeats/near-repeats; `digest` builds the SessionStart recall digest.

The old module doc also listed a `heap` stage ("bounded top-k selection") — no such module exists; dropped as stale.
