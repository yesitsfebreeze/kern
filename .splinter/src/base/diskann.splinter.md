# src/base/diskann.rs — commentary

Memory-mapped design: query-time RSS is the OS page cache for the touched vectors, not the whole corpus. On-disk counterpart to the in-memory `crate::base::hnsw::HnswIndex`; design doc at `docs/kern/diskann-disk-index.md`. Wiring it into the daemon's hot graph is a separate, reviewed step — the live memory store's integrity is untouched until then.

- `Params`: defaults (r=32, build_l=64, alpha=1.2) follow common DiskANN guidance.
- `build_and_save`: two build passes per the Vamana recipe — first with α=1.0, then the configured α; visit order reshuffled per pass with Fisher–Yates on the seeded RNG. After pruning each node, back-edges are added and over-full neighbours re-pruned.
- `build_and_save`: a stale comment claimed mixed-dimension vectors are dropped; the code never dropped them (dim is taken from the first item, all items are written). Comment deleted 2026-07-17 — if mixed-dim input is a real concern, filtering still needs to be implemented.
- `ids`: consumed by the `VectorBackend::Disk` overlay to count live (non-tombstoned) vectors and, later, to fold the in-RAM delta back into a rebuilt snapshot.
Second-pass migration (2026-07-17):
- Module-doc layout block compressed onto `write_files`: `meta.bin` bincode `Meta { dim, count, r, entry, ids }`; `vectors.bin` count×dim f32 LE fixed stride; `graph.bin` count×r u32 LE padded with SENTINEL. vectors/graph are mmap'd at `open`; only meta is read into RAM. Search is a memory-mapped beam walk over a single-layer graph.
- `search`: larger `search_l` trades latency for recall.
- `search_hits`: the similarity convention (`1.0 - distance`) matches `HnswIndex::search` so hits fuse in `base::search::merge_hits`; the f32 on-disk distance is widened to f64 to match `HnswHit`.
- `search_hits_filtered`: post-filtering a fixed top-k would under-return; recall under a very selective `keep` scales with `search_l` — widen it when the filter is rare. Mirrors `base::search::search_all_filtered`'s full-k contract.
