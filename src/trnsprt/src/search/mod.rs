//! `SearchSvc` — typed-RPC surface for search. Client + serve fn are macro-
//! expanded in place: edit the trait in `svc.rs`, there is no generated file.

pub mod dto;
pub mod mock;
pub mod svc;

pub use dto::{
	EdgeKind, EdgeRef, EntityKindLite, EntityRef, EntityStatusLite, Facet, NeighborsReq,
	NeighborsRes, PreviewReq, PreviewRes, SearchReq, SearchRes,
};
pub use mock::MockSearchServer;
pub use svc::{serve_search_svc, SearchSvc, SearchSvcClient};
