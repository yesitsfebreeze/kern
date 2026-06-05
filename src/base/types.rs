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
}

impl Reason {
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
	pub purpose_text: String,
	pub purpose_vec: Vec<f64>,
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
		let id = util::content_hash(&format!(
			"{}{}",
			parent_id,
			SystemTime::now()
				.duration_since(SystemTime::UNIX_EPOCH)
				.unwrap_or_default()
				.as_nanos()
		));
		let mut k = Self::new(id, parent_id);
		k.root_id = root_id.to_string();
		k
	}

	pub fn is_unnamed(&self) -> bool {
		self.purpose_text.is_empty()
	}

	pub fn is_named(&self) -> bool {
		!self.purpose_text.is_empty()
	}

	pub fn is_immortal(&self) -> bool {
		self.is_named()
	}

	pub fn is_dead(&self) -> bool {
		self.purpose_text.is_empty() && self.entities.is_empty()
	}

	pub fn has_purpose(&self) -> bool {
		!self.purpose_text.is_empty() && !self.purpose_vec.is_empty()
	}

	pub fn is_remote(&self) -> bool {
		self.id.starts_with("remote-")
	}

	fn empty() -> Self {
		Self {
			id: String::new(),
			root_id: String::new(),
			purpose_text: String::new(),
			purpose_vec: Vec::new(),
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
	};
	e.refresh_score();
	e
}

#[cfg(test)]
mod tests {
	use super::*;

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
