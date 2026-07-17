# src/base/persist.rs — commentary

Entity rename (slice A) was a clean break: the on-disk bincode layout changed
(`Thought` -> `Entity`, `ThoughtKind` -> `EntityKind` + `EntityStatus`,
`SourceRef` -> typed `Source` enum, `Kern.thoughts` -> `Kern.entities`,
`created_at` moved off `Source` onto `Entity`). Old saved DBs are NOT migrated —
they must be regenerated. CLAUDE.md mandates "no compat".

Encryption-at-rest posture: intentionally none at this layer. `atomic_write`'s
tmp-write + fsync + atomic rename is durability, not confidentiality; the
intended protections are deployment-layer (full-disk / volume encryption,
filesystem ACLs).

- `backfill_created_at`: deprecation horizon — it silently mutates loaded data on
  every restart, acceptable ONLY because it belongs to the file-shard *reader*
  the store migration is retiring. Per
  docs/superpowers/plans/2026-06-10-redb-store.md (Step 1), `load_kern` /
  `load_legacy_dir` / `backfill_created_at` move into
  `migrate.rs::read_legacy_dir` and the whole path is deleted once stores are the
  only on-disk format.
- `load_legacy_dir`: why the decode is parallel — a real graph can hold hundreds
  of thousands of `.kern` files; sequential read+bincode-decode made loading an
  O(shards) multi-minute hang that blocked every CLI command and the daemon's
  startup reap. Decode is pure CPU+IO per file with no cross-shard state, so it
  fans out cleanly across rayon's pool; wall time drops by ~core count.

Second-pass migration (comment -> note):
- `reload_from_disk` rationale in full: this is how a live graph reloads after
  another writer advanced the store. Opening a second `Store` on the same dir
  would be a same-process double-open of the LMDB env — forbidden, it corrupts the
  lock and can SIGSEGV — so reload paths must thread the existing handle through
  here rather than calling `load_dir` again. Returns `None` for an in-memory graph.
- `load_dir` / `graph_from_store`: an empty or root-less store still yields a fresh
  graph bound to the now-open store, so the very first run on a new project
  persists correctly. The loaded epoch is stamped so the stale-flush guard knows
  which generation the graph is consistent with.
- `merged_root` scope: the authoritative root-only fields overlaid from `g.root`
  are id, root_id, anchor text/vec, inner/outer radii, and descriptors. Both
  `save_all` and the tick worker's per-kern persist write the root through this, so
  a `Persist` task targeting the root can never clobber those with a stale map
  entry. The descriptor REPLACE-don't-union rule stays inline — it is the
  regression oracle (the old `insert` loop re-added descriptors from the stale
  base, so `descriptor rm` never persisted).
- `save_graph_into` cost note: it clones the kern map once to apply the
  `merged_root` overlay. That is fine because this is the full-persist path
  (shutdown / explicit save / copy), not the hot per-kern `do_persist`.
- `FlushSnapshot` / `snapshot_for_flush` / `flush_snapshot` — the point of the
  split: cloning the map while the read lock is held only as long as the clone
  takes lets the caller DROP the graph lock before the LMDB transaction runs, so a
  multi-second flush no longer pins the read lock against writers. The disk-epoch
  comparison stays INSIDE the store's write txn, so `expected` (the graph's
  `flushed_epoch` at capture time) still detects another writer — the multi-writer
  safety net is unchanged, only the graph lock is released earlier.
- `save_all`: the store's `save_all_kerns` prunes any kern row not in the live set,
  which is what replaced the old on-disk orphan-file reconcile.
- `compress_dir`: on-disk vectors are always int8 now (the store's size win), so
  `target_mode` controls only the HNSW index mode the next load rebuilds with, not
  the durable vector form.
- `flush_guarded` framing: `expected` is the epoch the caller last observed; the
  refusal exists so a stale daemon can never overwrite a graph the CLI grew
  underneath it.
- Test contracts moved out of comments: `atomic_write` failure is forced by making
  the destination an existing DIRECTORY (renaming a file onto a dir errors on every
  platform) — the tmp must be cleaned and the original rename error surfaced.
  `named_kern_with_anchor_vec_round_trips` guards the bincode-positional assumption
  behind the purpose->anchor rename: if a future edit reorders `Kern`'s fields the
  decoded values shift, catching live-graph corruption before it ships.
  `load_dir_loads_every_sibling` is the parity guard for the rayon decode — a large
  sibling set must come back complete and order-independent, with a corrupt sibling
  mixed in so the skip path is exercised concurrently with the happy path.

Third-pass migration (2026-07-17, comment -> note):
- `load_legacy_dir` corrupt-sibling policy: a corrupt/unreadable sibling `.kern`
  is warned-and-skipped, never allowed to vanish silently or truncate the load;
  the root is loaded up front and still hard-errors, since a graph without its
  root is unusable.
