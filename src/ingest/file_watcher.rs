//! Bridges `src/watcher` events into kern's canonical ingest path: each
//! `IngestRecord` becomes a `Document` job through `Worker::enqueue`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use watcher::{FileWatcher, IgnoreRules, IngestPipeline, IngestRecord, IngestSink, WatcherError};

use crate::base::types::{EntityKind, Source};
use crate::ingest::{Config as IngestRunConfig, Worker};

/// Strip the `file://`/`file:///` prefix from `watcher::pipeline::file_uri`;
/// input without a prefix passes through unchanged (defensive).
fn strip_file_uri(uri: &str) -> String {
	if let Some(rest) = uri.strip_prefix("file:///") {
		// Empty authority. Windows `file:///C:/foo` → `C:/foo`; POSIX `file:///abs`
		// → `abs` (path minus the empty authority's leading slash).
		return rest.to_string();
	}
	if let Some(rest) = uri.strip_prefix("file://") {
		// Non-empty authority: `file://host/path`. Per RFC 8089 the local path is
		// everything from the first '/', so DROP the host (`file://host/p` -> `/p`).
		return match rest.find('/') {
			Some(i) => rest[i..].to_string(),
			None => String::new(), // bare `file://host` with no path
		};
	}
	uri.to_string()
}

/// Forwards each filesystem ingest record through the shared `Worker`. Cheap to
/// clone (one `Arc`) — the `IngestPipeline` takes its sink by value.
#[derive(Clone)]
pub struct KernFileWatcherSink {
	worker: Arc<Worker>,
}

impl KernFileWatcherSink {
	pub fn new(worker: Arc<Worker>) -> Self {
		Self { worker }
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

		let descriptor = language_hint.unwrap_or_default();

		// Fire-and-forget; `place_document`'s dedup keeps the entity count
		// stable when the same file is re-ingested.
		self.worker.enqueue(
			content,
			source,
			EntityKind::Document,
			descriptor,
			1.0,
			IngestRunConfig::default(),
		);
	}
}

/// Watcher event pump: runs until the `FileWatcher` channel closes; surfaces
/// watcher construction errors to the caller.
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
	use crate::base::types::{Acl, ChunkPart, ChunkPartKind, Entity, EntityStatus};
	use crate::base::util;
	use crate::crdt::GCounter;

	/// Bypasses the embed-backed `Worker`, writing directly into a graph so
	/// tests stay hermetic.
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
				statements: vec![text],
				chunks: vec![ChunkPart {
					kind: ChunkPartKind::StatementRef,
					text: String::new(),
					index: 0,
				}],
				vector: vec,
				gnn_vector: Vec::new(),
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

	/// Real `FileWatcher` + sink; the pipeline is driven manually so the test
	/// doesn't wire the kern startup path.
	#[tokio::test]
	async fn watcher_pipeline_creates_document_for_new_file() {
		let dir = tempdir().expect("tempdir");
		let root = dir.path().to_path_buf();

		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let sink = DirectFileSink::new(g.clone());

		let mut fw = FileWatcher::new(vec![root.clone()], IgnoreRules::empty()).expect("watcher new");
		let pipeline = IngestPipeline::new(sink);

		// Give the watcher a moment to register before we touch the fs.
		sleep(Duration::from_millis(100)).await;

		let target = root.join("note.md");
		std::fs::write(&target, "hello watcher").expect("write file");

		// Drain debounced events until the graph reflects the ingest (<= ~2s).
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

	/// Same record twice → one entity: identical text → identical content-hash
	/// id, matching production's `find_duplicate` dedup.
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
