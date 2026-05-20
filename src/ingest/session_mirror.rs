//! Slice K — session mirror.
//!
//! Tails the shared journal for `ForkOpen` / `ForkResume` / `ForkClose`
//! lifecycle events and ingests each fork into the kern as a
//! `Document` entity with `Source::Session { session_id = fork_id }`.
//! Sessions then become first-class citizens in the relay search palette
//! via the `:session` facet.
//!
//! Design notes
//! - The mirror does NOT duplicate ingest logic; it forwards every new
//!   fork through the shared `Worker` (in production) or a direct
//!   `accept` path (in tests where embedding services are unavailable).
//!   Both reuse the canonical `Entity` shape from `place.rs` /
//!   `build_chunk_entity`.
//! - A fork is mirrored exactly once. The `seen` set keys on `fork_id`
//!   so replaying a journal — including across process restarts and
//!   the in-memory poll loop — never produces duplicate entities.
//! - We treat `ForkResume` as a no-op against `seen` (the fork is
//!   already mirrored from its `ForkOpen`); `ForkClose` is recorded but
//!   does not delete the entity, so historical sessions stay
//!   searchable.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use journal::{Entry, Filter, History, Kind};
use tokio::sync::Mutex;

use crate::base::types::{EntityKind, Source};
use crate::ingest::Worker;

#[cfg(test)]
use std::sync::RwLock;
#[cfg(test)]
use std::time::SystemTime;
#[cfg(test)]
use crate::base::accept;
#[cfg(test)]
use crate::base::graph::GraphGnn;
#[cfg(test)]
use crate::base::types::{Acl, ChunkPart, ChunkPartKind, Entity, EntityStatus};
#[cfg(test)]
use crate::base::util;
#[cfg(test)]
use crate::crdt::GCounter;

/// Pluggable target for mirrored sessions. Implementations must be
/// idempotent on `fork_id` (the mirror itself dedupes via its `seen`
/// set, but a sink may be invoked multiple times across process
/// restarts before the seen set is rehydrated).
pub trait MirrorSink: Send + Sync {
	fn ingest_session(&self, fork_id: &str, parent: Option<&str>, text: &str);
}

/// Production sink — forwards to the shared `Worker` so the canonical
/// embed → place_document path runs (no logic duplication).
pub struct WorkerSink {
	worker: Arc<Worker>,
}

impl WorkerSink {
	pub fn new(worker: Arc<Worker>) -> Self {
		Self { worker }
	}
}

impl MirrorSink for WorkerSink {
	fn ingest_session(&self, fork_id: &str, parent: Option<&str>, text: &str) {
		let source = Source::Session {
			session_id: fork_id.to_string(),
			section: String::new(),
			title: format!("session://{fork_id}"),
		};
		let descriptor = match parent {
			Some(p) => format!("session fork_id={fork_id} parent={p}"),
			None => format!("session fork_id={fork_id}"),
		};
		// Fire-and-forget. The mirror's seen-set guarantees we won't
		// re-enqueue the same fork until the mirror itself restarts; if
		// a restart happens before the worker drains, dedup at the
		// `find_duplicate` step in `place_document` handles it
		// (vector-identical text → existing entity update).
		self.worker.enqueue(
			text.to_string(),
			source,
			EntityKind::Document,
			descriptor,
			1.0,
			crate::ingest::Config::default(),
		);
	}
}

/// In-process sink that bypasses the embedder and writes directly into
/// the graph via `accept::accept`. Used by unit tests where no embed
/// service exists; never wired into production. Vectors are deterministic
/// stubs (a constant unit-norm vector seeded from `fork_id`) so dedup,
/// search, and persistence still operate on a well-formed entity.
#[cfg(test)]
pub struct DirectSink {
	graph: Arc<RwLock<GraphGnn>>,
}

#[cfg(test)]
impl DirectSink {
	pub fn new(graph: Arc<RwLock<GraphGnn>>) -> Self {
		Self { graph }
	}

	fn stub_vector(seed: &str) -> Vec<f64> {
		// Near-orthogonal unit vector per fork id: a 256-dim one-hot
		// derived from the hash. Two distinct fork_ids almost certainly
		// land in different slots, so cosine similarity is ~0 and
		// `commit_entity`'s dedup check (similarity > threshold) is
		// dodged. This is test-only; production uses real embeddings.
		let h = util::content_hash(seed);
		let bytes = h.as_bytes();
		let slot = if bytes.is_empty() { 0 } else { bytes[0] as usize };
		let mut v = vec![0.0_f64; 256];
		v[slot] = 1.0;
		v
	}
}

#[cfg(test)]
impl MirrorSink for DirectSink {
	fn ingest_session(&self, fork_id: &str, parent: Option<&str>, text: &str) {
		let source = Source::Session {
			session_id: fork_id.to_string(),
			section: String::new(),
			title: format!("session://{fork_id}"),
		};
		let vec = Self::stub_vector(fork_id);
		let id = util::content_hash(text);
		let mut t = Entity {
			id,
			root_id: String::new(),
			external_id: source.object_id().to_string(),
			superseded_by: String::new(),
			kind: EntityKind::Document,
			status: EntityStatus::Active,
			statements: vec![text.to_string()],
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
		};
		t.refresh_score();
		let _ = parent; // recorded in descriptor by WorkerSink path; unused here
		let root_id = match self.graph.read() {
			Ok(g) => g.root.id.clone(),
			Err(_) => return,
		};
		if let Ok(mut g) = self.graph.write() {
			accept::accept(&mut g, &root_id, t, "");
		}
	}
}

/// Stateful mirror. Tracks `seen` fork ids so re-replaying the journal
/// (e.g. during the periodic poll) never re-enqueues the same fork.
pub struct SessionMirror<S: MirrorSink> {
	sink: S,
	seen: HashSet<String>,
	seen_order: VecDeque<String>,
	max_seen: usize,
	last_ts_ms: u64,
}

const DEFAULT_MAX_SEEN: usize = 4096;

impl<S: MirrorSink> SessionMirror<S> {
	pub fn new(sink: S) -> Self {
		Self {
			sink,
			seen: HashSet::new(),
			seen_order: VecDeque::new(),
			max_seen: DEFAULT_MAX_SEEN,
			last_ts_ms: 0,
		}
	}

	pub fn set_max_seen(&mut self, cap: usize) {
		self.max_seen = cap.max(1);
		while self.seen_order.len() > self.max_seen {
			if let Some(old) = self.seen_order.pop_front() {
				self.seen.remove(&old);
			}
		}
	}

	fn remember(&mut self, fork_id: String) {
		self.seen.insert(fork_id.clone());
		self.seen_order.push_back(fork_id);
		while self.seen_order.len() > self.max_seen {
			if let Some(old) = self.seen_order.pop_front() {
				self.seen.remove(&old);
			}
		}
	}

	/// Process one journal entry. Idempotent on `fork_id`. Returns
	/// `true` when the entry was dropped by the kern-self-feed filter
	/// so the caller can aggregate a per-tick drop count for telemetry.
	pub fn process(&mut self, entry: &Entry) -> bool {
		if entry.ts_ms > self.last_ts_ms {
			self.last_ts_ms = entry.ts_ms;
		}
		// kern self-feed loop: kern's own tracing must not be re-ingested as user/session data.
		if let Some(src) = entry.payload.get("src").and_then(|v| v.as_str()) {
			if src.starts_with("kern") {
				return true;
			}
		}
		match &entry.kind {
			Kind::ForkOpen { fork_id, parent } => {
				if self.seen.contains(fork_id) {
					return false;
				}
				let parent_label = parent.as_deref().unwrap_or("none");
				let text = format!("Session {fork_id} (parent={parent_label})");
				self
					.sink
					.ingest_session(fork_id, parent.as_deref(), &text);
				self.remember(fork_id.clone());
			}
			Kind::ForkResume { fork_id } => {
				// Idempotent: if we've never seen this fork (e.g. journal
				// starts mid-life), treat resume like an open so the
				// session still ends up mirrored.
				if self.seen.contains(fork_id) {
					return false;
				}
				let text = format!("Session {fork_id} (parent=none)");
				self.sink.ingest_session(fork_id, None, &text);
				self.remember(fork_id.clone());
			}
			Kind::ForkClose { .. } => {
				// Closing does not remove the document; sessions stay
				// searchable post-close. No-op for now.
			}
			_ => {}
		}
		false
	}

	pub fn process_all<'a, I: IntoIterator<Item = &'a Entry>>(&mut self, entries: I) {
		let mut dropped = 0_usize;
		for e in entries {
			if self.process(e) {
				dropped += 1;
			}
		}
		if dropped > 0 {
			tracing::debug!(
				target: "kern.session_mirror",
				dropped,
				"kern self-produced entries filtered"
			);
		}
	}

	pub fn seen_count(&self) -> usize {
		self.seen.len()
	}
}

/// Background task. Polls the SQLite history every `interval` for new
/// fork lifecycle events and forwards them through `mirror`. Reads
/// each `fork_*` kind separately because `Filter` is keyed on a single
/// `Kind` (cheap: three small queries every few seconds).
///
/// The task runs forever; spawn it on startup and let it die with the
/// process. `Arc<Mutex<...>>` lets future code (e.g. an admin probe)
/// inspect the mirror without racing with the poll loop.
pub async fn run<S: MirrorSink + 'static>(
	history: Arc<History>,
	mirror: Arc<Mutex<SessionMirror<S>>>,
	interval: Duration,
) {
	loop {
		let since = {
			let m = mirror.lock().await;
			m.last_ts_ms
		};
		// Fetch each of the three fork kinds; merge in ts order.
		let mut all: Vec<Entry> = Vec::new();
		for kind in [
			Kind::ForkOpen {
				fork_id: String::new(),
				parent: None,
			},
			Kind::ForkResume {
				fork_id: String::new(),
			},
			Kind::ForkClose {
				fork_id: String::new(),
			},
		] {
			let f = Filter {
				kind: Some(kind),
				since_ms: if since > 0 { Some(since + 1) } else { None },
				..Filter::default()
			};
			match history.query(f) {
				Ok(rows) => all.extend(rows),
				Err(e) => {
					tracing::warn!(target: "kern.session_mirror", error = %e, "history query failed");
				}
			}
		}
		all.sort_by_key(|e| e.ts_ms);
		if !all.is_empty() {
			let mut m = mirror.lock().await;
			m.process_all(all.iter());
		}
		tokio::time::sleep(interval).await;
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use journal::Entry;

	fn fork_open(ts: u64, fork_id: &str, parent: Option<&str>) -> Entry {
		Entry {
			v: 5,
			ts_ms: ts,
			kind: Kind::ForkOpen {
				fork_id: fork_id.to_string(),
				parent: parent.map(|s| s.to_string()),
			},
			key: fork_id.to_string(),
			payload: serde_json::Value::Null,
		}
	}

	fn fork_close(ts: u64, fork_id: &str) -> Entry {
		Entry {
			v: 5,
			ts_ms: ts,
			kind: Kind::ForkClose {
				fork_id: fork_id.to_string(),
			},
			key: fork_id.to_string(),
			payload: serde_json::Value::Null,
		}
	}

	/// Two ForkOpen + two ForkClose → two `Document` entities with
	/// `source.scheme() == "session"`, one per fork_id.
	#[test]
	fn two_forks_mirror_to_two_documents() {
		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let sink = DirectSink::new(g.clone());
		let mut mirror = SessionMirror::new(sink);

		let entries = vec![
			fork_open(100, "fork-a", None),
			fork_open(200, "fork-b", Some("fork-a")),
			fork_close(300, "fork-a"),
			fork_close(400, "fork-b"),
		];
		mirror.process_all(entries.iter());
		assert_eq!(mirror.seen_count(), 2);

		// Walk the graph and collect Session-scheme Document entities.
		let g = g.read().expect("graph lock");
		let mut session_ids: Vec<String> = Vec::new();
		for kern in g.kerns.values() {
			for t in kern.entities.values() {
				if matches!(t.kind, EntityKind::Document)
					&& t.source.scheme() == "session"
				{
					session_ids.push(t.source.object_id().to_string());
				}
			}
		}
		session_ids.sort();
		assert_eq!(session_ids, vec!["fork-a".to_string(), "fork-b".to_string()]);
	}

	/// Replaying the same journal twice must not produce duplicate
	/// entities (seen-set dedupe).
	#[test]
	fn replay_is_idempotent() {
		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let sink = DirectSink::new(g.clone());
		let mut mirror = SessionMirror::new(sink);

		let entries = vec![
			fork_open(100, "fork-a", None),
			fork_open(200, "fork-b", None),
		];
		mirror.process_all(entries.iter());
		mirror.process_all(entries.iter()); // second pass

		assert_eq!(mirror.seen_count(), 2);

		let g = g.read().expect("graph lock");
		let count = g
			.kerns
			.values()
			.flat_map(|k| k.entities.values())
			.filter(|t| t.source.scheme() == "session")
			.count();
		assert_eq!(count, 2);
	}

	/// Filtering by source scheme `"session"` returns only the mirrored
	/// session entities (palette `:session` facet path).
	#[test]
	fn filter_by_session_scheme_returns_mirrored_only() {
		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let sink = DirectSink::new(g.clone());
		let mut mirror = SessionMirror::new(sink);

		mirror.process(&fork_open(50, "only-fork", None));

		let g = g.read().expect("graph lock");
		let matching: Vec<&Entity> = g
			.kerns
			.values()
			.flat_map(|k| k.entities.values())
			.filter(|t| t.source.scheme() == "session")
			.collect();
		assert_eq!(matching.len(), 1);
		assert_eq!(matching[0].source.object_id(), "only-fork");
		assert!(matching[0].statements[0].contains("only-fork"));
	}

	/// `ForkResume` for a never-seen fork still mirrors it (mid-life
	/// journal / restart safety).
	#[test]
	fn resume_without_prior_open_still_mirrors() {
		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let sink = DirectSink::new(g.clone());
		let mut mirror = SessionMirror::new(sink);

		let resume = Entry {
			v: 5,
			ts_ms: 10,
			kind: Kind::ForkResume {
				fork_id: "resumed".into(),
			},
			key: "resumed".into(),
			payload: serde_json::Value::Null,
		};
		mirror.process(&resume);
		assert_eq!(mirror.seen_count(), 1);
	}
}
