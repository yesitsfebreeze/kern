# src/commands/graph_ops.rs — commentary

Convention: `forget_entity`, `link_vector`, and `degrade_entity_reasons` are deliberately pure graph mutations (no IO) so their policies (fact-guard, edge-vector fallback, decay/removal schedule) are unit-testable apart from the `cmd_*` load/save wrappers. `find_entity_by_prefix` lives here rather than the dispatch/server module because it is purely a graph read concern.

- `cmd_link` missing-kern guard: fails loudly because the previous silent path saved an unchanged graph yet still printed "linked", reporting a success that never happened (`from_kern_id` comes from `find_entity`, so a vanished kern is a bug).

Second-pass migration (test narration moved out of the source):

- `graph_with` fixture: `g.register(k)` is what populates `entity_kern`, so `find_entity` takes its fast path in these tests.
- `find_entity_by_prefix_resolves_a_unique_prefix` covers three cases in one test: a unique prefix resolves to the entity + its kern id; an exact id still resolves (the fast path taken before the prefix scan); a prefix matching nothing yields `None` rather than panicking.
find_entity_by_prefix: exact-id lookup, else the first id-prefix match across every kern. The `get` path falls back to the cold tier because stigmergy GC spills evicted thoughts to the store before dropping them from the hot graph. degrade_entity_reasons cuts each incident edge's score by a geometric schedule (BASE * POW^i); edges falling below DEGRADE_MIN_THRESHOLD are removed. forget_entity refuses facts (immutable).
