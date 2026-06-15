use std::sync::Arc;
use std::time::Instant;

use crate::base::search::search_all_unlocked;
use crate::profile::{render_timeline, Profile};
use crate::retrieval::seed::Mode;

use super::{load_graph, Client, Endpoint};

const TIMELINE_WIDTH: usize = 40;

/// Small fixed conversation so distill timing is comparable across runs.
const DISTILL_SAMPLE: &str = "User: The deploy failed because the config pointed at the staging \
	bucket. Assistant: Fixed — the bucket name is now anchored to the environment, so production \
	reads prod-artifacts and staging keeps its own.";

fn ms(t: Instant) -> f64 {
	t.elapsed().as_secs_f64() * 1000.0
}

fn flat(name: &str, total_ms: f64) -> Profile {
	Profile {
		name: name.to_string(),
		checkpoints: Vec::new(),
		total_ms,
	}
}

fn renamed(mut p: Profile, name: &str) -> Profile {
	p.name = name.to_string();
	p
}

/// Time every hot path against the live graph and print a scaled timeline.
/// Read-only: nothing is persisted, so it is safe to run next to a daemon.
pub(super) async fn cmd_profile(cfg: &crate::config::Config, text: &str, no_llm: bool) {
	let mut profiles: Vec<Profile> = Vec::new();

	let t = Instant::now();
	let g = load_graph(cfg);
	profiles.push(flat("load graph", ms(t)));
	let kerns = g.kerns.len();
	let mut entities = 0usize;
	for k in g.all() {
		entities += k.entities.len();
	}

	let reason_url = cfg.reason_url().to_string();
	let llm_client = Client::new(
		Endpoint::new(&reason_url, &cfg.reason.model, cfg.reason_key()),
		Endpoint::new(cfg.answer_url(), &cfg.answer.model, cfg.answer_key()),
		Endpoint::new(&cfg.embed.url, &cfg.embed.model, &cfg.embed.key),
	);

	// Cold vs warm split: the first embed may pay an Ollama model (re)load,
	// the second is the steady-state cost every later stage actually sees.
	let t = Instant::now();
	let qvec = match llm_client.embed(text).await {
		Ok(v) => v,
		Err(e) => {
			eprintln!("embed: {e} (embed endpoint up at {}?)", cfg.embed.url);
			return;
		}
	};
	profiles.push(flat("embed (cold)", ms(t)));

	let t = Instant::now();
	let _ = llm_client.embed(text).await;
	profiles.push(flat("embed (warm)", ms(t)));

	let t = Instant::now();
	let hits = search_all_unlocked(&g, &qvec, 10);
	profiles.push(flat(&format!("vector search ({} hits)", hits.len()), ms(t)));

	for (mode, label) in [
		(Mode::Content, "query content (no llm)"),
		(Mode::Reason, "query reason (no llm)"),
		(Mode::Hybrid, "query hybrid (no llm)"),
	] {
		let (_, p) = crate::retrieval::answer::query_profiled(
			&g,
			&cfg.retrieval,
			&qvec,
			text,
			mode,
			None,
			None,
			None,
		);
		profiles.push(renamed(p, label));
	}

	if no_llm || reason_url.is_empty() {
		if !no_llm {
			eprintln!("no reason endpoint configured; skipping llm stages");
		}
	} else {
		// One complete-closure shared by the profiled query and distill (was two
		// complete_func() calls). The embed closure comes from the shared factory.
		let llm_fn: crate::retrieval::LlmFunc = Arc::new(llm_client.complete_func());
		let embed_fn: crate::retrieval::EmbedFunc = super::embed_fn(&llm_client);

		let (_, p) = crate::retrieval::answer::query_profiled(
			&g,
			&cfg.retrieval,
			&qvec,
			text,
			Mode::Hybrid,
			Some(&llm_fn),
			Some(&embed_fn),
			None,
		);
		profiles.push(renamed(p, "query hybrid (llm)"));

		let t = Instant::now();
		let claims = crate::ingest::distill::distill(DISTILL_SAMPLE, &*llm_fn);
		let n = claims.map(|c| c.len()).unwrap_or(0);
		profiles.push(flat(&format!("distill ({n} claims)"), ms(t)));
	}

	let t = Instant::now();
	let digest = crate::retrieval::digest::build_digest(
		&g,
		cfg.capture.digest_k,
		cfg.capture.digest_min_trust,
		cfg.capture.digest_token_budget,
	);
	profiles.push(flat(
		&format!("digest build ({} bytes)", digest.len()),
		ms(t),
	));

	println!("kern profile — {kerns} kerns, {entities} entities, query: {text:?}");
	println!();
	print!("{}", render_timeline(&profiles, TIMELINE_WIDTH));
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::{json, Value};

	/// Spawn a throwaway HTTP server on an ephemeral port; returns its base URL.
	async fn serve(app: axum::Router) -> String {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap();
		tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
		format!("http://{addr}")
	}

	/// The no-LLM path must run end-to-end without panicking on an empty graph:
	/// load → cold/warm embed → vector search → the three no-LLM query modes →
	/// digest build. The reason/answer endpoints are never touched (no_llm=true),
	/// so only `/api/embed` is stubbed; everything downstream runs on a fresh,
	/// empty graph backed by a temp data dir.
	#[tokio::test]
	async fn cmd_profile_no_llm_path_does_not_panic() {
		// Stub Ollama-native /api/embed: any input -> a fixed 3-dim embedding.
		let app = axum::Router::new().route(
			"/api/embed",
			axum::routing::post(|_body: axum::Json<Value>| async move {
				axum::Json(json!({ "embeddings": [[0.1, 0.2, 0.3]] }))
			}),
		);
		let embed_url = serve(app).await;

		// Isolated empty data dir so load_graph yields a fresh graph (and Store::open
		// has a real directory to bind).
		let dir = std::env::temp_dir().join(format!("kern_profile_smoke_{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();

		let mut cfg = crate::config::Config {
			data_dir: dir.to_string_lossy().into_owned(),
			..Default::default()
		};
		cfg.embed.url = embed_url;

		// no_llm=true → the reason/answer stages are skipped entirely.
		cmd_profile(&cfg, "smoke test query", true).await;

		let _ = std::fs::remove_dir_all(&dir);
	}
}
