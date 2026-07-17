# src/retrieval/expand.rs — commentary

Beam-walk performance design: the hot loop never touches `String` keys — `Interner` maps ids to dense `u32`s so `visited`/`results` use integer keys (cheap hash, no per-touch clone); ids are borrowed back out via `Interner::name` only to resolve surviving results and materialise chains.

- `ChainNode`: replaces the old design where each frontier item carried a cloned `Vec<String>` of its whole walk — nodes now store a parent index and chains are materialised to owned strings only for popped nodes that get recorded. `materialize_chain` reproduces the exact order the old per-item vec accumulated.
- `Beam` is hand-rolled instead of `BinaryHeap` so the payload (interned u32 / arena index) stays out of the ordering; only `score` sifts.
