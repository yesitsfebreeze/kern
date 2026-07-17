# src/base.rs — commentary

Nothing migrated: the file carries only a short `//!` purpose doc (foundational
layer — graph, store/cold tier, vector + lexical indices, CRDT merge, heat decay,
shared types/constants/math), which stays inline as the module contract.

Second-pass migration (comment -> note):
- The `//!` doc was cut from four lines to two. The full subsystem roster it used
  to spell out — in-memory knowledge graph, LMDB store + cold tier, HNSW/DiskANN
  vector indices, BM25 lexical index, CRDT merge, heat decay, shared
  types/constants/math primitives — is just the `pub mod` list below it, so the
  doc now states the layer's purpose and lets the module list enumerate itself.
