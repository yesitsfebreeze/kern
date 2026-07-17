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
