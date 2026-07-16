//! Shared test-only helpers used by unit tests across the crate. Compiled only
//! under `#[cfg(test)]` (see the gated `mod test_support` in `lib.rs`).

use crate::base::types::{Entity, Reason};
use tokio::task::JoinHandle;

/// A default [`Entity`] with the given id; all other fields `Default`.
pub(crate) fn entity(id: &str) -> Entity {
	Entity {
		id: id.into(),
		..Default::default()
	}
}

/// A default [`Entity`] with the given id and vector; all other fields
/// `Default`. The fixture several `base`/`retrieval`/`tick` test modules each
/// open-coded as a local `fn ent(id, vector)`.
pub(crate) fn entity_vec(id: &str, vector: Vec<f32>) -> Entity {
	Entity {
		id: id.into(),
		vector,
		..Default::default()
	}
}

/// A default [`Reason`] edge `from -> to`, id `"{from}->{to}"`; all other fields
/// `Default`. The fixture `reason`/`pagerank` test modules each open-coded.
pub(crate) fn edge(from: &str, to: &str) -> Reason {
	Reason {
		id: format!("{from}->{to}"),
		from: from.into(),
		to: to.into(),
		..Default::default()
	}
}

/// Bind an axum app to an ephemeral localhost port, spawn it, and return its
/// base URL plus the server task handle. Replaces the per-module
/// `serve(app)` boilerplate that each hand-rolled the same
/// `bind 127.0.0.1:0 → local_addr → spawn → format!("http://{addr}")` dance.
///
/// Callers that only need the URL can drop the handle (`let (url, _server) =`);
/// dropping a `JoinHandle` detaches — it does not abort — so the stub server
/// keeps serving for the rest of the test, exactly as the old detached spawn did.
pub(crate) async fn spawn_http(app: axum::Router) -> (String, JoinHandle<()>) {
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let addr = listener.local_addr().unwrap();
	let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
	(format!("http://{addr}"), handle)
}
