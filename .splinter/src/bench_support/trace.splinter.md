# splinter: src/bench_support/trace.rs


Second-pass migration: the `Trace` doc's JSON example was moved out of the source (the serde structs plus the retained "all fields required except `mode`" line carry the contract). The example shape, for reference:

```json
{
  "name": "my-trace",
  "docs": [
    { "id": "d1", "text": "the borrow checker rejects aliased mutable refs" }
  ],
  "queries": [
    { "id": "q1", "query": "borrow checker", "expected_ids": ["d1"], "mode": "hybrid" }
  ]
}
```

- `docs` seed the graph; each `query` is scored against its `expected_ids` using its declared `mode` (`"content"` | `"reason"` | `"hybrid"`, defaulting to `"hybrid"`).
- The identical example previously appeared a second time in `src/bin/retrieval_bench.rs`'s `//!` doc; both copies are gone, and this note is the single record.
# src/bench_support/trace.rs — commentary (migrated from source doc comments)

- `Trace` = a retrieval benchmark trace: named corpus (`docs`) + `queries`, deserialized by `load`. All JSON fields are required except each query's `mode`, which defaults to `"hybrid"` via `default_mode`.
- `TraceDoc.kind`: optional entity kind (via `EntityKind::parse`, defaults to `Claim`) so a single trace can mix kinds for `filter_kind` queries.
- `TraceQuery.mode`: retrieval mode, one of `"content" | "reason" | "hybrid"`; optional in JSON, defaults to `"hybrid"`.
- `TraceQuery.filter_kind`: optional entity-kind filter; when set, the query runs the filtered retrieval path end-to-end. `expected_ids` are the docs that count as relevant.
