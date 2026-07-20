# DiskANN-style disk-resident index — design

**Status (updated 2026-06-12): WIRED (opt-in, entity index only).** DiskANN now
serves the live entity vector search above a configurable threshold — it is the
architecture's designated answer to the unbounded resident-set ceiling (see
`src/config/graph.rs`: huge-corpus scaling is "the DiskANN index's job, not this
cap"; `src/base/constants.rs`: no entity-eviction cap ships, so a resident kern's
in-RAM HNSW grows unbounded).

How it works:
- `GraphGnn`'s entity/gnn/reason indices are a `VectorBackend` enum
  (`src/base/vector_backend.rs`): `Resident(HnswIndex)` or `Disk { snapshot:
  DiskIndex, delta: HnswIndex, tombstones }`.
- `rebuild_index` spills `entity_idx` to a `<data_dir>/diskann/entity` DiskANN
  snapshot once the resident searchable-entity count exceeds `[graph]
  disk_threshold` (default `KERN_CAP_DISABLED` = **never spill**, so small
  deployments are byte-for-byte unchanged). A build/open failure falls back to the
  in-RAM index — a disk error never breaks the graph.
- Post-snapshot writes buffer in the in-RAM `delta` (with tombstones shadowing
  stale/removed snapshot ids). A tick-driven `DiskConsolidate` task folds the
  delta back into a fresh snapshot once it grows past
  `DISK_CONSOLIDATE_MIN_DELTA`, at most hourly, so the delta stays bounded.

Still standalone (`src/base/diskann.rs`): `build_and_save` + mmap
`DiskIndex::open`/`search`, recall@10 ≥ 0.90 vs brute force.

**Not yet done (follow-ups):** `gnn_entity_idx`/`reason_idx` still stay resident
(entity-only spill); no product quantization yet — `DiskIndex` mmaps full `f32`
vectors (PQ-in-RAM, the RAM-of-codes optimization below, is the next step).
Execution plan: `docs/superpowers/plans/2026-06-12-diskann-wiring.md`.

> **Reality drift since this doc was written.** The original Phase-1 target below
> (replace `cold.rs`'s O(n) JSONL scan) is OBSOLETE: `cold.rs` and `persist.rs`
> were replaced by an LMDB store (`src/base/store.rs`) with int8-on-disk vectors,
> and `Store::cold_search` is now a BOUNDED scan (capped by `COLD_MAX_ENTRIES`),
> so the cold tier no longer degrades linearly. What DiskANN fixes today is the
> **hot/resident** ceiling: per loaded kern the in-memory `HnswIndex` holds every
> entity vector on the heap and is rebuilt on load, so RSS and load-time grow with
> the kern without bound. PQ (vectors compressed in RAM) is still unbuilt; the
> current `DiskIndex` mmaps full f32 vectors, which already removes them from the
> resident heap — PQ is a later RAM-of-codes optimization, not a prerequisite.
> The "ceiling today" list below is retained for historical context.

## The ceiling today

Three things keep the whole corpus in memory and bound it to a single host's RAM:

1. **`HnswIndex` is in-memory** (`src/base/hnsw.rs`). Nodes, the layered graph,
   and quantized vectors all live on the heap; rebuilt from the graph on load.
2. **The graph is a full-RAM bincode blob** (`src/base/persist.rs`). `load_dir`
   decodes an entire kern (`Entity { vector: Vec<f64>, gnn_vector: Vec<f64>, … }`)
   into memory; `save_all` re-encodes it. Load time and RSS scale with corpus.
3. **The cold tier is an O(n) linear scan** (`src/base/cold.rs`):
   `search()` reads `cold.jsonl` and computes cosine against every row.

Quantization exists but is **scalar int8 only** (`src/quant.rs`:
`QuantizationMode::{None, Int8}`) — no product quantization yet.

So: a kern with millions of thoughts won't load, won't fit, and cold recall
degrades linearly. That is the corpus-size wall.

## Approach: Vamana + PQ-in-RAM + full-vectors-on-disk

Standard DiskANN decomposition, mapped onto kern's existing pieces:

- **Vamana graph** — a single-layer, long-range-pruned proximity graph (the
  "α-pruning" RobustPrune). kern's HNSW beam search (`beam_search`,
  `prune_neighbors`, the Min/Max heaps) is ~80% of what a Vamana searcher needs;
  the deltas are single-layer (drop `random_level`/layer loop) and disk-resident
  adjacency.
- **PQ-compressed vectors in RAM** — product-quantized codes (e.g. 32–64 bytes
  per vector) kept resident for the approximate distance during graph traversal.
  This is the new quantization mode: extend `QuantizationMode` with `Pq { m, nbits }`
  and a trained codebook (k-means per subspace). `quantized_cosine_distance`
  already abstracts the distance call site.
- **Full vectors on disk, memory-mapped** — exact `f64`/`f32` vectors in a flat,
  fixed-stride file (`vectors.bin`), read on demand via `memmap2` to rerank the
  beam's survivors. Adjacency lives in a parallel `graph.bin` (fixed out-degree
  R, so node i's neighbors are at `i*R`). Search = traverse on PQ codes, fetch a
  bounded number of full vectors for final rerank.

Net: RAM holds PQ codes + the mmap'd page cache, not full vectors. RSS drops from
`O(N·dim·8)` to `O(N·pq_bytes)`.

## Incremental rollout (lowest risk first)

**Phase 1 — disk ANN over the cold tier.** Replace `cold.rs`'s linear scan with
a Vamana index built over the cold store. Self-contained: the cold tier is
already append-only, already a separate file, already the fallback path in
`query`. Win: cold recall goes from O(n) to O(log n) with no change to the hot
path. This is the recommended first slice — it exercises the whole Vamana +
mmap + PQ stack on the least-critical tier.

**Phase 2 — disk-backed hot index for large kerns.** A per-kern threshold
(`[graph] max_resident` or similar): below it, today's in-RAM HNSW; above it,
the kern's vectors+adjacency spill to disk and `search` runs the disk path. The
graph metadata (ids, edges, heat, confidence) can stay in RAM far longer than
the vectors — vectors are the bulk.

**Phase 3 — streaming inserts + deletes.** DiskANN is batch-built by default;
kern ingests continuously. Adopt FreshDiskANN semantics: an in-RAM delta index
for recent inserts, periodic merge into the on-disk Vamana, tombstones for
`forget`/GC, consolidation on the tick. The tick worker already owns periodic
maintenance, so the merge/consolidate job slots in next to stigmergy GC.

## Open questions / risks

- **PQ codebook training & drift.** Codebooks need training data and go stale as
  the embedding distribution shifts. When/where to (re)train — on a tick? On
  model swap (which already forces a clean re-embed)? A bad codebook silently
  degrades recall.
- **mmap on Windows.** `memmap2` works cross-platform but file-locking and
  flush semantics differ; the daemon is per-cwd and single-writer, which helps.
- **Incremental Vamana quality.** Naive incremental inserts degrade the graph;
  RobustPrune + periodic full rebuild is the usual answer. Needs a recall
  regression harness (no bench harness exists yet — see CHANGELOG 2026-07-20).
- **Crash consistency.** Disk graph + vectors + the bincode metadata must not
  diverge on a mid-write crash. Write-ahead or atomic rename per segment.
- **Compatibility.** Per repo policy ("no compat, clean base"), the disk format
  is introduced as the only format for large kerns; small kerns keep the
  in-RAM path. No on-disk migration shim.

## Reusable building blocks already in tree

- `src/base/hnsw.rs` — beam search, neighbor pruning, heaps (adapt to 1 layer).
- `src/quant.rs` — quantization seam + int8; extend with PQ.
- `src/base/store.rs` — the LMDB store + cold tier (the original Phase-1
  target, `src/base/cold.rs`, was absorbed here).
- `src/base/vector_backend.rs` — the `Resident`/`Disk` backend seam the wiring
  landed on.
