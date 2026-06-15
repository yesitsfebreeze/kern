use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::SystemTime;

use super::util;
use crate::crdt::GCounter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ChunkPartKind {
	Context = 0,
	StatementRef = 1,
}

/// Canonical entity kinds. Fixed enum — identical across every kern instance.
/// `Receipt` is **not** a kind (receipts live in the journal, not knowledge).
/// `Superseded` is **not** a kind — lifecycle moved to [`EntityStatus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum EntityKind {
	/// Verified high-confidence claim, immutable.
	Fact = 0,
	/// Default unverified statement.
	#[default]
	Claim = 1,
	/// Source artifact (file body, ticket body, session slice, agent blob).
	Document = 2,
	/// Open inquiry awaiting an answer.
	Question = 3,
	/// Resolution to a Question.
	Answer = 4,
	/// Synthesized stance over many Claims.
	Conclusion = 5,
}

impl EntityKind {
	/// Stable lower-case label. Used by the MCP query tool's `kind` filter.
	pub fn as_str(self) -> &'static str {
		match self {
			EntityKind::Fact => "fact",
			EntityKind::Claim => "claim",
			EntityKind::Document => "document",
			EntityKind::Question => "question",
			EntityKind::Answer => "answer",
			EntityKind::Conclusion => "conclusion",
		}
	}

	/// Parse a lower-case label. Returns `None` on unknown input.
	pub fn parse(s: &str) -> Option<Self> {
		match s {
			"fact" => Some(EntityKind::Fact),
			"claim" => Some(EntityKind::Claim),
			"document" => Some(EntityKind::Document),
			"question" => Some(EntityKind::Question),
			"answer" => Some(EntityKind::Answer),
			"conclusion" => Some(EntityKind::Conclusion),
			_ => None,
		}
	}
}

/// Lifecycle flag, orthogonal to [`EntityKind`].
///
/// `Superseded` migrated here from the old `ThoughtKind` enum: a Fact that
/// gets superseded retains `kind = Fact` but transitions `status = Superseded`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum EntityStatus {
	#[default]
	Active = 0,
	Superseded = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(i32)]
pub enum ReasonKind {
	#[default]
	Similarity = 0,
	Provenance = 1,
	Question = 2,
	Spawn = 3,
	Supersedes = 4,
	Ratification = 5,
	Rephrase = 6,
}

impl ReasonKind {
	/// Fallback display label for structural edges that have no LLM text.
	/// Returns `None` for kinds that carry no explanatory label.
	pub fn fallback_label(self) -> Option<&'static str> {
		match self {
			ReasonKind::Supersedes => Some("superseded by a newer version"),
			ReasonKind::Rephrase => Some("rephrased as"),
			_ => None,
		}
	}

	pub fn is_semantic(self) -> bool {
		matches!(
			self,
			ReasonKind::Similarity | ReasonKind::Provenance | ReasonKind::Ratification
		)
	}
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Acl {
	pub scope: String,
	pub users: Vec<String>,
	pub groups: Vec<String>,
}

/// Typed source value object. Each variant is one URI scheme.
///
/// Schemes:
/// - `file://<path>` — filesystem document.
/// - `ticket://<system>/<id>[#section]` — issue tracker artifact.
/// - `session://<id>[#slice]` — agent/repl session slice.
/// - `agent://<name>` — output produced by an agent.
/// - `inline://<hash>` — caller-supplied inline text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Source {
	File {
		path: String,
		section: String,
		title: String,
		author: String,
		url: String,
	},
	Ticket {
		system: String,
		object_id: String,
		section: String,
		title: String,
		author: String,
		url: String,
	},
	Session {
		session_id: String,
		section: String,
		title: String,
	},
	Agent {
		agent: String,
		object_id: String,
		title: String,
	},
	Inline {
		hash: String,
		section: String,
	},
}

impl Default for Source {
	fn default() -> Self {
		Source::Inline {
			hash: String::new(),
			section: String::new(),
		}
	}
}

impl Source {
	/// Stable URI scheme tag — `"file"`, `"ticket"`, `"session"`, `"agent"`,
	/// `"inline"`. Used by the MCP query tool's `scheme` filter.
	pub fn scheme(&self) -> &'static str {
		match self {
			Source::File { .. } => "file",
			Source::Ticket { .. } => "ticket",
			Source::Session { .. } => "session",
			Source::Agent { .. } => "agent",
			Source::Inline { .. } => "inline",
		}
	}

	/// Parse `"file"`/`"ticket"`/... into the matching scheme tag string.
	pub fn parse_scheme(s: &str) -> Option<&'static str> {
		match s {
			"file" => Some("file"),
			"ticket" => Some("ticket"),
			"session" => Some("session"),
			"agent" => Some("agent"),
			"inline" => Some("inline"),
			_ => None,
		}
	}

	pub fn object_id(&self) -> &str {
		match self {
			Source::File { path, .. } => path,
			Source::Ticket { object_id, .. } => object_id,
			Source::Session { session_id, .. } => session_id,
			Source::Agent { object_id, .. } => object_id,
			Source::Inline { hash, .. } => hash,
		}
	}

	pub fn section(&self) -> &str {
		match self {
			Source::File { section, .. } => section,
			Source::Ticket { section, .. } => section,
			Source::Session { section, .. } => section,
			Source::Agent { .. } => "",
			Source::Inline { section, .. } => section,
		}
	}

	pub fn title(&self) -> &str {
		match self {
			Source::File { title, .. }
			| Source::Ticket { title, .. }
			| Source::Session { title, .. }
			| Source::Agent { title, .. } => title,
			Source::Inline { .. } => "",
		}
	}

	pub fn author(&self) -> &str {
		match self {
			Source::File { author, .. } | Source::Ticket { author, .. } => author,
			_ => "",
		}
	}

	pub fn url(&self) -> &str {
		match self {
			Source::File { url, .. } | Source::Ticket { url, .. } => url,
			_ => "",
		}
	}

	/// Legacy descriptive system tag (e.g. ticket system "github"). Other
	/// variants synthesize from scheme. Used by retrieval filters.
	pub fn system(&self) -> &str {
		match self {
			Source::Ticket { system, .. } => system,
			Source::File { .. } => "file",
			Source::Session { .. } => "session",
			Source::Agent { agent, .. } => agent,
			Source::Inline { .. } => "inline",
		}
	}

	/// Stable, content-addressable id for this source location.
	pub fn source_id(&self) -> Option<String> {
		let scheme = self.scheme();
		let object = self.object_id();
		if object.is_empty() {
			return None;
		}
		Some(util::content_hash(&format!(
			"{}\x00{}\x00{}",
			scheme,
			object,
			self.section()
		)))
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkPart {
	pub kind: ChunkPartKind,
	pub text: String,
	pub index: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Entity {
	pub id: String,
	pub root_id: String,
	pub external_id: String,
	pub superseded_by: String,
	pub kind: EntityKind,
	#[serde(default)]
	pub status: EntityStatus,
	pub statements: Vec<String>,
	pub chunks: Vec<ChunkPart>,
	pub vector: Vec<f64>,
	pub gnn_vector: Vec<f64>,
	pub score: f64,
	#[serde(default)]
	pub conf_alpha: f32,
	#[serde(default)]
	pub conf_beta: f32,
	pub source: Source,
	#[serde(default)]
	pub created_at: Option<SystemTime>,
	pub acl: Acl,
	#[serde(default)]
	pub access_count: GCounter,
	pub accessed_at: Option<SystemTime>,
	#[serde(default)]
	pub heat: f32,
	#[serde(default)]
	pub heat_updated_at: Option<SystemTime>,
	#[serde(default)]
	pub updated_at: Option<SystemTime>,
	#[serde(default)]
	pub valid_until: Option<SystemTime>,
	pub producer_id: String,
	pub unlinked_count: i32,
	/// Set when the thought's text is edited in place (wiki-style). A dirty
	/// entity has a stale `vector`/`gnn_vector` until the reevaluation sweep
	/// re-embeds it and clears the flag. Persistent so an interrupted
	/// reevaluation resumes after restart.
	#[serde(default)]
	pub dirty: bool,
}

impl Entity {
	pub fn text(&self) -> String {
		let mut buf = String::new();
		for c in &self.chunks {
			match c.kind {
				ChunkPartKind::Context => buf.push_str(&c.text),
				ChunkPartKind::StatementRef => {
					if c.index < self.statements.len() {
						buf.push_str(&self.statements[c.index]);
					}
				}
			}
		}
		buf
	}

	/// Replace the thought's text in place (wiki-style edit) and mark it dirty
	/// so the reevaluation sweep re-embeds it. Collapses to a single Context
	/// chunk and drops the statement refs the original distillation produced.
	pub fn set_text(&mut self, text: String) {
		self.statements.clear();
		self.chunks = vec![ChunkPart {
			kind: ChunkPartKind::Context,
			text,
			index: 0,
		}];
		self.updated_at = Some(SystemTime::now());
		self.dirty = true;
	}

	pub fn is_fact(&self) -> bool {
		self.kind == EntityKind::Fact
	}

	pub fn is_superseded(&self) -> bool {
		self.status == EntityStatus::Superseded
	}

	pub fn has_vector(&self) -> bool {
		!self.vector.is_empty()
	}

	pub fn has_gnn_vector(&self) -> bool {
		!self.gnn_vector.is_empty()
	}

	pub fn conf_mean(&self) -> f64 {
		let a = self.conf_alpha as f64;
		let b = self.conf_beta as f64;
		let n = a + b;
		if n <= 0.0 {
			return 0.5;
		}
		a / n
	}

	pub fn conf_variance(&self) -> f64 {
		let a = self.conf_alpha as f64;
		let b = self.conf_beta as f64;
		let n = a + b;
		if n <= 0.0 {
			return 0.0;
		}
		(a * b) / (n * n * (n + 1.0))
	}

	pub fn refresh_score(&mut self) {
		self.score = self.conf_mean();
	}

	pub fn observe_support(&mut self, w: f64) {
		let w = w.clamp(0.0, 1.0) as f32;
		self.conf_alpha += w;
		self.refresh_score();
	}

	pub fn observe_contradict(&mut self, w: f64) {
		let w = w.clamp(0.0, 1.0) as f32;
		self.conf_beta += w;
		self.refresh_score();
	}
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Reason {
	pub id: String,
	pub from: String,
	pub to: String,
	pub to_kern_id: String,
	pub to_net_id: String,
	pub kind: ReasonKind,
	pub text: String,
	pub vector: Vec<f64>,
	pub score: f64,
	#[serde(default)]
	pub traversal_count: GCounter,
	pub producer_id: String,
	/// Set when the edge's text is edited in place. Its `vector` is recomputed
	/// by the reevaluation sweep (mean of its endpoints) and the flag cleared.
	#[serde(default)]
	pub dirty: bool,
}

impl Reason {
	/// Replace the edge's text in place and mark it dirty for reevaluation.
	pub fn set_text(&mut self, text: String) {
		self.text = text;
		self.dirty = true;
	}

	pub fn has_vector(&self) -> bool {
		!self.vector.is_empty()
	}

	pub fn is_enriched(&self) -> bool {
		!self.text.is_empty()
	}

	pub fn is_remote(&self) -> bool {
		!self.to_net_id.is_empty()
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRef {
	pub kern_id: String,
	pub entity_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Kern {
	pub id: String,
	pub root_id: String,
	pub anchor_text: String,
	pub anchor_vec: Vec<f64>,
	pub inner_radius: f64,
	pub outer_radius: f64,
	pub spawn_reason_id: String,
	pub parent: String,
	pub children: Vec<String>,

	pub entities: HashMap<String, Entity>,
	pub refs: HashMap<String, EntityRef>,
	pub reasons: HashMap<String, Reason>,
	pub by_from: HashMap<String, Vec<String>>,
	pub by_to: HashMap<String, Vec<String>>,
	pub source_index: HashMap<String, String>,
	pub descriptors: HashMap<String, String>,

	#[serde(default)]
	pub gnn_weights: Vec<u8>,

	#[serde(skip)]
	pub last_access: Option<SystemTime>,
}

/// Wall-clock nonce (nanoseconds since the epoch) that keeps generated kern ids
/// unique across rapid successive creations. Isolated here so the only
/// non-deterministic input to id derivation is a single, named call.
fn now_nanos() -> u128 {
	SystemTime::now()
		.duration_since(SystemTime::UNIX_EPOCH)
		.unwrap_or_default()
		.as_nanos()
}

/// Derive an unnamed kern's id from its parent and a uniqueness nonce. Pure —
/// split out of `new_unnamed` so the id scheme is deterministic for a fixed
/// `(parent_id, nonce)` and can be unit-tested without mocking the clock (the
/// public ctor supplies `now_nanos()`).
fn unnamed_kern_id(parent_id: &str, nonce_nanos: u128) -> String {
	util::content_hash(&format!("{parent_id}{nonce_nanos}"))
}

/// Derive a named child's id from parent, anchor name, and nonce. Pure (see
/// `unnamed_kern_id`); the name is folded into the hash so two anchors with
/// different names under the same parent never collide.
fn named_child_kern_id(parent_id: &str, name: &str, nonce_nanos: u128) -> String {
	util::content_hash(&format!("{parent_id}{name}{nonce_nanos}"))
}

impl Kern {
	pub fn new(id: impl Into<String>, parent_id: impl Into<String>) -> Self {
		Self {
			id: id.into(),
			parent: parent_id.into(),
			last_access: Some(SystemTime::now()),
			..Self::empty()
		}
	}

	pub fn new_root() -> Self {
		let mut k = Self::new("root", "");
		k.last_access = Some(SystemTime::now());
		k
	}

	pub fn new_unnamed(parent_id: &str, root_id: &str) -> Self {
		let mut k = Self::new(unnamed_kern_id(parent_id, now_nanos()), parent_id);
		k.root_id = root_id.to_string();
		k
	}

	/// A named child of `parent_id` carrying an anchor vector. An **anchor** is
	/// exactly this: a named kern directly under the root. `vec` may be empty
	/// (the `generic` catch-all), in which case similarity routing never matches
	/// it. Default radii come from the kern routing constants.
	pub fn new_named_child(parent_id: &str, root_id: &str, name: &str, vec: Vec<f64>) -> Self {
		let mut k = Self::new(named_child_kern_id(parent_id, name, now_nanos()), parent_id);
		k.root_id = root_id.to_string();
		k.anchor_text = name.to_string();
		k.anchor_vec = vec;
		k.inner_radius = crate::base::constants::KERN_INNER_RADIUS;
		k.outer_radius = crate::base::constants::KERN_OUTER_RADIUS;
		k
	}

	pub fn is_unnamed(&self) -> bool {
		self.anchor_text.is_empty()
	}

	pub fn is_named(&self) -> bool {
		!self.anchor_text.is_empty()
	}

	pub fn is_immortal(&self) -> bool {
		self.is_named()
	}

	pub fn is_dead(&self) -> bool {
		self.anchor_text.is_empty() && self.entities.is_empty()
	}

	pub fn has_anchor(&self) -> bool {
		!self.anchor_text.is_empty() && !self.anchor_vec.is_empty()
	}

	pub fn is_remote(&self) -> bool {
		self.id.starts_with("remote-")
	}

	fn empty() -> Self {
		Self {
			id: String::new(),
			root_id: String::new(),
			anchor_text: String::new(),
			anchor_vec: Vec::new(),
			inner_radius: 0.0,
			outer_radius: 0.0,
			spawn_reason_id: String::new(),
			parent: String::new(),
			children: Vec::new(),
			entities: HashMap::new(),
			refs: HashMap::new(),
			reasons: HashMap::new(),
			by_from: HashMap::new(),
			by_to: HashMap::new(),
			source_index: HashMap::new(),
			descriptors: HashMap::new(),
			gnn_weights: Vec::new(),
			last_access: None,
		}
	}
}

/// Shared test fixture: a minimal `Active` inline entity with the given
/// heat and kind. Used by tests across the base/retrieval/gossip modules
/// so the 25-field `Entity` literal lives in exactly one place.
#[cfg(test)]
pub(crate) fn mk_entity(id: &str, text: &str, heat: f64, kind: EntityKind) -> Entity {
	let mut e = Entity {
		id: id.to_string(),
		root_id: String::new(),
		external_id: String::new(),
		superseded_by: String::new(),
		kind,
		status: EntityStatus::Active,
		statements: vec![text.to_string()],
		chunks: vec![ChunkPart {
			kind: ChunkPartKind::StatementRef,
			text: String::new(),
			index: 0,
		}],
		vector: vec![0.0; 8],
		gnn_vector: Vec::new(),
		score: 0.0,
		conf_alpha: 2.0,
		conf_beta: 1.0,
		source: Source::Inline {
			hash: id.into(),
			section: String::new(),
		},
		created_at: None,
		acl: Acl::default(),
		access_count: GCounter::new(),
		accessed_at: None,
		heat: heat as f32,
		heat_updated_at: None,
		updated_at: None,
		valid_until: None,
		producer_id: String::new(),
		unlinked_count: 0,
		dirty: false,
	};
	e.refresh_score();
	e
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn entity_set_text_replaces_text_and_marks_dirty() {
		let mut e = Entity {
			statements: vec!["old statement".into()],
			chunks: vec![ChunkPart {
				kind: ChunkPartKind::StatementRef,
				text: String::new(),
				index: 0,
			}],
			..Default::default()
		};
		assert_eq!(e.text(), "old statement");
		assert!(!e.dirty);

		e.set_text("brand new text".into());

		assert_eq!(e.text(), "brand new text");
		assert!(e.dirty, "edit must mark the entity dirty for reevaluation");
		assert!(
			e.statements.is_empty(),
			"statement refs are dropped on edit"
		);
		assert!(e.updated_at.is_some());
	}

	#[test]
	fn reason_set_text_replaces_text_and_marks_dirty() {
		let mut r = Reason {
			text: "old edge".into(),
			..Default::default()
		};
		assert!(!r.dirty);
		r.set_text("new edge".into());
		assert_eq!(r.text, "new edge");
		assert!(r.dirty);
	}

	#[test]
	fn conf_mean_and_variance_handle_a_zero_total_prior() {
		// alpha = beta = 0 is a degenerate prior: mean falls back to 0.5, var to 0.
		let e = Entity {
			conf_alpha: 0.0,
			conf_beta: 0.0,
			..Default::default()
		};
		assert_eq!(e.conf_mean(), 0.5);
		assert_eq!(e.conf_variance(), 0.0);
	}

	#[test]
	fn conf_mean_and_variance_for_a_beta_prior() {
		// Beta(2,1): mean = a/(a+b) = 2/3; var = ab / ((a+b)^2 (a+b+1)) = 2/36.
		let e = Entity {
			conf_alpha: 2.0,
			conf_beta: 1.0,
			..Default::default()
		};
		assert!((e.conf_mean() - 2.0 / 3.0).abs() < 1e-12);
		assert!((e.conf_variance() - 2.0 / 36.0).abs() < 1e-12);
	}

	#[test]
	fn kern_is_dead_only_when_unnamed_and_empty() {
		let empty_unnamed = Kern::new("k", "");
		assert!(empty_unnamed.is_dead(), "unnamed + no entities -> dead");

		let mut named = Kern::new("k2", "");
		named.anchor_text = "topic".into();
		assert!(!named.is_dead(), "a name keeps an empty kern alive");

		let mut with_thought = Kern::new("k3", "");
		with_thought.entities.insert(
			"e".into(),
			Entity {
				id: "e".into(),
				..Default::default()
			},
		);
		assert!(
			!with_thought.is_dead(),
			"an entity keeps an unnamed kern alive"
		);
	}

	#[test]
	fn kern_has_anchor_requires_both_text_and_vector() {
		let mut k = Kern::new("k", "");
		assert!(!k.has_anchor(), "fresh kern has no anchor");
		k.anchor_text = "topic".into();
		assert!(
			!k.has_anchor(),
			"text without a vector is not a full anchor"
		);
		k.anchor_vec = vec![0.1, 0.2];
		assert!(k.has_anchor(), "text + vector -> anchored");
	}

	#[test]
	fn new_named_child_sets_anchor_parent_and_root() {
		let k = Kern::new_named_child("parent", "rootid", "generic", vec![0.5, 0.5]);
		assert_eq!(k.parent, "parent");
		assert_eq!(k.root_id, "rootid");
		assert_eq!(k.anchor_text, "generic");
		assert_eq!(k.anchor_vec, vec![0.5, 0.5]);
		assert!(k.is_named() && k.has_anchor() && !k.is_dead());
		assert!(!k.id.is_empty(), "id is the content hash, never empty");
	}

	#[test]
	fn kern_id_derivation_is_deterministic_and_input_sensitive() {
		// Deterministic for a fixed (parent, nonce) — the property that the bare
		// SystemTime::now() call previously made un-testable.
		assert_eq!(unnamed_kern_id("p", 42), unnamed_kern_id("p", 42));
		assert_eq!(
			named_child_kern_id("p", "code", 9),
			named_child_kern_id("p", "code", 9)
		);
		// The nonce disambiguates rapid successive creations under one parent.
		assert_ne!(unnamed_kern_id("p", 1), unnamed_kern_id("p", 2));
		// Parent and anchor name both feed the hash.
		assert_ne!(unnamed_kern_id("a", 7), unnamed_kern_id("b", 7));
		assert_ne!(
			named_child_kern_id("p", "code", 9),
			named_child_kern_id("p", "docs", 9)
		);
		// Ids are non-empty content hashes.
		assert!(!unnamed_kern_id("p", 0).is_empty());
		assert!(!named_child_kern_id("p", "x", 0).is_empty());
	}

	#[test]
	fn entity_kind_serde_roundtrip() {
		for k in [
			EntityKind::Fact,
			EntityKind::Claim,
			EntityKind::Document,
			EntityKind::Question,
			EntityKind::Answer,
			EntityKind::Conclusion,
		] {
			let json = serde_json::to_string(&k).expect("serialize");
			let back: EntityKind = serde_json::from_str(&json).expect("deserialize");
			assert_eq!(k, back, "roundtrip failed for {k:?}");
			assert_eq!(EntityKind::parse(k.as_str()), Some(k));
		}
	}

	#[test]
	fn entity_status_default_is_active() {
		assert_eq!(EntityStatus::default(), EntityStatus::Active);
	}

	#[test]
	fn source_scheme_returns_correct_tag() {
		let cases: &[(Source, &str)] = &[
			(
				Source::File {
					path: "/x".into(),
					section: String::new(),
					title: String::new(),
					author: String::new(),
					url: String::new(),
				},
				"file",
			),
			(
				Source::Ticket {
					system: "gh".into(),
					object_id: "1".into(),
					section: String::new(),
					title: String::new(),
					author: String::new(),
					url: String::new(),
				},
				"ticket",
			),
			(
				Source::Session {
					session_id: "s".into(),
					section: String::new(),
					title: String::new(),
				},
				"session",
			),
			(
				Source::Agent {
					agent: "a".into(),
					object_id: "o".into(),
					title: String::new(),
				},
				"agent",
			),
			(
				Source::Inline {
					hash: "h".into(),
					section: String::new(),
				},
				"inline",
			),
		];
		for (src, want) in cases {
			assert_eq!(src.scheme(), *want);
		}
		assert!(Source::parse_scheme("file").is_some());
		assert!(Source::parse_scheme("bogus").is_none());
	}
}
