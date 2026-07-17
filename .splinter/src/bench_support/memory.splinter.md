# splinter: src/bench_support/memory.rs


Second-pass migration (from the `//!` doc and item docs):
- Why this module exists: vector payload is the dominant memory cost of a vector DB and the headline capacity number ("N vectors of D dims cost X"). Reporting f32 alongside the int8-quantized equivalent makes the scalar-quantization saving (a kern moat) a concrete ratio rather than a claim.
- Why an estimate and not RSS: the estimate is portable and deterministic; RSS is neither. Scope is the vector PAYLOAD only — excludes HNSW graph structure, entity text/metadata, and allocator overhead — so it is a lower bound on process RSS, never a measurement of it. That scope caveat stays inline.
- `quant_ratio`: the ratio is exactly `size_of::<f32>()` = 4 when every entity shares one dim; it returns 0 (not NaN) when there are no vectors, guarded by `empty_graph_reports_zero_and_no_divide_by_zero`.
- `estimate_memory`: computes `vectors * dim * 4` (f32) and `vectors * dim * 1` (int8); `dim` is the widest embedding seen.
