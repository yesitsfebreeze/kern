# src/base/accept.rs — commentary

- `accept`: the hoisted dedup check previously re-ran on every routing-loop iteration and again in `commit_entity` — up to 65 identical HNSW searches per accept.
- `supersede` (old-entity lookup): resolves the owning kern via the entity index (O(1)); replaced an O(kerns × entities) scan over `g.all()`.
- `supersede` (ANN eviction rationale): a superseded entity left in the ANN indices is still returned by candidate generation, occupies top-k slots, then gets filtered downstream by `score::matches_filter` / the `status != Superseded` retain in `retrieval::score` — fewer-than-k recall loss (same class as the kind-filter fix) plus index memory on data that can never surface.
- `add_anchor`: the one-anchor-per-name update-in-place guard exists because the live root once carried the same anchor string twice.
- `equivalent_anchor_exists` / `promote_to_root_if_generic`: duplicate-anchor guard for promotion — the naming LLM rephrases one concept many ways; without the guard every rephrasing minted a fresh root anchor (observed live: 9+ for a single concept, including an exact-string duplicate). Skipping promotion is loss-free: nothing is moved or merged; the kern stays named under generic and the existing anchor keeps catching matching memories.
- `route_to_child_id`: returns None when the best acceptance probability is below ACCEPT_FLOOR; `route_entity` then falls through to the generic catch-all.

Second-pass migration:
- `route_entity` (dup early-return): the deleted line said a duplicate is "committed in the starting kern" — in fact `commit_entity` short-circuits with `deduped: true` and stores nothing; the early return only skips descent.
- `get_or_spawn_unnamed_child` / `get_or_spawn_generic_child`: full get-vs-loaded story — `loaded` sees only in-memory kerns, so under a kern-load cap an evicted (or spilled, for the named/immortal generic) child looked absent and a fresh one was spawned per call, a runaway that filled the graph to `max_kerns` unnamed kerns / duplicate `generic` children. `get` auto-loads from disk and reuses the existing child.
- `supersede_by_contradiction`: the incoming revision was WITHHELD at dedup (only a `Rephrase` edge recorded it); this path materializes it as Active, stamps the old entity bi-temporally (invalidated_at = now, valid_to = successor's valid_from), and evicts old from `entity_idx`/`gnn_entity_idx` — the exact mirror of same-source `supersede`.
- `parse_contradiction`: full contract — UPDATE/CONTRADICTION → Supersede; RELATED, empty, or garbage → Related, so an ambiguous or failed LLM classification fails OPEN to the rephrase behavior; any RELATED mention wins even alongside "update".
- `add_anchor`: "normalized" = trimmed + lowercased (see `find_anchor_by_name`).
- test `unnamed_child_not_duplicated_when_non_root_parent_evicts`: the empty-kern bloat grew a real daemon to 178k kerns; it came from deep trees where the non-root parent spilled AFTER its unnamed child was linked — `unload` persists the parent's `children` before dropping it, which is what the test locks.
- test `accept_never_leaves_empty_unnamed_kern`: the orphan-shard leak grew the data dir to 347k files; invariant: a duplicate short-circuits before any spawn, and a spawned unnamed child always receives the committed entity.
- test `supersede_stamps_both_temporal_clocks`: contract — invalidated_at records transaction time; valid_to closes at the successor's valid_from.
- test `promote_skips_when_root_anchor_vec_is_near_duplicate`: near-dup direction in the fixture is cosine ~0.995 to the existing anchor.
- Deleted narration/labels duplicating test names or assert messages in: unnamed/generic reuse tests, supersede_drops_the_old_entity_from_the_search_index, contradiction tests, parse_contradiction, add_anchor/promote tests.
