# src/base/search.rs — commentary

- `merge_hits`: regression history — the blend branch once used `*entry > 0.0` as a presence proxy; since scores are cosine similarities in [-1, 1], zero/negative content hits were mistaken for "absent" and silently overwritten with the raw GNN score. Fixed to key on map presence (`Entry::Occupied`); `merge_blends_a_nonpositive_content_hit_present_in_both` is the regression test.
- `GNN_BLEND` (0.6) > `CONTENT_BLEND` (0.4) because the learned GNN re-embedding is trusted more than the raw content index.
Second-pass migration (2026-07-17):
- `merge_hits` blend formula (was in the doc): both-index nodes score `CONTENT_BLEND*content + GNN_BLEND*gnn`; single-index nodes keep their raw score. Shared by `search_all_unlocked` and `search_all_filtered` so fusion + ranking lives in one place. Sort: score desc, id-asc tiebreak over the unstable HashMap iteration order — same convention as fuse::rrf.
- `search_all_filtered`: post-filtering an unfiltered top-k yields fewer than k when matches are sparse; this returns a full k matching hits. `keep` is built at the retrieval layer from a `QueryOptions` filter (`score::matches_filter`), keeping this base-layer function free of any retrieval dependency.
