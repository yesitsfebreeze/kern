# src/gossip/seen.rs — commentary

- `SeenSet`: the HashMap + insertion-order VecDeque design replaced a fixed ring buffer that overwrote by slot position regardless of expiry, so a flood could evict still-live ids under normal traffic. The current structure only evicts live ids under a genuine flood of >`GOSSIP_SEEN_SET_CAP` distinct ids within one TTL window.


Second-pass migration:
- `SeenSet` doc compressed. Complexity argument moved here: membership is O(1) via `HashMap<id, expiry>`; the insertion-order VecDeque gives O(1)-amortised reclamation because the constant TTL makes expiry monotonic in insertion order (expired entries sit at the front). Live ids are only evicted under a genuine flood of >`GOSSIP_SEEN_SET_CAP` distinct ids within one TTL window, oldest-first; normal traffic never hits the cap.
- `add_and_check` doc: "O(1) amortised" moved here.
- `len_order` doc compressed; settled-state meaning: every live id has exactly one order entry and expired originals are reclaimed rather than left as stale duplicates.
- Test narration deleted in `count_is_bounded_under_flood_recent_id_survives` (flood >CAP ids at one instant, all live) and `reinsert_after_expiry_*` (expire, re-record past original TTL).

# Ratings — scope: src/gossip/seen.rs

Scope rating: 8/10 — clean O(1) design (HashMap+VecDeque), monotonic-expiry reclaim, flood cap, tests cover expiry/flood/reinsert. Only nit: bare unwrap style inconsistency (fixed).

## Function ratings

- `SeenSet::new` — 9/10: minimal, correct cap+ttl init.
- `SeenSet::add_and_check` — 9/10: thin delegate to `_at`, Instant::now at boundary. Good.
- `SeenSet::add_and_check_at` — 8/10: core logic correct; two while loops reclaim expired + flood-cap. First loop used bare unwrap (now expect), second uses let-else — minor style drift, fixed. Live-id fast-path early return is correct.
- `SeenSet::len` / `len_order` — 9/10: test-only accessors, trivial.
- `SeenSet::default` — 9/10: delegates to new.
- `first_sight_is_new_repeat_is_seen` — 9/10: covers core contract.
- `distinct_ids_are_each_new` — 9/10: covers isolation.
- `entry_expires_after_ttl` — 9/10: covers TTL boundary.
- `expired_entries_are_reclaimed_not_accumulated` — 9/10: covers the reclaim invariant.
- `count_is_bounded_under_flood_recent_id_survives` — 9/10: covers the cap + oldest-eviction.
- `reinsert_after_expiry_gets_a_fresh_ttl_and_leaves_no_stale_dupe` — 9/10: covers the stale-dupe skip.
