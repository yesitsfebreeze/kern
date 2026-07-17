# src/trnsprt/src/search/mod.rs — commentary

Layout: `dto` wire types; `svc` the `service!` invocation emitting `SearchSvc`/`SearchSvcClient`/`serve_search_svc`; `mock` in-memory `MockSearchServer` for tests and downstream slices (palette UI, preview pane). Consumers re-export the generated trait/client/serve fn from this module.

Second-pass migration: module doc compressed to the "macro-expanded in place, no generated file to hand-edit" trap.
Design notes (moved from source comments during comment sweep):
- SearchSvc is the typed-RPC surface for search. Client + serve fn are macro-expanded in place: edit the trait in svc.rs, there is no generated file.
