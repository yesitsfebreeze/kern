//! `kern reembed`: re-embed the entire graph with the configured embedding
//! model. The embedding dimension is locked into the graph on first ingest, so
//! switching models (e.g. nomic-embed-text 768-d -> qwen3-embedding) requires
//! re-embedding every stored vector. Run this AFTER changing `[embed] model`
//! in kern.toml and with the daemon stopped (it writes the graph directly).

use std::collections::HashMap;

use crate::base::math::average_vec;

use super::{Client, Endpoint, load_graph, save_graph};

const BATCH: usize = 64;

pub(super) async fn cmd_reembed(cfg: &crate::config::Config, embed_url: &str, embed_model: &str) {
	let mut g = load_graph(cfg);
	// Embed-only client (reason/answer unused here).
	let client = Client::new(
		Endpoint::default(),
		Endpoint::default(),
		Endpoint::new(embed_url, embed_model, &cfg.embed.key),
	);

	// Collect every entity's text, graph-wide, in a stable order.
	let mut ids: Vec<String> = Vec::new();
	let mut texts: Vec<String> = Vec::new();
	for kern in g.kerns.values() {
		for e in kern.entities.values() {
			ids.push(e.id.clone());
			texts.push(e.text());
		}
	}
	if ids.is_empty() {
		println!("reembed: graph is empty, nothing to do");
		return;
	}
	println!("reembed: {} entities -> model '{embed_model}'", ids.len());

	let new_vecs = match embed_all(&client, &ids, &texts).await {
		Ok(v) => v,
		Err(e) => {
			eprintln!("reembed: aborted, graph unchanged: {e}");
			return;
		}
	};

	// Assign new vectors. gnn_vector is re-seeded from the raw embed; the GNN
	// tick refines it later (a stale-dimension gnn_vector would break its index).
	for kern in g.kerns.values_mut() {
		for e in kern.entities.values_mut() {
			if let Some(v) = new_vecs.get(&e.id) {
				e.vector = v.clone();
				e.gnn_vector = v.clone();
			}
		}
	}
	// Reason edge vectors are the mean of their endpoints — recompute from the
	// new entity vectors so the reason index matches the new dimension.
	for kern in g.kerns.values_mut() {
		for r in kern.reasons.values_mut() {
			if let (Some(fv), Some(tv)) = (new_vecs.get(&r.from), new_vecs.get(&r.to)) {
				r.vector = average_vec(fv, tv);
			}
		}
	}

	g.rebuild_index();
	save_graph(&g);
	println!("reembed: hot graph done ({} entities)", new_vecs.len());

	match reembed_cold(g.store(), &client).await {
		Ok(0) => println!("reembed: complete — model is now '{embed_model}'"),
		Ok(n) => {
			println!("reembed: cold tier done ({n} entities)");
			println!("reembed: complete — model is now '{embed_model}'");
		}
		// Hot graph is already on the new model; cold failed and was left intact.
		// Report exactly what is stale so the operator can re-run, rather than
		// printing a misleading "complete".
		Err(e) => eprintln!(
			"reembed: {e}\nreembed: hot graph is on '{embed_model}' but the cold tier still \
			 uses the old model — re-run once the embed endpoint is healthy"
		),
	}
}

async fn embed_all(
	client: &crate::llm::Client,
	ids: &[String],
	texts: &[String],
) -> Result<HashMap<String, Vec<f64>>, String> {
	let mut out: HashMap<String, Vec<f64>> = HashMap::with_capacity(ids.len());
	let mut done = 0usize;
	for chunk in texts.chunks(BATCH) {
		let vs = client
			.embed_batch(chunk)
			.await
			.map_err(|e| e.to_string())?;
		if vs.len() != chunk.len() {
			return Err(format!(
				"embed returned {} vectors for {} inputs",
				vs.len(),
				chunk.len()
			));
		}
		for v in vs {
			out.insert(ids[done].clone(), v);
			done += 1;
		}
		println!("  {done}/{ids_len}", ids_len = ids.len());
	}
	Ok(out)
}

/// Re-embed spilled cold-tier entities too — otherwise their old-dimension
/// vectors mismatch the query and cold search silently drops them.
///
/// Atomic: vectors are reassigned in memory and only committed if EVERY batch
/// succeeds, so any failure leaves the cold tier fully unchanged (never a
/// partial-dimension mix that would corrupt cold search). On failure the error
/// names the offending batch and exactly how many cold entities were left
/// un-re-embedded, so the caller can report a precise partial-success state
/// instead of a generic abort. `Ok(n)` is the number of cold entities
/// re-embedded — `0` when there is no store or nothing is cold.
async fn reembed_cold(
	store: Option<std::sync::Arc<crate::base::store::Store>>,
	client: &crate::llm::Client,
) -> Result<usize, String> {
	let Some(store) = store else { return Ok(0) };
	let mut cold = store
		.cold_all()
		.map_err(|e| format!("cold load failed: {e}; cold tier left unchanged"))?;
	if cold.is_empty() {
		return Ok(0);
	}
	let total = cold.len();
	let n_batches = total.div_ceil(BATCH);
	println!("reembed: {total} cold entities");
	let texts: Vec<String> = cold.iter().map(|e| e.text()).collect();

	for (i, chunk) in texts.chunks(BATCH).enumerate() {
		let start = i * BATCH;
		// If we bail here, every entity from this batch onward keeps its old vector.
		let stale = total - start;
		let vs = client.embed_batch(chunk).await.map_err(|e| {
			format!(
				"cold batch {}/{n_batches} embed failed ({e}); {stale} of {total} cold \
				 entities NOT re-embedded; cold tier left unchanged",
				i + 1
			)
		})?;
		if vs.len() != chunk.len() {
			return Err(format!(
				"cold batch {}/{n_batches} returned {} vectors for {} inputs; {stale} of \
				 {total} cold entities NOT re-embedded; cold tier left unchanged",
				i + 1,
				vs.len(),
				chunk.len(),
			));
		}
		for (j, v) in vs.into_iter().enumerate() {
			cold[start + j].vector = v;
		}
	}

	// Write the re-embedded entities back in one transaction (latest-wins per id).
	// A crash mid-commit leaves the OLD rows intact — LMDB never exposes a partial
	// transaction.
	store
		.cold_put_all(&cold)
		.map_err(|e| format!("cold write-back failed: {e}; cold tier left unchanged"))?;
	Ok(total)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn embed_all_errs_when_server_returns_a_mismatched_vector_count() {
		// Stub /api/embed that always returns exactly ONE embedding regardless of
		// how many inputs are sent, so a 2-input batch trips embed_all's count guard
		// (vs.len() != chunk.len()).
		let app = axum::Router::new().route(
			"/api/embed",
			axum::routing::post(|| async {
				axum::Json(serde_json::json!({ "embeddings": [[0.1, 0.2, 0.3]] }))
			}),
		);
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap();
		let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

		let client = crate::llm::Client::new_embed_only(&format!("http://{addr}"), "test-model");
		let ids = vec!["a".to_string(), "b".to_string()];
		let texts = vec!["alpha".to_string(), "beta".to_string()];

		let err = embed_all(&client, &ids, &texts)
			.await
			.expect_err("a short vector count must abort the re-embed");
		assert!(
			err.contains("1 vectors for 2 inputs"),
			"the count mismatch is surfaced verbatim, got: {err}",
		);

		server.abort();
	}

	#[tokio::test]
	async fn reembed_cold_reports_stale_count_and_leaves_the_tier_unchanged_on_failure() {
		use crate::base::store::Store;
		use crate::base::types::Entity;

		// Stub returns ONE embedding regardless of input count, so the (2-entity)
		// cold batch trips the count guard and reembed_cold must bail.
		let app = axum::Router::new().route(
			"/api/embed",
			axum::routing::post(|| async {
				axum::Json(serde_json::json!({ "embeddings": [[0.5, 0.5]] }))
			}),
		);
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap();
		let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

		let dir = tempfile::tempdir().unwrap();
		let store = std::sync::Arc::new(Store::open(&dir.path().to_string_lossy()).unwrap());
		// Seed two cold entities carrying a recognizable OLD vector.
		let old = vec![9.0, 9.0];
		let seed = vec![
			Entity { id: "c1".into(), vector: old.clone(), ..Default::default() },
			Entity { id: "c2".into(), vector: old.clone(), ..Default::default() },
		];
		store.cold_put_all(&seed).unwrap();

		let client = crate::llm::Client::new_embed_only(&format!("http://{addr}"), "m");
		let err = reembed_cold(Some(store.clone()), &client)
			.await
			.expect_err("a mismatched cold batch must surface a partial-failure error");

		// Precise partial-success reporting: names how many were left stale + that
		// the tier is untouched.
		assert!(err.contains("2 of 2"), "names the stale entity count: {err}");
		assert!(err.contains("left unchanged"), "states the cold tier is untouched: {err}");

		// Atomicity: the cold tier still holds the ORIGINAL vectors, never the stub's.
		let after = store.cold_all().unwrap();
		assert_eq!(after.len(), 2);
		assert!(after.iter().all(|e| e.vector == old), "no partial write on failure");

		server.abort();
	}
}
