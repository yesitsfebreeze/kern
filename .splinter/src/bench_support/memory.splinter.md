# splinter: src/bench_support/memory.rs


Second-pass migration (from the `//!` doc and item docs):
- Why this module exists: vector payload is the dominant memory cost of a vector DB and the headline capacity number ("N vectors of D dims cost X"). Reporting f32 alongside the int8-quantized equivalent makes the scalar-quantization saving (a kern moat) a concrete ratio rather than a claim.
- Why an estimate and not RSS: the estimate is portable and deterministic; RSS is neither. Scope is the vector PAYLOAD only — excludes HNSW graph structure, entity text/metadata, and allocator overhead — so it is a lower bound on process RSS, never a measurement of it. That scope caveat stays inline.
- `quant_ratio`: the ratio is exactly `size_of::<f32>()` = 4 when every entity shares one dim; it returns 0 (not NaN) when there are no vectors, guarded by `empty_graph_reports_zero_and_no_divide_by_zero`.
- `estimate_memory`: computes `vectors * dim * 4` (f32) and `vectors * dim * 1` (int8); `dim` is the widest embedding seen.
# src/bench_support/memory.rs — commentary (migrated from source doc comments)

- `estimate_memory` reports the vector-storage footprint of a built graph: f32 vs int8. It counts PAYLOAD ONLY — it excludes HNSW structure, text/metadata, and allocator overhead — so every byte figure is a LOWER BOUND on RSS.
- `MemoryReport.vectors`: entities carrying a non-empty embedding (may be fewer than `entities`).
- `MemoryReport.dim`: embedding dimension = the widest seen; 0 when there are no vectors.
- `quant_ratio` = float_vector_bytes / int8_vector_bytes; 0 when there are no vectors (avoids divide-by-zero / NaN). int8 is 4x smaller than f32.
