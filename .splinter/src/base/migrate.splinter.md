# src/base/migrate.rs — commentary

The absence of a dual-read fallback is deliberate — repo law: no compat. Legacy
`.kern` shards are read only through this one-shot path via the retained legacy
reader in `persist.rs`.

Second-pass migration (comment -> note):
- Post-migration state, moved out of the `//!` doc: the old `.kern` shard files are
  left in place for the user to delete — migration never destroys the source — and
  they go inert, because after migrating, `load_dir` reads only the store. This is
  the ONLY remaining reader of the legacy format (the no-dual-read law above).
- `migrate_dir` writes `data.mdb` / `lock.mdb` alongside the old `.kern` files in
  the same directory; the doc now keeps only the idempotency contract.
- Test narration removed: the setup lays down a legacy graph (root + one child with
  one vector-bearing entity), migrates, then asserts the store-backed `load_dir`
  sees the data with no legacy read, vectors intact within int8 tolerance.
