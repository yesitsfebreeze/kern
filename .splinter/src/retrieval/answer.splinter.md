# src/retrieval/answer.rs — commentary

- `retrieve` / `query_locked` lock-scoping history: holding the graph read lock across a multi-second LLM call let a single slow `answer:true` query starve every worker (blocking writers pile up behind the long-held read lock) and trip the 30 s watchdog; scoping the lock to the sub-millisecond graph phase (and, in `query_locked`, releasing it before HyDE/rerank/answer) removed that. A slow cloud model can no longer pin the read lock.
- `retrieve_profiled`: per-stage checkpoints are seed_dense → fuse_hybrid → expand → merge → boosts+filter → mmr → materialize → chains; the bench's `--profile` leg uses it to see which graph stage dominates.
- `retrieve`: only the delivery survivors are cloned out of the graph (`ScoredRef::to_owned` at the end); every earlier stage works on borrowed entities.
- `refine_edges`: the score write-back goes through the kern id `find_reason` already returned — O(1) per edge instead of the old O(N_kerns) `all_ids()` rescan that made the loop O(R * K).
Second-pass migration:

- `fuse_hybrid_seeds` weighting (resolves the `(see note)` on its doc comment): four lists go into `fuse::rrf` — dense, lexical, importance, and (when `pagerank_enabled`) PageRank. Dense and lexical are query-relevant and weigh 1.0; importance and PageRank are query-*independent* priors and both get `cfg.rrf_global_weight`. At equal weight a globally popular but irrelevant entity at rank 1 of the importance list contributes exactly as much as a genuinely relevant entity at rank 1 of the dense list, so the priors would drive the ranking instead of tie-breaking it. The PageRank list is only appended when non-empty, so a disabled/empty PageRank never shifts the other lists' weights. Fusion is capped at `seed_k.max(1) * 2`.
- `fuse_hybrid_seeds` PageRank teleport: the personalization seeds are dense + lexical hits only, deliberately excluding the importance list — importance is query-independent, so teleporting at it would make the personalized PageRank query-blind and collapse it toward global PageRank.
- `retrieve_profiled` filter ordering (resolves the `(see note)` at the pre-`filter_delivery` retain): an active metadata filter must be applied BEFORE `score::filter_delivery` truncates the pool. Expansion pulls in graph neighbours that need not match the filter; if truncation ran first, those non-matching neighbours would occupy slots in the cap and push matching entities out of the delivered set, so a filtered query could return fewer matches than exist. This is the same fewer-than-k coverage bug the filtered-ANN path fixes at the seed source (see the seed.rs note) — expansion is the second place non-matching entities enter the pool.
Query pipeline structure and lock discipline:
- query() = query_profiled() dropping the Profile; query_profiled returns stage-level Profile so `kern profile` can render the timing breakdown.
- Retrieved.chain_text is pre-rendered WHILE the graph lock is held, so the answer prompt needs no graph access after the lock is released.
- retrieve() is the graph-only half: seed -> expand -> merge -> score -> diversify. NO LLM — callers hold the graph lock for exactly this sub-millisecond phase. retrieve_profiled is the single impl; retrieve delegates and drops the profile.
- synthesize()/answer_prompt_from(): build the answer prompt and run the LLM with NO graph access — callable after the lock is released. build_answer_prompt is the graph-taking convenience wrapper.

fuse_hybrid_seeds (hybrid seed fusion via weighted RRF):
- Query-relevant lists (dense, lexical) weigh 1.0; query-independent priors (importance, PageRank) get cfg.rrf_global_weight.
- PPR teleport is personalized at dense + lexical seeds ONLY — importance is query-independent and would make PageRank query-blind if included.

retrieve_profiled:
- The O(N) importance scan feeds BOTH the dense-seed merge and the RRF list in fuse_hybrid_seeds — run once here and threaded into both.
- An active filter must run BEFORE filter_delivery's pool truncation, or expansion's non-matching neighbours crowd matching entities out of the cap.

query_locked (daemon MCP path; plain query() serves one-shot CLI): holds the read lock for ONLY the graph phase; every LLM call runs unlocked.
- HyDE LLM call is graph-free, so it runs BEFORE taking any lock.
- The mutation epoch is captured under the SAME lock as retrieval: a write during the lock-free LLM phase leaves the cache stamp born stale -> miss, never a stale serve.
- Live-graph access write-back is deferred to a CommitAccess tick task (see mcp::Server::tool_query) so this path takes ONLY a read lock.
