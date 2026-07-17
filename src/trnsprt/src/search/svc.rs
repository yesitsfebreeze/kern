//! `SearchSvc` service definition — the `service!` macro generates the trait,
//! `SearchSvcClient<C>`, and the `serve_search_svc(channel, handler)` loop.

use super::dto::{
	EntityKindLite, NeighborsReq, NeighborsRes, PreviewReq, PreviewRes, SearchReq, SearchRes,
};

crate::service! {
		/// Search palette RPC surface.
		pub trait SearchSvc {
				/// Incremental ranked search across the connected index.
				async fn search(req: SearchReq) -> SearchRes;
				/// Drill: typed neighbors of an entity (depth clamped server-side to 3).
				async fn neighbors(req: NeighborsReq) -> NeighborsRes;
				/// Right-pane preview payload for the selected entity.
				async fn preview(req: PreviewReq) -> PreviewRes;
				/// Canonical entity-kind enumeration — the facet parser validates
				/// `!fact`-style sigils against it.
				async fn kinds() -> Vec<EntityKindLite>;
		}
}
