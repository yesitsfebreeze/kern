# Features

A full technical scrape of everything that actually exists in the kern source
today. Organized by subsystem. For each: **what** it does, **how** it works,
**where** it lives in the code, and **gaps** (known limitations / improvement
opportunities). Version: `1.1.0`. LoC ~44.7k across 175 `.rs` files.

State legend: `active` (runs today), `building` (wired but partial/unverified),
`off` (present but disabled by default).

---

## 0. Architecture at a glance

```
session delta (.txt) ──► intake ──► distill (LLM) ──► typed claims
                                                               │
                            kern tree (content-hash ids) ◄─────┘ accept()
                                   │            │
                              reason edges    access heat
                                   │            │
   ┌───────── MCP (stdio+SSE) ────┼──────── RPC (tarpc socket) ────┐
   │            ▲                  │                 ▲              │
   │        query pipeline ◄───────┴──────────►  recall            │
   │  (HNSW+BM25 seed → expand → RRF+PageRank → MMR → answer)      │
   │                                                                │
   │   tick queue ──► cluster / name / enrich / gc / gnn / persist  │
   │                                                                │
   └── gossip ◄── CRDT entity-body + delta merge (LAN, opt-in) ─────┘
```

One daemon per working directory (gated on `.kern/`). Everything below is the
single process that owns that directory's graph.

---

## 1. Graph data model — `active`

**What.** Two node kinds: *thoughts* (`Entity`, typed) and *justified edges*
(`Reason`). Ids are content hashes — identical content is the same node
everywhere, which is what makes conflict-free cross-node merge work.

**How.**

- `Entity` (`src/base/types.rs:246`) — typed (`Fact`/`Claim`/`Document`/
  `Question`/`Answer`/`Conclusion`, `src/base/types.rs:19`), weighted by
  confidence (a beta distribution stored as `conf_alpha`/`conf_beta`, read via
  the `conf_mean`/`conf_variance` methods, updated via
  `observe_support`/`observe_contradict`)
  - access `heat`. Carries a bi-temporal window (`valid_from`/`valid_to`,
  `created_at`), `status` (`Active`/`Superseded`), `superseded_by`, `statements`
  (OR-Set of text lines), two vectors (`vector` content, `gnn_vector` structure),
  and provenance (`Source` with `system`/`object_id`/`section`/`title`/`author`/
  `url`). `kind`/`source` parsed off the source string.
- `Reason` (`src/base/types.rs:408`) — an edge `from`→`to` with a `kind`
  (`Similarity`/`Provenance`/`Question`/`Spawn`/`Supersedes`/`Ratification`/
  `Rephrase`, `src/base/types.rs:66`), its own vector (mean of endpoints), a
  `traversal_count` GCounter (`src/base/types.rs:427`), and a CRDT `score`.
  `is_enriched`/`is_remote` flags. There is no `Contradiction` edge kind —
  `Related` is a `ContradictionClass` verdict, not an edge, and a deferred
  contradiction candidate is carried by a `Rephrase` edge.
- `Kern` (`src/base/types.rs:456`) — a container node in the kern tree:
  `entities` + `reasons` maps, `children` ids, a `graviton_vec`/`graviton_text` + `mass` (default 1.0),
  radii (`inner_radius`/`outer_radius`) for acceptance gating, and an
  `access_count`. Root, named children, and unnamed (spill) children are all
  `Kern`s distinguished by `is_unnamed`/`is_named`/`has_graviton`.
- `GraphGnn` (`src/base/graph.rs:64`) — the whole in-memory forest: `kerns`
  map, `root`, `entity_idx` (HNSW over content vectors), `gnn_entity_idx`
  (HNSW over GNN vectors), `entity_adjacency` (reason-edge incidence),
  source routing, a `Lamport` clock, a `mutation_epoch`, pending CRDT deltas,
  and an optional bound `Store` (LMDB) for hot/cold tiers + disk fallback.

**Where.** `src/base/types.rs` (916 LoC), `src/base/graph.rs` (1106 LoC),
`src/base/reason.rs` (edge add/remove/move), `src/base/search.rs` (graph-wide
entity/reason lookup + unlocked vector search).

**Gaps.** `Entity` is a large flat struct (~30 fields); a trait-object or
sharded layout could cut serialization cost. `Kern` carries no per-kern
statistics (mean heat, fill ratio) that clustering could reuse cheaply.

---

## 2. Acceptance & routing — `active`

**What.** Decides where a new thought lives in the tree and whether it
supersedes an existing one. The core write path every ingestion funnels through.

**How** (`src/base/accept.rs:26` `accept()`):

1. **Dedup** — graph-wide top-1 vector search; if `score > INGEST_DEDUP_THRESHOLD`
   (0.95, `src/base/constants.rs:22`; `DEDUP_EF=64`), the thought is a duplicate and merges into the
   existing entity (no new node).
2. **Route** (`route_entity`) — descend from the target kern toward a leaf:
   - For each loaded child, route into the one whose graviton is nearest by
     effective distance `cosine_distance / mass` (`mass` default `1.0`,
     `1e-6` epsilon floor) — heavier gravitons both attract and retain.
   - At the **root** (a pure dispatcher): a no-graviton-match falls through to a
     `generic` catch-all child (empty graviton vec, never matches on similarity) —
     the root never commits entities itself.
   - At a **named** kern with a graviton: compute `acceptance_probability`
     (softmax over cosine distance vs `inner`/`outer` radii); below
     `ACCEPT_FLOOR` (0.5) → spawn an unnamed child and descend.
   - Max depth 64 to bound a runaway descent.
3. **Commit** (`commit_entity`) — stamp `root_id`, insert into the
   `entity_idx`/`gnn_entity_idx`, attach a `Similarity` reason to the nearest
   existing neighbor and a `Provenance` reason to the source doc.

**Where.** `src/base/accept.rs` (1259 LoC). Radii defaults in `constants.rs`
(`KERN_INNER_RADIUS=0.15`, `KERN_OUTER_RADIUS=0.35`).

**Gaps.** Routing does a vector lookup per level (O(depth·log n)); a cached
per-kern centroid could make root-level fan-out O(gravitons). Unnamed children
are currently unbounded per parent (only emptied by cluster eviction).

---

## 3. Bi-temporal supersede & contradiction — `active`

**What.** Conflicting claims *supersede* rather than delete. The old revision
stays as history with a stamped `valid_to`; `query` can recover the past via
`as_of` or walk the supersede chain via `include_history`.

**How.**

- `supersede_by_contradiction` (`src/base/accept.rs:400`) — inserts the new
  thought, sets the old `status=Superseded`, `superseded_by=new_id`, and
  `stamp_invalidated(now, new_valid_from)` so the window closes exactly when
  the new claim became true. Removes the old id from both vector indexes (so it
  stops seeding) but keeps it in the kern for history. Adds a `Supersedes`
  reason edge with the averaged vector.
- Classification is LLM-driven (`classify_prompt` `accept.rs:380` /
  `parse_contradiction` `accept.rs:390`) and **fails open to `Related`** (co-exist) — the
  conservative choice that never loses data. Driven from the tick's
  `do_classify_contradiction` task (`src/tick/tasks.rs:115`) so recall stays
  LLM-free at query time.
- `is_valid_at(instant)` / `valid_from_or_created()` on `Entity` answer
  point-in-time membership; the query layer's `include_history` walks the
  `superseded_by` chain.

**Where.** `src/base/accept.rs`, `src/base/types.rs` (temporal helpers),
`src/tick/tasks.rs` (background classification).

**Gaps.** Classification runs once per near-duplicate pair on the tick; a
re-classify when either side changes isn't triggered. No UI/MCP tool exposes
the history chain directly beyond `include_history`.

---

## 4. Retrieval pipeline — `active`

**What.** The hybrid query engine. Hand-rolled end to end (no external ANN or
rerank lib). This is the product's core IP.

**Stages** (`retrieve_profiled`, `src/retrieval/answer.rs:138`, each checkpoint
profiled via `src/profile.rs`):

| # | Stage | File | What happens |
| --- | ------- | ------ | -------------- |
| 1 | **Seed dense** | `retrieval/seed.rs` | HNSW top-`k` over a 0.4/0.6 blend of content + GNN vectors (`Weights::for_mode` per `Mode` Hybrid/Vector/Lexical/Reason). Plus `seed_important` — an O(N) scan feeding access/recency (`IMPORTANT_ACCESS_THRESHOLD=3`, `IMPORTANT_MIN_COSINE=0.20`) into both the dense merge and RRF, run once. |
| 2 | **Seed lexical** | `retrieval/seed.rs:98` | BM25 (`LexicalIndex`) candidate list, fused via RRF when `mode==Hybrid`. |
| 3 | **Fuse (RRF)** | `retrieval/fuse.rs` | Reciprocal-rank fusion of dense + lexical + important lists with mode weights. |
| 4 | **PageRank** | `retrieval/pagerank.rs` | Centrality weighting of the fused seeds over the reason graph. |
| 5 | **Expand** | `retrieval/expand.rs:178` | Walk reason edges out from seeds (`PathChain` recording the *why*), scoring neighbors (`score_neighbor`). Optional **HyDE** (`retrieval/hyde.rs`) generates a hypothetical answer to broaden recall. |
| 6 | **Merge** | `retrieval/merge.rs` | Combine seeds + expanded neighbors into `ScoredEntity` list. |
| 7 | **Boosts** | `retrieval/score.rs:79` | `apply_boosts`: confidence × score + **QBST** access/recency boost (`qbst`, capped at 0.1, 24h half-life) + `fact_score_boost` (0.3) for Facts. |
| 7b | **Gravity** | `retrieval/gravity.rs` | Query-time graviton pull: `score += gravity_weight (0.15) * max_over_gravitons(mass * max(0, cos(entity, graviton_vec)))`. Max, not sum — overlapping gravitons never double-count. `gravity_weight=0` disables (early return, zero cost); no gravitons → no-op. Bench 2026-07-19: recall/NDCG unchanged, ~+7% p50 with 5 gravitons. |
| 8 | **Filter** | `retrieval/score.rs:93` | Drop superseded; floor at `MIN_DELIVER_SCORE=0.40`; cap at `MAX_DELIVER_RESULTS=10` (MMR keeps a larger pool when on). Apply query options (source/kind/time/min_conf). |
| 9 | **Dedup by section** | `retrieval/diversify.rs:6` | Collapse near-duplicate sections. |
| 10 | **MMR** | `retrieval/diversify.rs:46` | Maximal-marginal-relevance diversification so the `k` results actually differ. |
| 11 | **Rerank** (opt) | `retrieval/rerank.rs` | LLM reranker reorders the head; `parse_ranking` recovers a permutation. |
| 12 | **Answer** (opt) | `retrieval/answer.rs:217` | `synthesize` glues top chains + thoughts into an LLM answer prompt (`ANSWER_MAX_CHAINS=5`, `ANSWER_MAX_THOUGHTS=5`). Prompt instructs declining with the exact `NO_ANSWER` string when context lacks the answer; empty context short-circuits to that string with no LLM call. `QueryOptions::answer_style` appends a caller-supplied style hint (eval uses it for short-fact answers; product default none). |
| 13 | **Cold backfill** | `base/store.rs:515` | If hot returns `< k`, cold-tier hits (brute-force `cold_search`) fill remaining slots, flagged `cold:true`. |
| 14 | **Query cache** | `retrieval/cache.rs` | LRU keyed on query-vector hash + tag (256 cap, θ=0.97 similarity). `lookup`/`lookup_text`/`insert`. `commit_access` deposits heat on every returned hit. |

**Where.** `src/retrieval/*` (4081 LoC, 13 files). Entry: `retrieval::query`
(one-shot CLI) and `retrieval::query_locked` (daemon, holds read lock only for
the graph phase; every LLM call runs unlocked).

**Gaps.** The O(N) importance scan runs every retrieve; at scale it should be
indexed. RRF weights and mode blends are config but not auto-tuned. No learned
rerank model — the LLM rerank is a cold call per query.

---

## 5. Indexes — `active`

**What.** Three hand-built approximate/brute indexes backing seed + dedup +
cold backfill.

**How.**

- **HNSW** (`src/base/hnsw.rs`, 1082 LoC) — id-stable, deterministic-build
  graph ANN. `insert`/`delete`/`search`/`search_batch`/`search_filtered`
  (pre-filtered ANN that shares one filter predicate with post-filtering).
  Quantization-aware: stores `QuantizedVec` (int8) when configured. `structure_digest`
  for parity checks.
- **DiskANN** (`src/base/diskann.rs`, 620 LoC) — disk-resident graph index.
  `build_and_save` (Params `r=32, build_l=64, alpha=1.2`) writes
  `meta.bin`/`vectors.bin`/`graph.bin`; `DiskIndex::open`/`search`/
  `search_hits_filtered`. Selected when a kern exceeds `disk_threshold`.
- **BM25 LexicalIndex** (`src/base/lexical.rs`) — in-RAM inverted index,
  `k1`/`b` tunable, `rebuild_from_graph`, `search_filtered`.
- **VectorBackend** (`src/base/vector_backend.rs`) — enum switch
  (`Resident(HnswIndex)` | `Disk(DiskIndex)`) unifying the search API so the
  retrieval layer is backend-agnostic.

**Where.** `src/base/{hnsw,diskann,lexical,vector_backend,search}.rs`.

**Gaps.** HNSW delete is logical (tombstone) — no compaction of dead nodes
in-graph. DiskANN is build-once; incremental updates funnel through
`consolidate_disk_index` on the tick. Lexical index is RAM-only.

---

## 6. Quantization — `active`

**What.** int8 (and float fallback) vector storage + distance, cutting vector
memory ~4×.

**How.** `QuantizationMode` (`None`/`Int8`/`Binary`, `src/quant.rs:8`; `Binary`
is implemented and tested but deliberately excluded from `parse`, so it is not
user-selectable — recall floor is too low without rescore),
`QuantizedVec::encode`/`decode`, `quantized_cosine_distance` /
`float_cosine_distance`. `INT8_MAX_ABS=127`. The HNSW index picks the mode at
build; both resident and disk backends honor it.

**Where.** `src/quant.rs` (485 LoC).

**Gaps.** No int4 / product-quantization path. Scale is fixed at encode time.

---

## 7. Persistence (LMDB) — `active`

**What.** One ACID LMDB env per data dir (`data.mdb` + `lock.mdb`); hot graph
and cold tier live together. Readers never block, writers serialize.

**How.**

- `Store::open` (`src/base/store.rs:230`) opens the env; `StoredKern`/
  `StoredVec`/`StoredTemporal` are the on-disk bincode shapes, values
  `zstd(bincode)`-compressed, vectors int8.
- **Guarded flush** (`flush_guarded`, `store.rs:411` + `persist.rs:283`) — a
  snapshot carries an expected `mutation_epoch`; if disk advanced under us
  (another writer / external edit), the flush is *refused*, the disk rows are
  *absorbed* back (`merge::absorb_graph`), and the flush retries. Prevents a
  stale in-memory snapshot from clobbering newer on-disk state.
- **Cold tier** — `cold_spill`/`cold_get`/`cold_all`/`cold_put_all`/
  `cold_search` (brute-force). Capped at `COLD_MAX_ENTRIES=50_000` (latest-wins
  keyed table).
- **Compaction** (`compact_dir`, `store.rs:568`) — the only way to shrink
  LMDB's high-water mark; writes a fresh env to a tmp file then `swap_compacted`
  renames with retry (25 attempts). Requires exclusive access (run offline).
- **Migration** (`src/base/migrate.rs`) — `migrate_dir` is a one-shot idempotent
  import of legacy per-kern bincode shards (`load_legacy_dir` → `save_graph_into`),
  exposed as `kern migrate`. Source shards left in place.
- **Snapshots** — `snapshot_for_flush` / `FlushSnapshot` capture a consistent
  point-in-time; the maintenance tick runs a mutation-epoch-gated snapshot so
  crash loss is bounded to one tick interval.

**Where.** `src/base/store.rs` (1123 LoC), `src/base/persist.rs` (490 LoC),
`src/base/migrate.rs`, `src/store.rs` (per-cwd `Registry` of open stores).

**Gaps.** Single-writer means CLI commands reading the on-disk graph can race a
live daemon (documented in README). No WAL beyond LMDB's own. Compaction is
manual/offline.

---

## 8. Intake & distillation (self-learning) — `active`

**What.** A conversation delta (`.txt`) dropped in `.kern/intake/` is drained,
run through one LLM pass, and turned into typed claims ingested into the graph.
Nothing is lost on an LLM outage — the delta stays queued until it succeeds.

**How.**

- **Intake** (`src/ingest/intake.rs`) — `run()` polls `.kern/intake/`,
  `extract_claims` distills, `archive`/`finalize` move processed deltas to a
  `done/` dir, `prune_done` ages them out.
- **Distill** (`src/ingest/distill.rs`) — a structured prompt asks the LLM for
  a JSON array of `{text, kind, valid_from?}` where `kind ∈ {preference,
  decision, project, fact, code-fact, reference, procedural}` (the 7 seeded
  descriptors). `Some([])` = nothing worth keeping (archive); `None` = no LLM
  output (transient outage, retry). `parse_claims` is lenient (finds the JSON
  array anywhere in the output).
- **Worker** (`src/ingest/worker.rs`) — async job queue (`enqueue`/`run`),
  owns the embed + accept path. Defers question/contradiction follow-ups to
  the tick via callback closures (`DeferQuestionsFn`/`DeferContradictionFn`).
- **Embed** (`src/ingest/embed.rs`) — batches texts to the embedding endpoint.
- **Dedup** (`src/ingest/dedup.rs`) — `find_duplicate` at
  `INGEST_DEDUP_THRESHOLD=0.95` (stricter than accept-time), `update_existing_entity`.
- **Place / split / direct** — `place.rs` builds chunk `Entity`s
  (`build_chunk_entity`, `chunk_source_id`), `split.rs` chunks by descriptor
  (LLM-assisted when given), `direct.rs` handles `.kern/intake/direct/` synchronous
  ingest (`drain_direct_once`).
- **File watcher sink** (`src/ingest/file_watcher.rs`) — `KernFileWatcherSink`
  adapts the repo file watcher into ingest jobs.
- **Outcome** (`src/ingest/outcome.rs`) — `OutcomeStatus` (`Committed`/`Partial`/`Deduped`/`Failed`, `src/ingest/outcome.rs:2`),
  `FailureReport::document_permanent` for non-retryable errors.

**Where.** `src/ingest/*` (2673 LoC, 12 files). Spawned by `spawn_intake`
(`src/commands.rs:807`).

**Gaps.** Distill prompt is one-shot; long deltas may truncate. No per-descriptor
prompt tuning. Dedup threshold is global, not per-descriptor.

---

## 9. Self-compaction (tick) — `active`

**What.** A background task queue drives heat decay, clustering, naming,
enrichment, GC, GNN propagation, and persistence. An idle daemon still
maintains itself.

**How.**

- **Queue** (`src/tick/queue.rs`) — bounded (`TICK_QUEUE_CAPACITY=512`) mpsc
  with backpressure, `TaskKind` enum (Cluster/SeedQuestions/
  ClassifyContradiction/Name/Enrich/ResolveQuestion/Persist/GnnPropagate/
  StigmergyGc/Reembed/DiskConsolidate/CommitAccess). Records per-task latency
  and pending/done metrics.
- **Driver** (`tick::start`, `src/tick.rs:37`) — one async task drains the
  queue and dispatches via `process_task`. `tick_sync` is the synchronous
  one-shot variant (used by CLI `--sync`). `enqueue_all` fans a Cluster task
  out to every non-empty kern.
- **Maintenance tick** (`spawn_maintenance_tick`, `commands.rs`) — periodic
  driver at `TICK_INTERVAL_SECS=60` (0 = event-driven only): pulses heat,
  gates GC/consolidation on clock + interval (`pulse::should_run_gc`/
  `should_consolidate`), enqueues persist.
- **Pulse** (`src/tick/pulse.rs`) — `pulse_with_heat` (`src/tick/pulse.rs:20`) re-deposits heat
  on entities reachable from the root, decaying strength by `PULSE_DECAY=0.5`
  per level; below `PULSE_THRESHOLD=0.05` it stops. Heat itself decays lazily
  by age (`heat::decayed`, half-life based), *not* per tick.
- **Cluster** (`src/tick/cluster.rs` + `tick::do_cluster`) — `vector_cluster`
  samples up to `TICK_MAX_CLUSTER_SAMPLE=200` entities, groups them; a cluster
  that is `≥ KERN_MIN_CLUSTER_SIZE=10` and `cohesion ≥ KERN_COHESION_THRESHOLD=0.60`
  and not a core cluster spawns a distinct unnamed child and migrates its
  members. Unnamed kerns never spawn (bounds descent). Empty unnamed children
  are evicted back to the parent each pass.
- **Name** (`do_name`, `tasks.rs:225`) — LLM names an unnamed kern from its
  centroid (`cluster::graviton_prompt`) once it crosses the naming thresholds
  (`cohesion ≥ 0.50`, size ≥ 5).
- **Enrich** (`do_enrich`, `tasks.rs:304`) — LLM writes the explanatory text
  for an un-enriched reason edge.
- **Resolve question** (`do_resolve`, `tasks.rs:372`) — open `Question` edges
  (`to` empty) get answered by retrieval; if a hit scores above
  `QUESTION_RESOLVE_THRESHOLD=0.80` the edge is closed.
- **Seed questions** (`do_seed_questions`, `tasks.rs:42`) — broadcasts open
  questions to peers (federation).
- **Commit access** (`do_commit_access`, `tasks.rs:448`) — flushes queued
  access-count/heat updates.
- **Persist / reembed / disk consolidate** — `do_persist`, `do_reembed`,
  `do_disk_consolidate`.

**Where.** `src/tick/*` (2442 LoC, 6 files) + `src/tick.rs`.

**Gaps.** `KERN_CAP_DISABLED` (no per-kern entity cap) — comment marks it
"currently unsafe" to enable. Clustering is vector-only; no semantic/structural
features. Naming/enrich are LLM-cold per kern.

---

## 10. Stigmergy GC — `active`

**What.** Cold, stale, non-durable thoughts evict themselves; **Facts and
Documents are immune while Active** (immunity is revoked once superseded);
evictions spill to the cold tier before dropping (spill-before-drop). Spill is
lossless out of RAM, not lossless overall — the cold tier is capped at
`COLD_MAX_ENTRIES = 50_000` and `Store::cold_cap` (`src/base/store.rs:541`)
deletes the oldest rows past it, and with no store bound `run_gc` drops the
victim outright (`src/tick/stigmergy.rs:56`).

**How.** `stigmergy::run_gc` (`src/tick/stigmergy.rs:32`) collects victims per
kern where `is_cold_victim` holds (heat below `COLD_HEAT_THRESHOLD=0.01` *and*
not accessed within `COLD_GC_AGE = 7 days` *and* not an Active `Fact`/`Document`,
`src/tick/stigmergy.rs:14`), spills each
to the cold store, and only on spill success calls `remove_entity`. A failed
spill keeps the victim hot and retries next pass. Runs on the maintenance tick
gated by `STIGMERGY_GC_INTERVAL = 1 hour` and clock validity.

**Where.** `src/tick/stigmergy.rs`, `src/base/reason.rs` (`remove_entity`
cascade-deletes its edges).

**Gaps.** Victim selection is per-kern linear. No priority/age queue. Cold tier
is brute-force search only.

---

## 11. GNN (learned structure re-embedding) — `active`

**What.** A from-scratch graph neural network that re-embeds each thought from
*graph structure* (not just content), so the dense seed blends content + structure.
Trained per-kern on the tick.

**How.**

- **Graph** (`src/gnn/graph.rs`) — `add_node`/`add_edge`/`add_self_loops`/
  `normalized_adjacency` (symmetric normalized adjacency matrix as a `Tensor`),
  `feature_matrix`.
- **Layers** — `LinearLayer` (`src/gnn/layer.rs`), `GCNLayer`
  (`src/gnn/gcn.rs`: linear + optional `LayerNorm` + `Dropout` + `Activation`),
  `LayerNorm` (`src/gnn/norm.rs`), `Dropout` (`src/gnn/dropout.rs`).
  Activations (`src/gnn/activation.rs`): ReLU/LeakyReLU/GELU/Sigmoid/Tanh +
  derivatives.
- **Model** (`src/gnn/model.rs`) — `Model::new_residual` stacks
  `Box<dyn BackwardGraphLayer>`, `forward`/`backward`, `parameters(_mut)`,
  `param_grads(_mut)`, `zero_grads`, `set_training`. Manual autograd via
  `backward.rs` (`GraphLayer`/`BackwardGraphLayer` traits).
- **Training** (`run_learned_propagation`, `src/gnn/propagate.rs:61`) — builds
  a `GnnSnapshot` (features + positive reason edges + last weights), samples
  negative edges, trains a 2-layer GCN (`dim → dim/2 → dim`) for
  `DEFAULT_TRAIN_EPOCHS=24` with `Adam` (`DEFAULT_TRAIN_LEARNING_RATE=0.01`)
  minimizing `link_prediction_loss` (sigmoid dot-product over pos/neg edges,
  `src/gnn/loss.rs`). Output embeddings blended with input features at
  `DEFAULT_SELF_WEIGHT=0.6`, normalized, written back as `gnn_vector`.
  Requires `≥ DEFAULT_MIN_THOUGHTS=128` thoughts.
- **Optimizers** (`src/gnn/optim.rs`) — `SGD` (+momentum), `Adam`.
- **Persist** (`src/gnn/persist.rs`) — `marshal_weights`/`unmarshal_weights`/
  `save_weights`/`load_weights` (versioned `WEIGHT_FILE_VERSION=1`).
- **Tensor** (`src/gnn/tensor.rs`, 371 LoC) — own 2D tensor + matmul.

**Where.** `src/gnn/*` (2905 LoC, 14 files). Driven by `tick::gnn_propagate::do_gnn_propagate`.

**Gaps.** Training is synchronous on the tick (can stall a large kern). No GPU.
Weights are per-kern, not shared across the tree. Link prediction only — no
node-classification objective.

---

## 12. MCP surface — `active`

**What.** Model Context Protocol server (stdio + HTTP/SSE) exposing the graph
to external clients (Claude, Cursor, etc.). Protocol version `2024-11-05`.

**Tools** (10, defined in `src/mcp/tools*.rs`, dispatched in `mcp.rs`
`call_tool`):

| Tool | File | Purpose |
| ------ | ------ | --------- |
| `query` | `tools_query.rs` | Hybrid search + optional LLM answer. Filters: `mode`/`kind`/`source`/time range/`min_conf`/`as_of`; `include_history` for supersede chain. |
| `ingest` | `tools_mutate.rs` | Add text. `object_id` update semantics, `descriptor` chunking. |
| `link` | `tools_mutate.rs` | Create a reason edge (LLM writes the reason if blank). |
| `forget` | `tools_mutate.rs` | Remove a thought + cascade edges (Facts immune). |
| `degrade` | `tools_mutate.rs` | Down-weight edges along a bad retrieval path (`DEGRADE_*` decay). |
| `health` | `tools_admin.rs` | Graph stats: gravitons/kerns/entities/reasons/unnamed/descriptors. |
| `graviton` | `tools_admin.rs` | list/add/remove focus attractors (name + text — phrase or full document — + optional mass). Replaced the single per-kern "purpose". |
| `descriptor` | `tools_admin.rs` | add/remove data-type descriptors. |
| `pulse` | `tools_admin.rs` | Trigger a clustering pass across the tree. |
| `gc` | `tools_admin.rs` | Live reap of empty/orphan kerns (`kern_gc`). |

Plus MCP **prompts** (`src/mcp/prompt.rs`) and **resources** (`src/mcp/resources.rs`).

**Server** (`src/mcp.rs`) — `Server` holds the shared `graph`/`worker`/`llm`/
`cache`/`task_q`/`cfg`; implements `trnsprt::McpServer`. `run`/`run_stdio` use
the trnsprt framing. `run_sse` (`src/mcp/sse.rs`) serves HTTP/SSE.

**Where.** `src/mcp/*` (2213 LoC, 7 files).

**Gaps.** No streaming `answer` over stdio (SSE only). Tool schemas are hand-
rolled JSON, not derived. No batch query.

---

## 13. RPC surface (tarpc) — `active`

**What.** A `KernRpc` tarpc server over a per-cwd Unix socket for local clients
that want the full graph API without MCP framing.

**How.** `KernRpcHandler` (`src/rpc/kern_rpc_server.rs:17`) wraps the same
`mcp::Server` and implements the `KernRpc` trait (122 methods + helpers, 712
LoC). `serve_kern_rpc_loop` accepts on a `LocalListener`. The DTO/mock layer
lives in `src/trnsprt/src/kern_rpc/{dto,mock,svc,client_local}.rs`.

**Where.** `src/rpc/*` (715 LoC), `src/trnsprt/src/kern_rpc/` (931 LoC).

**Gaps.** tarpc pulls a heavyweight dependency; the socket has no auth. The
trait surface mirrors MCP one-to-one (drift risk).

---

## 14. CLI — `active`

**What.** The `kern` binary. Reads the on-disk graph directly (can race a live
daemon — prefer MCP for live state).

**Subcommands** (`Commands` enum, `src/commands.rs:64`): `ingest`, `query`,
`search`, `reembed`, `get`, `list`, `forget`, `link`, `health`, `profile`,
`gc`, `compact`, `graviton {add|list|remove}`, `degrade`, `descriptor {add|rm}`,
`peers`, `register`, `unnamed {list|promote}`, `mcp`, `compress`, `migrate`,
`daemon`.

**How.** `dispatch` (`commands.rs:404`) routes; per-subcommand handlers in
`src/commands/{admin,graph_ops,ingest_cmd,mcp_cmd,profile_cmd,query,reembed}.rs`.
Notable:

- `reembed` (`reembed.rs`) — re-embeds every entity with a new model in batches
  of 64, re-seeds `gnn_vector` from the raw embed, recomputes reason-edge
  vectors (endpoint means), rebuilds the index, saves. Daemon must be stopped.
- `profile` (`profile_cmd.rs`) — runs a query with a `Profiler` timeline.
- `compress` (`admin.rs`) — compresses vectors with a chosen `QuantizationMode`.
- `migrate` — legacy shard → LMDB.
- `daemon` / `run_server` (`commands.rs:650`) — boots the full runtime: loads
  graph, spawns watchdog, LLM keepalive, file watcher, the intake, gossip,
  maintenance tick, MCP (stdio or SSE), and the RPC socket.

**Where.** `src/commands/*` (1859 LoC), `src/main.rs`.

**Gaps.** CLI vs daemon race is a documented footgun. No `kern status` to
check for a running daemon. `unnamed promote` is manual.

---

## 15. Federation (gossip + CRDT) — `building`

**What.** Opt-in LAN knowledge sharing with no coordinator. Each node
heartbeats peers and merges entity bodies via content-addressed CRDTs — a
thought ingested on node A becomes searchable on node B under the same id.

**How.**

- **Node** (`src/gossip/node.rs`) — TCP listener, `Lamport` clock
  (`bump_lamport`/`observe_lamport`), peer list (`GOSSIP_MAX_PEERS=50`),
  `broadcast` with `GOSSIP_FANOUT=3`, `fetch_thought` RPC
  (`GOSSIP_FETCH_TIMEOUT=5s`), `start_heartbeat`
  (`GOSSIP_HEARTBEAT_INTERVAL=30s`), `GOSSIP_MAX_FRAME_BYTES=4MB` bounds.
- **Discovery** (`src/gossip/discovery.rs`) — multicast announce/parse on
  `GOSSIP_DISCOVERY_MULTICAST=239.77.75.68:7475` every
  `GOSSIP_DISCOVERY_INTERVAL=10s`. Only pairs nodes sharing the same
  `network_id`.
- **Handler** (`src/gossip/handler.rs`, 803 LoC) — `start_announce`,
  `start_entity_sync` (broadcasts top-32 hottest entities every heartbeat),
  `start_delta_flush` (drains `GraphGnn`'s pending CRDT deltas), and inbound
  handlers for Sphere/Question/Pulse/PeerExchange/Fetch/CrdtDelta/EntitySync.
- **CRDTs** (`src/crdt.rs`) — `GCounter`, plus the shared `lww_wins` comparison
  the last-writer-wins call sites now route through (`join_lww_time`
  `src/base/merge.rs`, `merge_reason`, and both `gossip/handler.rs` sites). There
  is no `PnCounter`/`LwwRegister`/`OrSet` type anywhere in the tree. Applied to
  four live `CrdtTarget`s (`src/gossip/types.rs`): `ThoughtAccessCount`
  (GCounter), `ReasonTraversalCount` (GCounter), `ReasonScore` (LWW),
  `ValidUntil` (LWW). This replaces the old wall-clock max-join. `Statements` is
  inert by design — it has no sender, and statements are deliberately never
  imported because entity ids are `content_hash(text)`, so merging them can only
  admit content an id does not hash to.
- **Merge** (`src/base/merge.rs`) — `merge_entity`/`merge_reason`/
  `merge_remote_entity`/`absorb_graph` apply remote bodies into local kerns
  under `remote-` prefixed ids, capped at `GOSSIP_REMOTE_KERN_ENTITY_CAP=50_000`.
- **Seen / Ledger** (`src/gossip/seen.rs`, `ledger.rs`) — `SeenSet` dedup
  (`GOSSIP_SEEN_SET_CAP=10_000`, TTL 60s); `Ledger` routes thought-id and
  kern-id → peer addr (`LEDGER_THOUGHT_TTL=72h`, `LEDGER_ROUTING_TTL=5min`).

**Status.** Entity-body sharing is verified on a single host with manually
seeded `peers` (the reliable path). Multicast discovery only pairs same-
`network_id` nodes. The **Delta, Pulse and Question senders are all live**
(`src/gossip/handler.rs:135`, `src/commands.rs:897`, `src/commands.rs:911`,
driven from `src/tick/tasks.rs:439`). The **fetch RPC is the dead path**: the
server side answers (`src/gossip/node.rs:219`) but `set_fetch_handler`
(`src/gossip/node.rs:56`) is never called and `fetch_thought`
(`src/gossip/node.rs:138`) has no caller, so every reply is `found: false`.
OR-Set deltas for `statements` never send either — `src/gossip/handler.rs:160`
hardcodes an empty `orset_delta`, so the receive path at `:384` is unreachable
over the wire; statements still converge through full EntitySync bodies.
Federation tuning at scale (batch size, push vs pull, anti-entropy) is open.

**Security.** **Unauthenticated and unencrypted.** See `docs/FEDERATION-SECURITY.md`.
Off by default.

**Where.** `src/gossip/*` (1817 LoC, 7 files), `src/crdt.rs`, `src/base/merge.rs`.

**Gaps.** No auth/crypto. No anti-entropy merkle/snapshot exchange — EntitySync
ships the hottest 32 by heat per heartbeat, so cold entities may never
propagate. Fetch RPC unwired. OR-Set delta send unimplemented. No backpressure
on remote-id cap (drops new, keeps known).

---

## 16. LLM client — `active`

**What.** One client wrapping three endpoints (reason / answer / embed) against
Ollama by default; fail-open everywhere.

**How.** `Client` (`src/llm.rs:64`) — `embed`/`embed_batch` (embedding
endpoint), `complete` (reason / distillation), `answer` (streamed answer via
Ollama native `/api/chat`), `complete_func` (sync closure for the tick/ingest
blocking bridges). `is_transient` classifies retryable errors. `Endpoint`
holds url/model/key. `for_eval(seed)` makes it deterministic for benchmarks.

**Where.** `src/llm.rs` (861 LoC).

**Gaps.** Ollama-centric; OpenAI-compatible only via manual url/key. No
embedding-dimension validation at config time (dimension locks the graph —
`reembed` is the only escape). No retry/backoff policy object.

---

## 17. Profiling — `active`

**What.** Lightweight per-phase timing for queries and the tick.

**How.** `Profiler` (`src/profile.rs`) records labeled `Checkpoint`s with
`Instant`; `finish` produces a `Profile`; `render_timeline` draws an ASCII
Gantt. Used by `retrieve_profiled` and the `profile` CLI.

**Where.** `src/profile.rs` (293 LoC).

---

## 18. Transport layer (`trnsprt` crate) — `active`

**What.** A reusable MCP framing + multi-server registry, factored into its own
workspace crate so other tools can embed kern as one server among many.

**How.**

- **Transport** (`transport.rs`) — `Transport` trait, `ChildStdio::spawn`.
- **Server** (`server.rs`) — `McpServer` trait, `serve_stdio`/`serve_rw`
  (JSON-RPC over any reader/writer). Protocol `PROTOCOL_VERSION=2024-11-05`.
- **Client** (`client.rs`) — `Client::initialize`/`list_tools`/`call_tool`.
- **HTTP / InProc** (`http.rs`, `inproc.rs`) — `serve_http` (axum),
  `InProcTransport` (server-in-process).
- **Registry** (`registry.rs`) — `Registry` of `LiveServer`s, `spawn_stdio`/
  `register_inproc`, aggregated `list_tools`/`call_tool` across servers.
- **Typed** (`typed/`) — `adapter`/`channel`/`codec` for typed RPC over the
  wire. **kern_rpc/** — DTO + mock + svc + local client for the tarpc surface.
  **search/** — a parallel typed search service. **macros/** (`trnsprt-macros`)
  derives boilerplate.

**Where.** `src/trnsprt/` (workspace member, ~2600 LoC).

**Gaps.** Two parallel typed surfaces (kern_rpc + search) with overlapping DTOs.
No connection pooling in the client.

---

## 18a. Hub — machine-level control plane — `active`

**What.** `kern hub` is a per-machine supervisor: one socket (`kern-hub.sock`),
a routing table of project root → node daemon. Clients resolve a root through
the hub; the hub spawns the node if absent (or adopts an externally started
daemon), unloads it gracefully on request, auto-unloads idle nodes, and merges
one project's store into another offline. The data path stays direct
client→node — the hub is connect-time only, never a proxy hop.

**How.**

- **hub_rpc** (`trnsprt/src/hub_rpc/`) — `resolve(root)` / `status` / `unload`
  service + `connect_hub` client. `Endpoint::hub()` (machine-scoped),
  `Endpoint::kern_for(root)` (hub computes a node's socket without chdir).
- **Supervisor** (`src/hub/`) — `node.rs` spawn/probe/ready-wait/shutdown,
  `serve.rs` handler + accept loop + dead-node reaper. Hub exit leaves nodes
  running; a restarted hub re-adopts them via probe.
- **Graceful unload** — `KernRpc::shutdown` fires the daemon's save-then-exit
  path (no signals, works on Windows named pipes too).
- **Idle auto-unload** — nodes report `HealthRes.idle_ms` (last real tool call,
  health polls excluded); the hub reaper re-checks under the per-root lock and
  unloads hub-owned nodes past `--idle-unload-secs` (default 1800, 0 off).
  Adopted nodes are exempt; `idle_ms == 0` (pre-field daemon) is never trusted.
- **Cross-kern merge** — `kern hub merge <src> <dst>`: stops both daemons,
  offline CRDT union via `base::merge::absorb_graph`, src never written.
- **Hub-first proxy + auto-start** (`commands/mcp_cmd.rs`) — `kern mcp` asks
  the hub first, auto-starting a detached hub when none answers
  (`[hub] auto_start = false` opts out); any failure falls through to the
  legacy direct-connect/auto-spawn path. `kern hub stop` ends the hub over
  RPC; nodes stay up.

**Where.** `src/hub/`, `src/trnsprt/src/hub_rpc/`, `commands/admin.rs::cmd_hub`,
`src/config/hub.rs`, `tests/hub_supervisor.rs`.

**Gaps.** Gossip still lives in each node; the transport moves hub-side
together with §5's TLS work (ordering recorded in ROADMAP §5x). Version skew
hub↔node unmanaged beyond same-binary spawning.

---

## 19. File watcher (`watcher` crate) — `active`

**What.** Watches repo roots and turns file events into ingest records.

**How.** `FileWatcher` (`src/watcher/src/watcher.rs`) wraps `notify`, emits
`WatchEvent`s (`event.rs`: Create/Modify/Remove). `IgnoreRules`
(`ignore_rules.rs`, built `from_roots` reading `.gitignore`-style patterns)
filters noise. `IngestPipeline` (`pipeline.rs`) debounces, caps at
`MAX_INGEST_BYTES=1MB`, and pushes `IngestRecord`s to an `IngestSink`
(kern's is `KernFileWatcherSink`).

**Where.** `src/watcher/` (workspace member, 721 LoC + tests).

**Gaps.** `.gitignore` parsing is approximate (no full spec). No rename
tracking.

---

## 20. Config — `active`

**What.** Layered TOML config, all-optional (works zero-config against local
Ollama).

**How.** `Config` (`src/config/mod.rs`) aggregates 14 sub-configs:
`Embed`, `Reason`, `Answer`, `Intake`, `Tick`, `Gossip`, `Gnn`, `Graph`,
`Ingest`, `Retrieval`, `Serve`, `Watcher`, `Hub`, `Heat`. Resolved
project-scope (`<cwd>/.kern/kern.toml`) over user-scope
(`<XDG_CONFIG>/kern/kern.toml`). `Config::resolve_root` walks up to the
nearest `.kern/` ancestor. Under WSL2 NAT a loopback Ollama URL must be
pinned to the Windows host gateway in `kern.toml` — kern does not rewrite
URLs.

**Where.** `src/config/*` (1418 LoC, 15 files).

**Gaps.** No env-var override layer. Secrets (API keys) stored in plaintext
TOML. Section-level replace rather than deep merge (`src/config/io.rs`)
means a project section silently drops keys the user section set.
(`Config::validate` `src/config/mod.rs:140`, called from `src/main.rs:44`, does
validate embed url/model and delegates to the sub-validators.)

---

## 21. Bench & eval — `removed 2026-07-20`

The LoCoMo end-to-end eval, the retrieval bench, both feature-gated binaries
and the `bench` feature are deleted. They measured
`ingest x retrieval x answering` as one LLM-judged number, which is dominated
by the answerer: a grounded run (whole conversation in the prompt, kern
bypassed) scored 0.187, so answer quality — not memory — set the ceiling, and
three prompt tweaks moved the score more than any retrieval change.

A replacement is planned around the retrieval layer alone: recall@k / MRR /
NDCG against LoCoMo's per-turn `evidence` labels, no LLM in the loop. It needs
turn-level claim provenance, which ingest does not record yet.


## 22. Cross-cutting utilities

- **math** (`src/base/math.rs`) — `cosine`, `cosine_distance`, `l2_normalize`,
  `average_vec`, content-hash `reason_id`, `OnlineSoftmax`, `softmax_merge_scores`,
  `clamp_confidence` (caps AI confidence at `MAX_AI_CONFIDENCE=0.95`, Facts at 1.0).
- **util** (`src/base/util.rs`) — `content_hash`, `now_nanos`, `cmp_rank`
  (deterministic tiebreak on score then id), token estimation.
- **time** (`src/base/time.rs`) — clock helpers (graceful on unreadable clock).
- **locks** (`src/base/locks.rs`) — `read_recovered`/`write_recovered` wrappers
  that survive a poisoned lock by recovering the last good graph (crash-
  resilience for the daemon).
- **health** (`src/base/health.rs`) — `graph_health_stats` (graviton/kern/entity/
  reason/unnamed counts).
- **descriptors / constants** (`src/base/{descriptors,constants}.rs`) — the 7
  seeded descriptor kinds + all magic numbers in one file.
- **log** (`src/log/` workspace crate), **test-utils** (`src/test-utils/`) —
  shared workspace helpers.

---

## 23. Improvement opportunities (consolidated)

Ranked by leverage:

1. **O(N) importance scan per retrieve** (`retrieval/seed.rs`) — index it; it's
   the scaling cliff at query time.
2. **Federation security** — add auth + encryption before any real deployment
   (`docs/FEDERATION-SECURITY.md`). Pulse/Question senders still partial.
3. **Per-kern entity cap** — `KERN_CAP_DISABLED` today; a safe cap + escalation
   policy would bound memory deterministically.
4. **CLI vs daemon race** — add `kern status` + advisory locking so the CLI
   can't clobber a live graph.
5. **GNN training is synchronous** on the tick — move to a background thread
   pool or incremental updates to avoid stalling large kerns.
6. **Distill prompt** is one-shot and global — per-descriptor prompts +
   chunking for long deltas would raise claim quality.
7. **HNSW tombstone compaction** — dead nodes accumulate in-graph; a periodic
   rebuild-and-swap would reclaim them.
8. **Query cache keyed on vector hash only** — semantically near-identical
   queries miss; a small ANN-over-queries layer would raise hit rate.
9. **No learned rerank model** — every rerank is a cold LLM call; a small
   cross-encoder trained on `degrade` feedback could replace it.
10. **Two parallel typed transport surfaces** (kern_rpc + search) with
    overlapping DTOs — consolidate.

---

*Scraped from source at `v1.1.0` (commit `b29ae13`). Update this file when a
subsystem's public surface changes — it is the canonical feature inventory.*
