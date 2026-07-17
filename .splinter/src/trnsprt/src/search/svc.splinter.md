# src/trnsprt/src/search/svc.rs — commentary

The repl palette holds a `SearchSvcClient`; kern (or a mock for tests) implements `SearchSvc`. See the relay search TUI design doc (`docs/relay-search-tui.md`) for how this surface maps onto the palette's search/drill/preview panes.

Second-pass migration: module doc compressed to the macro-output inventory; `kinds` doc trimmed — the facet parser validates `!fact`, `?question`, ... sigils against the fixed kern surface.
