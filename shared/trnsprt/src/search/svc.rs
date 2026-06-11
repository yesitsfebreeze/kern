//! `SearchSvc` typed-RPC service definition.
//!
//! Driven by the `service!` proc macro from `trnsprt-macros`. Generates
//! - the `SearchSvc` trait (one method per RPC),
//! - `SearchSvcClient<C>` that owns a [`Channel`](crate::typed::Channel),
//! - `serve_search_svc(channel, handler)` server loop.
//!
//! The repl palette holds a `SearchSvcClient`; kern (or a mock for
//! tests) implements `SearchSvc`.
//!
//! See also: the [relay search TUI design doc](../docs/relay-search-tui.md) for
//! how this surface maps onto the palette's search/drill/preview panes.

use super::dto::{
    EntityKindLite, NeighborsReq, NeighborsRes, PreviewReq, PreviewRes, SearchReq, SearchRes,
};

crate::service! {
    /// Search palette RPC surface
    /// (Transport section).
    pub trait SearchSvc {
        /// Incremental ranked search across the connected index.
        async fn search(req: SearchReq) -> SearchRes;
        /// Drill: typed neighbors of an entity (depth clamped server-side to 3).
        async fn neighbors(req: NeighborsReq) -> NeighborsRes;
        /// Right-pane preview payload for the selected entity.
        async fn preview(req: PreviewReq) -> PreviewRes;
        /// Canonical entity-kind enumeration. Used by the facet parser
        /// to validate `!fact`, `?question`, ... sigils against the
        /// fixed kern surface.
        async fn kinds() -> Vec<EntityKindLite>;
    }
}
