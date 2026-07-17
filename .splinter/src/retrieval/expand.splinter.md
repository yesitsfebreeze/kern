# src/retrieval/expand.rs — commentary

Beam-walk performance design: the hot loop never touches `String` keys — `Interner` maps ids to dense `u32`s so `visited`/`results` use integer keys (cheap hash, no per-touch clone); ids are borrowed back out via `Interner::name` only to resolve surviving results and materialise chains.

- `ChainNode`: replaces the old design where each frontier item carried a cloned `Vec<String>` of its whole walk — nodes now store a parent index and chains are materialised to owned strings only for popped nodes that get recorded. `materialize_chain` reproduces the exact order the old per-item vec accumulated.
- `Beam` is hand-rolled instead of `BinaryHeap` so the payload (interned u32 / arena index) stays out of the ordering; only `score` sifts.
Beam-expansion internals:
- ScoredRef<'a>: a scored entity borrowed from the graph. The pipeline works on these, cloning to owned ScoredEntity only for delivery survivors.
- Scored trait: uniform view over owned ScoredEntity and borrowed ScoredRef, so the scoring/diversify stages run on either without cloning.
- Interner: assigns a dense u32 to each distinct entity id in one expand() run. The id clones into an Rc<str> once on intern; every later touch is a u32 lookup. name_rc returns an owned Rc<str> handle (a refcount bump) — held as Rc<str> not &str so the loop can keep mutating the interner (interning neighbours) meanwhile.
- ChainNode: one node of the beam's path forest. A seed root has no edge (rid == "") and no parent (NO_PARENT = u32::MAX).
- BeamNode: one frontier entry. Its payload (interned id, arena index) never participates in ordering; only score does.
- Beam: hand-rolled binary max-heap over BeamNodes keyed on score (assumed finite), hand-rolled so the u32/arena payload stays out of the ordering.
- materialize_chain walks a node's parent chain into the [seed, rid, ent, rid, ent, ...] id list that PathChain carries.
- find_entity_and_kern: two-pass — O(1) via the kern_of_entity index, then a full scan fallback for stale/missing index entries.
