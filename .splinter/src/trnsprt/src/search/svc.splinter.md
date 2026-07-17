# src/trnsprt/src/search/svc.rs — commentary

The repl palette holds a `SearchSvcClient`; kern (or a mock for tests) implements `SearchSvc`. See the relay search TUI design doc (`docs/relay-search-tui.md`) for how this surface maps onto the palette's search/drill/preview panes.

Second-pass migration: module doc compressed to the macro-output inventory; `kinds` doc trimmed — the facet parser validates `!fact`, `?question`, ... sigils against the fixed kern surface.
Design notes (moved from source comments during comment sweep):
- SearchSvc service definition — the service! macro generates the trait, SearchSvcClient<C>, and the serve_search_svc(channel, handler) loop.
- RPC contracts: search = incremental ranked search across the connected index. neighbors = drill: typed neighbors of an entity (depth clamped server-side to 3). preview = right-pane preview payload for the selected entity. kinds = canonical entity-kind enumeration; the facet parser validates !fact-style sigils against it.
