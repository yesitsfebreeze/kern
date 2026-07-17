# src/gossip/seen.rs — commentary

- `SeenSet`: the HashMap + insertion-order VecDeque design replaced a fixed ring buffer that overwrote by slot position regardless of expiry, so a flood could evict still-live ids under normal traffic. The current structure only evicts live ids under a genuine flood of >`GOSSIP_SEEN_SET_CAP` distinct ids within one TTL window.


Second-pass migration:
- `SeenSet` doc compressed. Complexity argument moved here: membership is O(1) via `HashMap<id, expiry>`; the insertion-order VecDeque gives O(1)-amortised reclamation because the constant TTL makes expiry monotonic in insertion order (expired entries sit at the front). Live ids are only evicted under a genuine flood of >`GOSSIP_SEEN_SET_CAP` distinct ids within one TTL window, oldest-first; normal traffic never hits the cap.
- `add_and_check` doc: "O(1) amortised" moved here.
- `len_order` doc compressed; settled-state meaning: every live id has exactly one order entry and expired originals are reclaimed rather than left as stale duplicates.
- Test narration deleted in `count_is_bounded_under_flood_recent_id_survives` (flood >CAP ids at one instant, all live) and `reinsert_after_expiry_*` (expire, re-record past original TTL).
