# src/trnsprt/tests/search_rpc.rs — commentary

The roundtrip runs on JSON only because the typed-RPC stack hardcodes its frame type to `serde_json::Value`; bincode-serializability of the DTOs is exercised by unit tests inside `search::dto` instead.

- `search_with_unmatched_scheme_returns_no_hits_not_an_error`: the scheme axis is an exact-match filter — an unknown scheme yields an empty result set, but the RPC must still succeed (graceful, fresh), never error or panic.

Second-pass migration:

- Module doc 3 lines -> 2. The parenthetical it dropped: the cancellation race is that a newer search supersedes an older one, which surfaces as `fresh: false` on the older response. `cancellation_marks_older_keystroke_as_stale` asserts exactly that (tokens 2, then 1 -> stale, then 3 -> fresh), so the name plus the asserts carry it.
- Kept inline: `// Empty edge_kinds = all edges; depth gets clamped to 3 server-side.` in `neighbors_respects_edge_kind_filter` — the only explanation of the magic `depth: 99` and the empty `edge_kinds` vec.
Design notes (moved from source comments during comment sweep):
- Integration tests for SearchSvc: end-to-end client/server roundtrip on InprocAdapter + JsonEnvelopeCodec, plus the cancellation race. (empty edge_kinds = all edges; depth gets clamped to 3 server-side.)
