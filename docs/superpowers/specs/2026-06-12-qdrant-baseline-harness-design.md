# Qdrant head-to-head baseline harness (design)

**Status:** design / approved-for-planning.

**One line:** Run the **same corpus and queries** through kern *and* a real
Qdrant, computing **identical** recall@k / NDCG@k / latency / RPS / RAM, and emit
a side-by-side table — so "supersede Qdrant in every regard" stops being a claim
and becomes a measured number.

---

## Problem & evidence

`aspiration.md`'s **Tier-0** is "measure first (unblocks everything)", and its
single most-blocking ❌ is the **head-to-head benchmark harness vs Qdrant**. The
doc is explicit and repeated: *"Measure, don't assume … never claim 'parity'
without the number."*

The **kern side is now done** (built across this session's measurement arc):

| Capability | Where |
|---|---|
| recall@k | `bench_support::ndcg::recall_at_k` (`6e5c094`) |
| NDCG@k | `bench_support::ndcg::ndcg_at_k` |
| latency p50/p95/p99 (LLM-free graph path) | `bench_support::latency::measure_latency` (`fb8e478`) |
| throughput / RPS (concurrent readers) | `bench_support::latency::measure_throughput` (`650827a`) |
| vector memory (f64 vs int8) | `bench_support::memory::estimate_memory` (`18ddfdb`) |
| trace corpus + filtered queries | `bench_support::trace` / `build` (`6779332`, `9386de0`) |
| faithful bench (proven: recall@10=NDCG@10=1.0) | `ac86bb8`, `44fd545`, `fcf83e1`, `e463c24` |
| one-shot snapshot | `retrieval_bench --all` (`2a30323`) |

What is **missing** is the *other half*: the **identical** pipeline run against
Qdrant on the **same** vectors, so the two columns are comparable. Today there is
no Qdrant in the loop at all.

## Goals

1. A harness that, given a corpus + query set + a Qdrant endpoint, produces a
   **side-by-side** report: kern vs Qdrant on recall@10, NDCG@10, p50/p95/p99
   latency, RPS, and RAM.
2. **Apples-to-apples**: both systems index the **same embedding vectors**
   (produced once by kern's embedder) and the recall/NDCG are computed by the
   **same** `ndcg` functions on both rankings — no metric or embedder drift.
3. Graceful **kern-only** mode when no Qdrant endpoint is configured (so the
   harness is useful before Qdrant is wired).
4. Honest accounting of the structural differences (in-process vs network hop;
   during-traversal filtering vs payload filtering) **in the report**, not hidden.

## Non-goals

- Beating Qdrant in this change. This is the *ruler*, not the climb.
- A distributed/multi-node Qdrant. Single-node baseline first.
- Replacing the kern-internal `retrieval_bench` (it stays; this composes it).

## Design

### 1. The `VectorBackend` abstraction

```rust
// bench_support/backend.rs
pub struct Doc { pub id: String, pub vector: Vec<f32>, pub kind: Option<String> }
pub struct QueryHit { pub id: String, pub score: f32 }

pub trait VectorBackend {
    fn name(&self) -> &str;                 // "kern" | "qdrant"
    fn index(&mut self, docs: &[Doc]) -> anyhow::Result<()>;
    fn query(&self, vec: &[f32], k: usize, kind_filter: Option<&str>) -> anyhow::Result<Vec<QueryHit>>;
    fn vector_bytes(&self) -> usize;        // for the memory column (payload only)
}
```

Both backends receive the **same** `Doc`s. The embedding is computed **once** by
the caller (kern's embedder, or a real model in a future revision) so neither
backend re-embeds — that is the apples-to-apples invariant.

### 2. Backends

- **`KernBackend`** — wraps a `GraphGnn` built via the existing `bench_support::build`
  path (which now also builds the lexical index + realistic similarity edges).
  `query` calls `retrieval::answer::query` with the kind filter; `vector_bytes`
  delegates to `memory::estimate_memory`.
- **`QdrantBackend`** — feature-gated (`--features qdrant`, optional
  `qdrant-client` dep). Creates a collection (cosine distance, same dim), upserts
  the `Doc` vectors with `kind` as a payload field, and `query` runs a
  `search_points` with a payload filter equal to `kind_filter`. `vector_bytes` =
  `n * dim * 4` (Qdrant stores f32) or the scalar-quant size when quant is on.

### 3. The comparison harness

```rust
// bench_support/compare.rs
pub struct BackendReport { pub name, recall10, ndcg10, lat: LatencyReport, qps, vector_bytes }
pub fn compare(backends: &mut [Box<dyn VectorBackend>], corpus: &Corpus) -> Vec<BackendReport>
```

For each backend: `index(corpus.docs)`, then for each query run `query`, collect
ranked ids, and feed them through the **same** `ndcg::recall_at_k` /
`ndcg::ndcg_at_k`. Latency/RPS via the same warmup+iters loop as
`measure_latency`/`measure_throughput`, but over the trait (so it times Qdrant's
network round-trip honestly). Emit the rows; the CLI prints a table and the
in-process vs network-hop delta is labelled.

### 4. Corpus

The current `synthetic.json` (25 docs, stub embedder) is too small to discriminate
ANN recall (a fixed ef already explores everything — see `e463c24`). The baseline
needs **scale + a real embedder**:

- Phase A: a **larger synthetic** corpus (1k–10k docs) generated with kern's
  embedder, with planted near-duplicates so recall@k is discriminating.
- Phase B (preferred): a slice of a **standard IR set** (e.g. BEIR/`scifact`,
  or an MS-MARCO subset) embedded once with kern's embedder, with the dataset's
  graded qrels as ground truth.

`build_graph`'s O(n²) edge seeding (`build.rs`) must move to a top-k ANN-based
edge build before Phase A (documented TODO in `seed_similarity_edges`), or the
1k–10k build dominates wall-clock.

### 5. CLI

`retrieval_bench --compare --qdrant http://localhost:6333 --trace <corpus>` →
prints:

```
metric          kern        qdrant
recall@10       0.94        0.93
ndcg@10         0.71        0.70
latency p50 ms  0.9         3.2     (kern in-process; qdrant + network hop)
rps             6500        2100
vector MiB      12.5(int8)  100.0(f32)
```

## Acceptance criteria (EARS)

- **WHEN** run with a Qdrant endpoint and a corpus, the harness **SHALL** emit one
  row per backend with recall@10, NDCG@10, p50/p95/p99 latency, RPS, and vector
  bytes, computed by the **same** `ndcg`/`latency` code for both.
- **WHEN** no `--qdrant` endpoint is given, the harness **SHALL** run kern-only and
  succeed (no Qdrant dependency at runtime).
- **WHERE** a kind filter is present, both backends **SHALL** apply it (kern during
  traversal, Qdrant as a payload filter) and recall **SHALL** be measured over the
  filtered result.
- The embedding vectors fed to both backends **SHALL** be byte-identical (embedded
  once), so any recall gap is the index/fusion, not the embedder.
- The report **SHALL** label the in-process vs network-hop latency difference
  rather than presenting kern's latency as directly comparable.

## Risks & notes

- **Embedder parity is the #1 confound** (this session proved it: the stub
  embedder's collisions faked a retrieval bug, `44fd545`/`fcf83e1`). Embed once,
  feed both. Use a *real* model for Phase B.
- **Filter semantics**: kern filters during traversal (avoids the post-filter
  fewer-than-k loss, `9a57096`); Qdrant uses payload filters. Both are correct;
  the report compares *outcomes*, not mechanisms.
- **kern's structural advantage is the network hop** — call it out in the latency
  row; it is the whole "in-process, no network hop" thesis.
- **Qdrant quant parity**: enable Qdrant's scalar/int8 quant to match kern's int8
  for a fair memory column.

## Leverages (already built this session)

The kern column is essentially free: `recall_at_k`/`ndcg_at_k`, `measure_latency`/
`measure_throughput`, `estimate_memory`, the faithful `build`/`embed`/`trace`
harness, and the `--all` snapshot. This SPEC adds the `VectorBackend` seam, the
`QdrantBackend` adapter, a representative corpus, and the comparison/printing.

## Phasing

1. `VectorBackend` trait + `KernBackend` + `compare` harness + a kern-vs-kern test
   (proves the framework without Qdrant).
2. Larger corpus generator (and the `build.rs` O(n²)→ANN edge fix it needs).
3. `QdrantBackend` behind `--features qdrant` + the CLI `--compare`.
4. Phase-B real IR dataset + graded qrels.
