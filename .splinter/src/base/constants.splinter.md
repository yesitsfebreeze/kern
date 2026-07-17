# src/base/constants.rs — commentary

- `ACCEPT_FLOOR` (0.5): matches the long-standing rejection cutoff used by the per-node acceptance gate.
- `ANCHOR_DEDUP_THRESHOLD` (0.85): chosen against the observed failure — 9+ LLM rephrasings of one concept minted as separate root anchors, whose name embeddings were near-parallel.
- `QUERY_CACHE_DEFAULT_CAP` / `QUERY_CACHE_DEFAULT_THETA`: live here (pure data) rather than in `retrieval::cache` so `config` can default them without a `config -> retrieval` dependency cycle; the cache module reads them from here too.
- `TICK_MAX_CLUSTER_SAMPLE` / `TICK_QUEUE_CAPACITY` / `TICK_INTERVAL_SECS`: the default knobs behind `config::TickConfig`.

Second-pass migration (tuning rationale moved out of the doc comments):
- `COLD_COMPACT_MIN_BYTES` (256 KiB): compaction rewrites the whole file (O(total)), so gating on size stops steady-state GC rewriting the entire store every sweep for a handful of victims — and because compaction shrinks the file, the gate self-rate-limits. Reads stay correct meanwhile: latest-line-wins is applied in memory.
- `COLD_MAX_ENTRIES` (50k): absolute cap so the cold tier cannot grow without bound over the daemon's lifetime; compaction keeps the newest by creation time, drops the oldest.
- `DEDUP_EF` (64): dedup asks for the single closest entity (k=1); at `ef=1` HNSW search is greedy single-path and routinely misses the true nearest neighbour, so genuine duplicates slip through and create divergent content-hash entities. A wider beam restores recall at negligible cost for a k=1 query.
- `INGEST_DEDUP_THRESHOLD` (0.95): deliberately higher than the anchor-path `DEFAULT_DEDUP_THRESHOLD` (0.92) — ingest dedup wants near-exact matches before collapsing two thoughts. Consumed by `place::find_duplicate`.
- `TICK_MAX_CLUSTER_SAMPLE` (200): caps clustering cost on large kerns; above this size sampling is coarser by design.
- `KERN_CAP_DISABLED`: named so the sentinel reads as intent at every site instead of a bare `usize::MAX`. A finite cap is currently unsafe — see the `GraphConfig::default` comment for the evict/persist consistency bug it triggers — so this is the only value used.
- `ACCEPT_FLOOR` / `GENERIC_ANCHOR`: the floor is the per-node acceptance gate against a named child of the dispatcher; below it the entity falls through to the `generic` catch-all, which is reachable ONLY as a fallback because its empty `anchor_vec` never matches similarity routing.
- `ANCHOR_DEDUP_THRESHOLD` (0.85): deliberately well above `KERN_COHESION_THRESHOLD` (cluster membership, 0.60) and `ACCEPT_FLOOR` (routing, 0.5) — merging two genuinely distinct anchors is worse than tolerating a borderline duplicate, so the gate only fires on near-parallel vectors.
- `QUERY_CACHE_DEFAULT_THETA` (0.97): high enough that only paraphrases and re-asks collide, not merely topical neighbours.
- `GOSSIP_REMOTE_KERN_ENTITY_CAP` (50k): bounds memory growth from a peer spamming forged `EntitySync` bodies.
- `GOSSIP_CRDT_DELTA_MAX` (1M): the value is the sender's absolute slot total, max-merged into the local GCounter; rejecting values above this coarsely bounds a peer pinning a slot toward `u64::MAX`. Realistic access/traversal tallies are far below it. Full per-replica ownership authentication is tracked separately.
- Stigmergy GC trio — the arithmetic behind the numbers: heat half-life is ~36h (docs/kern/stigmergy-self-improving.md), so after 7 days an unaccessed thought decays by ~2^(-7*24/36) ≈ 0.01, which is exactly `COLD_HEAT_THRESHOLD`. `COLD_GC_AGE` (7d) gives the half-life roughly five half-lives to decay transient activity before a thought counts as abandoned. `STIGMERGY_GC_INTERVAL` (hourly) is more than fast enough because the age gate dominates, not the sweep frequency; the TaskKey dedup map prevents duplicate `StigmergyGc` tasks per kern while one is pending.
- `DISK_CONSOLIDATE_INTERVAL` (hourly) bounds delta growth without paying the snapshot-rebuild cost too often; `DISK_CONSOLIDATE_MIN_DELTA` (10k) is the point below which the delta is small enough to keep searching in RAM.
- `AGENT_SOURCE` trailing comment: kept the do-NOT-duplicate trap; the dropped half explained that AGENT_SOURCE pairs with USER_SOURCE for the Fact-tier gate and that `base::descriptors` reuses it rather than defining a second const with the same value.
