use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use watcher::{FileWatcher, IgnoreRules, IngestPipeline, IngestRecord, IngestSink, WatcherError};

use crate::base::types::{EntityKind, Source};
use crate::ingest::{Config as IngestRunConfig, Worker};

fn strip_file_uri(uri: &str) -> String {
	if let Some(rest) = uri.strip_prefix("file:///") {
		// Windows `file:///C:/foo` → `C:/foo`; POSIX `file:///abs` → `abs`
		// (drops the empty authority's leading slash).
		return rest.to_string();
	}
	if let Some(rest) = uri.strip_prefix("file://") {
		// Non-empty authority (`file://host/path`): per RFC 8089 drop the host,
		// the local path is everything from the first '/'.
		return match rest.find('/') {
			Some(i) => rest[i..].to_string(),
			None => String::new(),
		};
	}
	uri.to_string()
}

#[derive(Clone)]
pub struct KernFileWatcherSink {
	worker: Arc<Worker>,
	retention_secs: u64,
	review_policy: crate::ingest::ReviewPolicy,
	// `<intake>/direct`, or `None` when nothing drains it. An undrained intake is
	// worse than the RAM queue — the same gate `tool_ingest` applies.
	direct_dir: Option<PathBuf>,
}

impl KernFileWatcherSink {
	pub fn new(
		worker: Arc<Worker>,
		retention_secs: u64,
		review_policy: crate::ingest::ReviewPolicy,
		direct_dir: Option<PathBuf>,
	) -> Self {
		Self {
			worker,
			retention_secs,
			review_policy,
			direct_dir,
		}
	}

	// Per record, never once at construction: this sink lives as long as the
	// daemon, and a deadline resolved at startup would give a file edited a
	// month later a TTL measured from boot.
	fn ingest_config(&self) -> IngestRunConfig {
		IngestRunConfig {
			review_policy: self.review_policy.clone(),
			..Default::default()
		}
		.with_retention(self.retention_secs)
	}
}

#[async_trait]
impl IngestSink for KernFileWatcherSink {
	async fn ingest(&self, record: IngestRecord) {
		let IngestRecord {
			source_uri,
			content,
			language_hint,
		} = record;

		let path = strip_file_uri(&source_uri);
		let title = std::path::Path::new(&path)
			.file_name()
			.and_then(|s| s.to_str())
			.unwrap_or("")
			.to_string();

		let source = Source::File {
			path,
			section: String::new(),
			title,
			author: String::new(),
			url: source_uri,
		};

		let hint = language_hint.unwrap_or_default();

		// The channel, not a principal: nobody asserted this, a file changed on
		// disk. `scheme()` is also what `RetrievalConfig::source_trust` weights on,
		// so `source_trust = { file = ... }` is the lever that separates the
		// watcher from an agent — a `"watcher"` constant would only relabel the
		// same 0.95 ceiling.
		let tag = source.scheme();

		// Durable first, RAM second — `tool_ingest`'s shape. `notify` installs
		// watches and replays nothing, and there is no startup scan, so a record
		// still in the channel when the daemon dies is gone and nothing re-offers
		// it. The raw 1.0 travels rather than a pre-clamped value: `job()` is the
		// one clamp gate, and `source_tag` is what makes it clamp the same on both
		// legs. Fail open — a failed durable write falls through to the queue,
		// because a watcher that silently stops ingesting is the worse outcome.
		if let Some(dir) = &self.direct_dir {
			let cfg = self.ingest_config();
			let job = crate::ingest::direct::DirectJob {
				text: content.clone(),
				source: source.clone(),
				kind: EntityKind::Document,
				hint: hint.clone(),
				confidence: 1.0,
				valid_until: cfg.valid_until,
				acl: crate::base::types::Acl::default(),
				source_tag: tag.to_string(),
			};
			match crate::ingest::direct::intake_direct(dir, &job) {
				Ok(_) => return,
				Err(e) => tracing::warn!(
					target: "kern.ingest.direct",
					dir = %dir.display(),
					error = %e,
					"direct intake write failed; falling back to the in-RAM queue"
				),
			}
		}

		self
			.worker
			.submit(
				content,
				source,
				EntityKind::Document,
				hint,
				1.0,
				tag,
				self.ingest_config(),
			)
			.await;
	}
}

pub async fn run(
	roots: Vec<PathBuf>,
	ignore: IgnoreRules,
	sink: Arc<KernFileWatcherSink>,
) -> Result<(), WatcherError> {
	let mut watcher = FileWatcher::new(roots, ignore)?;
	let pipeline = IngestPipeline::new((*sink).clone());
	while let Some(ev) = watcher.next_event().await {
		pipeline.handle(ev).await;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use parking_lot::RwLock;
	use std::time::{Duration, SystemTime};

	use tempfile::tempdir;
	use tokio::time::{sleep, timeout};

	use crate::base::accept;
	use crate::base::graph::GraphGnn;
	use crate::base::types::{Acl, ChunkPart, ChunkPartKind, Embedding, Entity, EntityStatus};
	use crate::base::util;
	use crate::crdt::GCounter;

	#[derive(Clone)]
	struct DirectFileSink {
		graph: Arc<RwLock<GraphGnn>>,
	}

	impl DirectFileSink {
		fn new(graph: Arc<RwLock<GraphGnn>>) -> Self {
			Self { graph }
		}

		fn build_entity(&self, source: Source, text: String) -> Entity {
			let vec = crate::ingest::stub_one_hot(&text);
			let id = util::content_hash(&text);
			let mut t = Entity {
				id,
				root_id: String::new(),
				external_id: source.object_id().to_string(),
				superseded_by: String::new(),
				kind: EntityKind::Document,
				status: EntityStatus::Active,
				review: Default::default(),
				statements: vec![text],
				chunks: vec![ChunkPart {
					kind: ChunkPartKind::StatementRef,
					text: String::new(),
					index: 0,
				}],
				vector: vec.into(),
				gnn_vector: Embedding::default(),
				score: 0.0,
				conf_alpha: 2.0,
				conf_beta: 1.0,
				source,
				created_at: Some(SystemTime::now()),
				acl: Acl::default(),
				access_count: GCounter::new(),
				accessed_at: None,
				heat: 0.0,
				heat_updated_at: None,
				updated_at: None,
				valid_until: None,
				valid_until_lamport: 0,
				valid_until_producer: String::new(),
				producer_id: String::new(),
				unlinked_count: 0,
				dirty: false,
				valid_from: None,
				valid_to: None,
				invalidated_at: None,
			};
			t.refresh_score();
			t
		}
	}

	#[async_trait]
	impl IngestSink for DirectFileSink {
		async fn ingest(&self, record: IngestRecord) {
			let path = strip_file_uri(&record.source_uri);
			let title = std::path::Path::new(&path)
				.file_name()
				.and_then(|s| s.to_str())
				.unwrap_or("")
				.to_string();
			let source = Source::File {
				path,
				section: String::new(),
				title,
				author: String::new(),
				url: record.source_uri,
			};
			let entity = self.build_entity(source, record.content);
			let root_id = self.graph.read().root.id.clone();
			accept::accept(&mut self.graph.write(), &root_id, entity, "");
		}
	}

	fn count_file_documents(g: &GraphGnn) -> usize {
		g.kerns
			.values()
			.flat_map(|k| k.entities.values())
			.filter(|t| matches!(t.kind, EntityKind::Document) && t.source.scheme() == "file")
			.count()
	}

	fn collect_file_paths(g: &GraphGnn) -> Vec<String> {
		g.kerns
			.values()
			.flat_map(|k| k.entities.values())
			.filter(|t| matches!(t.kind, EntityKind::Document) && t.source.scheme() == "file")
			.map(|t| t.source.object_id().to_string())
			.collect()
	}

	#[test]
	fn strip_file_uri_handles_windows_and_posix() {
		assert_eq!(strip_file_uri("file:///C:/foo/bar.rs"), "C:/foo/bar.rs");
		assert_eq!(strip_file_uri("file:///abs/posix.rs"), "abs/posix.rs");
		assert_eq!(strip_file_uri("file://host/p.rs"), "/p.rs");
		assert_eq!(
			strip_file_uri("file://host"),
			"",
			"bare authority with no path"
		);
		assert_eq!(strip_file_uri("plain/path.rs"), "plain/path.rs");
	}

	#[tokio::test]
	async fn sink_ingest_produces_file_document() {
		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let sink = DirectFileSink::new(g.clone());
		let rec = IngestRecord {
			source_uri: "file:///tmp/hello.rs".to_string(),
			content: "fn hello() {}".to_string(),
			language_hint: Some("rust".to_string()),
		};
		sink.ingest(rec).await;

		let g = g.read();
		let paths = collect_file_paths(&g);
		assert_eq!(paths.len(), 1);
		assert_eq!(paths[0], "tmp/hello.rs");
	}

	#[tokio::test]
	async fn watcher_pipeline_creates_document_for_new_file() {
		let dir = tempdir().expect("tempdir");
		let root = dir.path().to_path_buf();

		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let sink = DirectFileSink::new(g.clone());

		let mut fw = FileWatcher::new(vec![root.clone()], IgnoreRules::empty()).expect("watcher new");
		let pipeline = IngestPipeline::new(sink);

		sleep(Duration::from_millis(100)).await;

		let target = root.join("note.md");
		std::fs::write(&target, "hello watcher").expect("write file");

		let deadline = std::time::Instant::now() + Duration::from_secs(2);
		while std::time::Instant::now() < deadline {
			match timeout(Duration::from_millis(200), fw.next_event()).await {
				Ok(Some(ev)) => pipeline.handle(ev).await,
				Ok(None) => break,
				Err(_) => {}
			}
			let g_read = g.read();
			if count_file_documents(&g_read) >= 1 {
				break;
			}
		}

		let g_read = g.read();
		let paths = collect_file_paths(&g_read);
		assert!(
			!paths.is_empty(),
			"expected at least one file Document, got {paths:?}"
		);
		let target_str = target.to_string_lossy().replace('\\', "/");
		assert!(
			paths
				.iter()
				.any(|p| target_str.ends_with(p) || p.ends_with("note.md")),
			"expected stored path to reference note.md; got {paths:?}"
		);
	}

	// The sink used to hand the worker `IngestRunConfig::default()` outright, so
	// a `[watcher] retention_secs` had nowhere to land and every watched file
	// became a document that never expires.
	#[tokio::test]
	async fn the_sink_stamps_the_configured_retention_on_what_it_ingests() {
		let app = axum::Router::new().route(
			"/api/embed",
			axum::routing::post(|_b: axum::Json<serde_json::Value>| async move {
				axum::Json(serde_json::json!({ "embeddings": [[0.1, 0.2, 0.3]] }))
			}),
		);
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap();
		let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let embedder = crate::llm::Client::new_embed_only(&format!("http://{addr}"), "m", "");
		let worker = Arc::new(crate::ingest::Worker::new(
			g.clone(),
			embedder,
			None,
			None,
			None,
		));
		let sink = KernFileWatcherSink::new(worker.clone(), 3600, Default::default(), None);

		let before = SystemTime::now();
		sink
			.ingest(IngestRecord {
				source_uri: "file:///tmp/ttl.rs".to_string(),
				content: "fn expires() {}".to_string(),
				language_hint: Some("rust".to_string()),
			})
			.await;

		// `enqueue` is fire-and-forget by design, so the commit is observed, not awaited.
		let mut deadlines = Vec::new();
		let cap = std::time::Instant::now() + Duration::from_secs(5);
		while std::time::Instant::now() < cap {
			deadlines = g
				.read()
				.kerns
				.values()
				.flat_map(|k| k.entities.values().map(|e| e.valid_until))
				.collect();
			if !deadlines.is_empty() {
				break;
			}
			sleep(Duration::from_millis(25)).await;
		}

		assert!(!deadlines.is_empty(), "the watched file reached the graph");
		for got in &deadlines {
			let got = got.expect("a configured retention is a deadline, not None");
			assert!(
				got >= before + Duration::from_secs(3600),
				"the deadline is resolved per record, not hardcoded to None"
			);
		}

		assert_eq!(
			KernFileWatcherSink::new(worker, 0, Default::default(), None)
				.ingest_config()
				.valid_until,
			None,
			"…while an unconfigured watcher still ingests with no TTL",
		);

		server.abort();
	}

	// The watcher is the fast producer the bound exists for, and its record has no
	// durable backstop — nothing re-offers a file whose event has been consumed.
	// So this leg must wait for capacity, never be handed a refusal.
	#[tokio::test]
	async fn the_sink_waits_for_queue_capacity_rather_than_losing_the_file() {
		let (url, _server) =
			crate::test_support::spawn_http(crate::test_support::hanging_embed_app()).await;
		let embedder = crate::llm::Client::new_embed_only(&url, "m", "");
		let worker = Arc::new(crate::ingest::Worker::new(
			Arc::new(RwLock::new(GraphGnn::new())),
			embedder,
			None,
			None,
			None,
		));

		let mut offered = 0;
		while worker
			.enqueue(
				format!("filler {offered}"),
				Source::Inline {
					hash: String::new(),
					section: String::new(),
				},
				EntityKind::Document,
				String::new(),
				1.0,
				"inline",
				IngestRunConfig::default(),
			)
			.is_some()
		{
			offered += 1;
			tokio::task::yield_now().await;
			assert!(offered < 10_000, "the queue never filled");
		}

		let refused_before = crate::ingest::worker::ingest_queue_refused();
		let sink = KernFileWatcherSink::new(worker, 0, Default::default(), None);
		let blocked = timeout(
			Duration::from_millis(150),
			sink.ingest(IngestRecord {
				source_uri: "file:///tmp/backpressure.rs".to_string(),
				content: "fn waits() {}".to_string(),
				language_hint: Some("rust".to_string()),
			}),
		)
		.await;

		assert!(
			blocked.is_err(),
			"the sink returned while the queue was full — the file was refused, not queued"
		);
		assert_eq!(
			crate::ingest::worker::ingest_queue_refused(),
			refused_before,
			"waiting for capacity is not a refusal, and must not be counted as one"
		);
	}

	// ROADMAP item 95. The sink submitted a raw 1.0, which is Beta(2,1) = 0.6667 —
	// a human CLI claim's posterior, and above the 0.6500 a deliberate agent
	// assertion gets. A file appearing on disk is not an assertion at all.
	#[tokio::test]
	async fn a_watched_file_is_capped_below_a_deliberate_agent_assertion() {
		let (url, _server) =
			crate::test_support::spawn_http(crate::test_support::fixed_vec_embed_app()).await;
		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let embedder = crate::llm::Client::new_embed_only(&url, "m", "");
		let worker = Arc::new(crate::ingest::Worker::new(
			g.clone(),
			embedder,
			None,
			None,
			None,
		));

		KernFileWatcherSink::new(worker, 0, Default::default(), None)
			.ingest(IngestRecord {
				source_uri: "file:///tmp/trusted.rs".to_string(),
				content: "fn appears_on_disk() {}".to_string(),
				language_hint: Some("rust".to_string()),
			})
			.await;

		// `conf_beta`, not `conf_mean`: this document's single chunk re-derives the
		// same stub vector, so the second dedup gate calls `observe_support` and
		// moves `conf_alpha` after the mint. Only alpha accrues evidence, so beta
		// is the one field that still reports what was MINTED —
		// beta_params_from_confidence(c) gives beta = 2 - c, so 1.05 <=> 0.95 and
		// 1.00 <=> the raw 1.0 this path used to submit.
		let mut betas: Vec<f32> = Vec::new();
		let cap = std::time::Instant::now() + Duration::from_secs(5);
		while std::time::Instant::now() < cap {
			betas = g
				.read()
				.kerns
				.values()
				.flat_map(|k| k.entities.values().map(|e| e.conf_beta))
				.collect();
			if !betas.is_empty() {
				break;
			}
			sleep(Duration::from_millis(25)).await;
		}
		assert!(!betas.is_empty(), "the watched file reached the graph");

		let agent = 2.0 - crate::base::constants::MAX_AI_CONFIDENCE as f32;
		for got in &betas {
			assert!(
				(got - agent).abs() < 1e-6,
				"a watched file lands on the non-user ceiling: conf_beta want {agent:.4}, got {got:.4}"
			);
			assert!(
				*got > 1.0,
				"a file on disk must not mint a human's 1.0 (conf_beta 1.0); got {got:.4}"
			);
		}
	}

	// ROADMAP item 30. `notify` replays nothing and there is no startup scan, so
	// a record the sink handed to the RAM queue was gone for good if the daemon
	// died before the distill leg committed it. The backstop is the durable
	// intake `tool_ingest` already writes — and the tag has to survive that hop:
	// the drain used to name AGENT_SOURCE for everything it read.
	#[tokio::test]
	async fn a_watched_file_is_parked_on_disk_and_drains_back_as_a_file_not_an_agent() {
		let (url, _server) =
			crate::test_support::spawn_http(crate::test_support::fixed_vec_embed_app()).await;
		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let embedder = crate::llm::Client::new_embed_only(&url, "m", "");
		let worker = Arc::new(crate::ingest::Worker::new(
			g.clone(),
			embedder,
			None,
			None,
			None,
		));

		let dir = tempdir().expect("tempdir");
		let direct = dir.path().join("direct");
		let content = "fn survives_a_kill() {}";

		KernFileWatcherSink::new(worker.clone(), 0, Default::default(), Some(direct.clone()))
			.ingest(IngestRecord {
				source_uri: "file:///tmp/durable.rs".to_string(),
				content: content.to_string(),
				language_hint: Some("rust".to_string()),
			})
			.await;

		let parked = direct.join(format!("{}.json", util::content_hash(content)));
		assert!(
			parked.is_file(),
			"the record is on disk before the worker ever sees it: {}",
			parked.display()
		);
		let job: crate::ingest::direct::DirectJob =
			serde_json::from_str(&std::fs::read_to_string(&parked).unwrap()).expect("valid payload");
		assert_eq!(
			job.source_tag, "file",
			"the channel is written down, not re-derived by the drain"
		);

		let drained = crate::ingest::direct::drain_direct_once(
			&direct,
			&worker,
			&IngestRunConfig {
				dedup_threshold: 0.95,
				..Default::default()
			},
		)
		.await;
		assert_eq!(drained, 1, "the parked record committed on the way back");

		// `conf_beta` for the reason the test above gives: beta = 2 - confidence is
		// what was MINTED. 1.05 is the 0.95 ceiling; 1.00 would be the raw 1.0 the
		// payload carries, i.e. a hop that never clamped at all.
		let g = g.read();
		let docs: Vec<&Entity> = g
			.kerns
			.values()
			.flat_map(|k| k.entities.values())
			.filter(|e| matches!(e.kind, EntityKind::Document))
			.collect();
		assert!(!docs.is_empty(), "the drain rebuilt the document");
		let want = 2.0 - crate::base::constants::MAX_AI_CONFIDENCE as f32;
		for d in &docs {
			assert_eq!(d.source.scheme(), "file", "it came back as a file");
			assert!(
				(d.conf_beta - want).abs() < 1e-6,
				"the durable leg mints the same file-clamped 0.95 the RAM leg does: \
				 conf_beta want {want:.4}, got {:.4}",
				d.conf_beta
			);
		}
	}

	// ROADMAP item 18's last owed decision, pinned: a watched file is **public**.
	// Nobody names a principal on this path, so the `Document` carries the empty
	// `Acl` and every caller reads it — one naming an unrelated principal
	// (`matches_filter`) and the surface that can name none at all
	// (`Acl::is_public`, `src/mcp/resources.rs`). Both legs are asserted because
	// the ACL is written twice: at the sink for the durable one, inside
	// `Worker::submit` for the RAM one. Two literals, one decision, and this is
	// what stops them drifting apart.
	// Pins the DURABLE leg only. The RAM leg reaches `Worker::submit`, whose job
	// sits in the channel until a worker loop drains it, and this test spawns no
	// loop — so a wrong ACL written in `submit` is invisible here. Both call sites
	// pass `Acl::default()`, so they cannot drift without an edit to one of them,
	// and that edit is exactly what this test would miss. Recorded in item 18.
	#[tokio::test]
	async fn a_watched_file_is_public_once_the_durable_leg_commits() {
		async fn watched_docs(direct: Option<PathBuf>, content: &str) -> Vec<Entity> {
			let (url, _server) =
				crate::test_support::spawn_http(crate::test_support::fixed_vec_embed_app()).await;
			let g = Arc::new(RwLock::new(GraphGnn::new()));
			let embedder = crate::llm::Client::new_embed_only(&url, "m", "");
			let worker = Arc::new(crate::ingest::Worker::new(
				g.clone(),
				embedder,
				None,
				None,
				None,
			));

			KernFileWatcherSink::new(worker.clone(), 0, Default::default(), direct.clone())
				.ingest(IngestRecord {
					source_uri: "file:///tmp/public.rs".to_string(),
					content: content.to_string(),
					language_hint: Some("rust".to_string()),
				})
				.await;

			if let Some(dir) = &direct {
				let drained = crate::ingest::direct::drain_direct_once(
					dir,
					&worker,
					&IngestRunConfig {
						dedup_threshold: 0.95,
						..Default::default()
					},
				)
				.await;
				assert_eq!(drained, 1, "the parked record committed");
			}

			let cap = std::time::Instant::now() + Duration::from_secs(5);
			loop {
				let docs: Vec<Entity> = g
					.read()
					.kerns
					.values()
					.flat_map(|k| k.entities.values())
					.filter(|e| matches!(e.kind, EntityKind::Document))
					.cloned()
					.collect();
				if !docs.is_empty() || std::time::Instant::now() > cap {
					return docs;
				}
				sleep(Duration::from_millis(25)).await;
			}
		}

		let dir = tempdir().expect("tempdir");
		let legs = [
			(
				"durable",
				watched_docs(Some(dir.path().join("direct")), "fn parked_public() {}").await,
			),
			("ram", watched_docs(None, "fn queued_public() {}").await),
		];

		// A principal the file does not name. If the default ever stamps a scope, a
		// user or a group, this caller stops matching and the assertion below fires.
		let stranger = crate::retrieval::score::QueryOptions {
			principals: vec!["bob".into()],
			..Default::default()
		};
		for (leg, docs) in &legs {
			assert!(
				!docs.is_empty(),
				"{leg}: the watched file reached the graph"
			);
			for d in docs {
				assert!(
					crate::retrieval::score::matches_filter(d, &stranger),
					"{leg}: bob names a principal this file does not and reads it anyway — \
					 a watched file is public, not tenant-scoped; got {:?}",
					d.acl
				);
				assert!(
					d.acl.is_public(),
					"{leg}: a watched file names nothing, so the unauthenticated resource \
					 surface serves it too; got {:?}",
					d.acl
				);
			}
		}
	}

	// The other half of item 30's backstop, and the half with no coverage until
	// now: the durable write is best-effort. A watcher that silently stops
	// ingesting because a disk went read-only is worse than one with no backstop
	// at all, so `intake_direct` failing must fall through to the RAM queue —
	// once. `intake_direct` is tmp-then-rename, and the drain only reads `*.json`,
	// so there is no half-success that both parks a payload and re-submits it.
	#[tokio::test]
	async fn an_unwritable_intake_falls_through_to_the_queue_instead_of_dropping_the_file() {
		let (url, _server) =
			crate::test_support::spawn_http(crate::test_support::fixed_vec_embed_app()).await;
		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let embedder = crate::llm::Client::new_embed_only(&url, "m", "");
		let worker = Arc::new(crate::ingest::Worker::new(
			g.clone(),
			embedder,
			None,
			None,
			None,
		));

		// A regular file where the intake wants a directory: `create_dir_all` on
		// `<file>/direct` is ENOTDIR on every platform, with no permission games.
		let dir = tempdir().expect("tempdir");
		let blocker = dir.path().join("not-a-dir");
		std::fs::write(&blocker, b"occupied").expect("blocker written");
		let direct = blocker.join("direct");

		KernFileWatcherSink::new(worker, 0, Default::default(), Some(direct.clone()))
			.ingest(IngestRecord {
				source_uri: "file:///tmp/failopen.rs".to_string(),
				content: "fn still_ingested() {}".to_string(),
				language_hint: Some("rust".to_string()),
			})
			.await;

		assert!(
			!direct.exists(),
			"the durable write really failed; the test is not proving the happy path"
		);

		let mut n = 0;
		let cap = std::time::Instant::now() + Duration::from_secs(5);
		while std::time::Instant::now() < cap {
			n = g
				.read()
				.kerns
				.values()
				.flat_map(|k| k.entities.values())
				.filter(|e| matches!(e.kind, EntityKind::Document))
				.count();
			if n > 0 {
				break;
			}
			sleep(Duration::from_millis(25)).await;
		}
		assert_eq!(
			n, 1,
			"the record reached the graph exactly once — the queue caught it, and \
			 a failed park cannot also have committed"
		);
	}

	#[tokio::test]
	async fn duplicate_ingest_is_idempotent() {
		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let sink = DirectFileSink::new(g.clone());
		let rec = IngestRecord {
			source_uri: "file:///tmp/dup.rs".to_string(),
			content: "fn dup() {}".to_string(),
			language_hint: Some("rust".to_string()),
		};
		sink.ingest(rec.clone()).await;
		sink.ingest(rec).await;

		let g = g.read();
		assert_eq!(count_file_documents(&g), 1);
	}
}
