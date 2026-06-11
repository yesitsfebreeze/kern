//! `SearchSvc` — typed-RPC surface for search.
//!
//! Layout:
//! - [`dto`] — wire types ([`SearchReq`], [`PreviewRes`], ...).
//! - [`svc`] — `service!` invocation that emits [`SearchSvc`],
//!   [`SearchSvcClient`], and [`serve_search_svc`].
//! - [`mock`] — in-memory [`MockSearchServer`] for tests and
//!   downstream slices (palette UI, preview pane).
//!
//! Crate-internal `service!` invocation is in [`svc`]; consumers
//! re-export the generated trait/client/serve fn from this module.
//!
//! NB: `svc.rs` is **macro-generated surface** — `SearchSvcClient` and
//! `serve_search_svc` are expanded in place by the `service!` macro from the
//! trait. To change the RPC, edit the trait in `svc.rs`; never hand-edit the
//! generated client/server shapes (they have no separate file).

pub mod dto;
pub mod mock;
pub mod svc;

pub use dto::{
    EdgeKind, EdgeRef, EntityKindLite, EntityRef, EntityStatusLite, Facet, NeighborsReq,
    NeighborsRes, PreviewReq, PreviewRes, SearchReq, SearchRes,
};
pub use mock::MockSearchServer;
pub use svc::{serve_search_svc, SearchSvc, SearchSvcClient};
