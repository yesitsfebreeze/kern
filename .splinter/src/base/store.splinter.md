# src/base/store.rs — commentary

SHARED-CANDIDATE: a content-addressed embedded store + zstd codec is exactly the
durable primitive relay/agnt would also want. Kept kern-local by explicit decision
(docs/superpowers/plans/2026-06-10-redb-store.md); lift into `shared/` if a second
daemon needs it rather than reinventing it.

The store replaced the legacy file-per-shard bincode tier (`persist.rs`) and the
JSONL cold tier (`cold.rs`) — container swap only, the codec stayed bincode-in-zstd.
LMDB (via `heed`) was chosen over single-process stores (redb/sled) because the
per-cwd daemon, the CLI, and the recall hook (`kern search`) all touch the same
data dir concurrently. LMDB is multi-process by design: many readers + one writer,
MVCC, mmap — readers never block the tick writer, a second writer waits rather than
failing. It is an in-process library (no network hop, no fallback backend), so it
is a storage primitive like bincode, not a "pluggable backend" — complies with the
no-pluggable-backend repo law.

- `MAP_SIZE`: incident behind the 16 GiB cap — under the old 4 GiB cap, an env
  bloated to the limit by the unnamed-child spawn runaway could not commit its own
  cleanup (observed: 178k empty kerns reaped in memory by `gc_empty_kerns` but the
  save failed `MDB_MAP_FULL`, so the bloat never cleared and every restart
  re-failed). On 64-bit the reservation is virtual address space (near-free); kept
  moderate rather than enormous only for the Windows non-sparse case. Bump further
  if a real graph + cold tier approaches it.
- `ZSTD_LEVEL`: 3 is the zstd default — good ratio/speed balance for the small,
  repetitive bincode blobs stored here.
- `StoredKern`: storing the heavy, high-entropy float vectors as int8 (1 byte/dim
  vs 8) is the size win zstd alone can't deliver on embeddings; `gnn_vector` is
  derived, so persisting it is pure waste at rest.
- `cold_spill`: latest-wins put means the cold tier never accumulates duplicate
  rows the way the old JSONL append log did.
- `flush_guarded`: the safety net behind the "never run two writers" hazard — a
  daemon that loaded an empty store while the CLI grew it on disk can no longer
  flush its stale snapshot over the larger graph.

Second-pass migration (comment -> note):
- `MAP_SIZE` mechanics: LMDB mmaps the range up front; most filesystems keep the
  backing file sparse so real disk tracks data, but on Windows not every
  filesystem does, so the cap also bounds worst-case file size. The
  even-deletes-fail behavior exists because a copy-on-write B-tree needs free
  pages to commit a deletion — the cap must stay well above the largest transient
  bloat or the graph cannot self-heal.
- FORMAT_V1/V2 contract details: V2 appends `StoredKern::temporal` after
  `reason_vecs`. The `Entity` temporal fields are `#[serde(skip)]`, so every other
  stored value (cold `Entity`, meta rows, `u64`) is byte-identical across V1/V2
  and decodes under either tag; only `StoredKern` needs the version-aware decode.
  Decoding a V1 blob yields entities with all temporal fields `None`.
- `bincode_cfg` limit: without it a corrupt/fuzzed length prefix tricked the
  decoder into a 5 EiB allocation on random bytes (tests/persist_fuzz.rs). Real
  snapshots are far smaller, so the 1 GiB cap only rejects pathological inputs.
- `StoredVec` trap in full: bincode is positional/non-self-describing, so a field
  omitted on write (`skip_serializing_if`) desyncs the decoder — it still reads
  the field — and corrupts everything after it. Encoding reuses the tested int8
  quantizer in `quant`.
- `Store` concurrency: LMDB's many-reader/single-writer MVCC means concurrent
  recalls (CLI, hook, daemon) read a consistent snapshot without blocking the
  tick writer.
- `swap_compacted` workaround detail: heed/LMDB unmaps the file ASYNCHRONOUSLY on
  Windows, so a rename issued immediately after closing can hit "Access is
  denied" while the unmap drains; the loop retries with backoff up to ~2.5s. On
  final failure the tmp copy is removed so a retry isn't confused by a stale one.
- `compact_dir` mechanics: LMDB only grows `data.mdb` to a high-water mark;
  deleted pages are reused but the file stays at peak size (on Windows NTFS it is
  not even sparse, so the peak is real disk). The compactor opens its own env,
  `copy_to_file(CompactionOption::Enabled)`, closes deterministically via
  `prepare_for_closing().wait()` (mmap released), then swaps.
- Test contracts moved out of comments: layout-invariant values tagged V1 must
  still `decode` (cold/meta rows are byte-identical across versions); a V1 kern
  row written straight into LMDB must load end-to-end through `load_all_kerns`
  (the version-aware scan migrates rather than skipping the old layout as
  corrupt); `flush_guarded` refusal proves the guard blocks only the stale case —
  after reconciling to the current epoch the same snapshot flushes and prunes.
