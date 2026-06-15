//! Shared test-only helpers used by unit tests across the crate. Compiled only
//! under `#[cfg(test)]` (see the gated `mod test_support` in `lib.rs`).

use tokio::task::JoinHandle;

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
