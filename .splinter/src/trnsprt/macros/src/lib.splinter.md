# src/trnsprt/macros/src/lib.rs — commentary

- `expand`: trait methods are re-emitted with a `Send` bound on the returned future (not verbatim).
- `codec_bound`: the Codec trait bound is identical on the client struct, its impl block, and the serve fn — built once and interpolated so the five-trait clause isn't copy-pasted three times and stays in sync.

Second-pass migration:

- Module doc 24 lines -> 2. Moved here — the full input/output spec of `service!`:
  - Input shape: `trnsprt::service! { pub trait MemoryRpc { async fn truncate_after(ts_ms: u64) -> Result<(), RpcError>; } }`.
  - Output 1 — the trait, with a `Send` bound added to each async method's returned future (see the existing `expand` note above: not verbatim).
  - Output 2 — `<Name>Client<C: Codec>`, owning a `Channel<C>` plus an atomic id counter. One async method per trait method, each serialising its named arguments via `serde_json::to_value`, sending a request envelope, and awaiting the reply with the matching id (out-of-order replies park in `pending`).
  - Output 3 — `serve_<snake>(channel, handler)`, an async loop reading requests, dispatching to the handler, and sending replies.
  - Consumers depend on `trnsprt`, never on this macro crate directly: generated code references `::trnsprt::*` paths only, which `trnsprt`'s `extern crate self as trnsprt` self-alias makes resolve even for in-crate invocations. That constraint stayed inline (1 line).
- `to_snake` test essay 8 lines -> 2. Full reasoning: `to_snake` derives the `serve_<snake>` fn name from the trait ident, so its output is part of the macro's PUBLIC surface — a regression there renames the generated server fn and breaks every call site. `expand()` itself cannot be unit-tested in this crate because it returns a `proc_macro::TokenStream`, a type that only exists inside the compiler; the generated client/server is instead proven to compile and round-trip by the consumer-crate integration tests (`tests/search_rpc.rs` and `tests/kern_rpc.rs` drive `SearchSvcClient`/`serve_search_svc` and `KernRpcClient`/`serve_kern_rpc` over real `Channel` pairs).
- Deleted two asserts' inline labels: the `to_snake("Memory")`/`("X")` cases cover the `i != 0` guard (a leading capital never yields a leading underscore) and `to_snake("ABC") == "a_b_c"` shows each interior capital starting a new word, consecutive caps included. The assertions state this themselves.
Design notes (moved from source comments during comment sweep):
- service! expands a trait decl into three items: the trait, <Name>Client<C>, and serve_<snake>(channel, handler). It emits ::trnsprt::* paths only (relies on `extern crate self as trnsprt` in lib.rs so the paths resolve inside this crate too).
- to_snake output names the generated serve_<snake> fn, so its output is public surface. expand() itself is exercised only by the consumer-crate integration tests, not here.
