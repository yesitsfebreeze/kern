//! DTOs for [`SearchSvc`](super::SearchSvc) — mirror types that intentionally
//! do NOT depend on the `kern` crate; kern translates at the wire boundary.

use serde::{Deserialize, Serialize};

/// Lightweight mirror of `kern::EntityKind`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum EntityKindLite {
	Fact,
	/// Default unverified statement — mirrors `kern::EntityKind`'s own default.
	#[default]
	Claim,
	Document,
	Question,
	Answer,
	Conclusion,
	Superseded,
}

impl EntityKindLite {
	/// Single source of truth for label→kind. `None` (unknown labels AND
	/// `"superseded"`, a status not a kind) means "no kind filter", never "match nothing".
	pub fn from_label(s: &str) -> Option<Self> {
		match s {
			"fact" => Some(Self::Fact),
			"claim" => Some(Self::Claim),
			"document" => Some(Self::Document),
			"question" => Some(Self::Question),
			"answer" => Some(Self::Answer),
			"conclusion" => Some(Self::Conclusion),
			_ => None,
		}
	}
}

/// Lightweight mirror of `kern::EntityStatus` — orthogonal lifecycle flag.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum EntityStatusLite {
	#[default]
	Active,
	Superseded,
}

/// Mirror of `kern::Reason` kinds — one variant per typed edge.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum EdgeKind {
	#[default]
	Answers,
	Supports,
	Contradicts,
	Extends,
	Requires,
	References,
	Derives,
	Instances,
	PartOf,
	Consolidates,
}

/// One enriched relationship edge attached to a search hit.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EdgeRef {
	pub from: String,
	pub to: String,
	pub kind: EdgeKind,
	/// LLM-generated sentence naming the `from` → `to` link mechanism. Empty
	/// until kern tick enrichment — callers should skip unenriched edges.
	pub text: String,
	/// Cosine similarity between the two endpoint vectors.
	pub score: f32,
}

/// One result row delivered to the palette — only what `Card` needs to
/// render, plus the id used to drill in.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EntityRef {
	pub id: String,
	pub kind: EntityKindLite,
	pub status: EntityStatusLite,
	/// URI scheme without the `://` (e.g. `file`, `ticket`, `inline`).
	pub scheme: String,
	pub label: String,
	/// Short snippet shown under the label; already server-truncated.
	pub snippet: String,
	/// Fused score (HNSW + BM25 + PageRank + heat). Higher = better.
	pub score: f32,
	/// Only edges with a non-empty `text` sentence are included. Empty when
	/// none exist or the response predates this field.
	#[serde(default)]
	pub edges: Vec<EdgeRef>,
}

/// One filter chip. `scheme` and `kind` are independently optional — a facet
/// constrains either axis or both (e.g. `>file !fact`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Facet {
	pub scheme: Option<String>,
	pub kind: Option<EntityKindLite>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchReq {
	pub query: String,
	pub facets: Vec<Facet>,
	pub k: u32,
	/// Monotonic per-keystroke token; newer supersedes older. Servers may use
	/// it to early-return stale work.
	pub cancel_token: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchRes {
	pub hits: Vec<EntityRef>,
	/// True iff this response was for the most-recent `cancel_token`
	/// the server has seen. The client may discard stale frames.
	pub fresh: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NeighborsReq {
	pub entity_id: String,
	/// Empty = all edge kinds.
	pub edge_kinds: Vec<EdgeKind>,
	/// Server clamps to `[0, 3]`.
	pub depth: u8,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NeighborsRes {
	pub neighbors: Vec<EntityRef>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PreviewReq {
	pub entity_id: String,
}

/// Preview pane payload; the palette dispatches a sub-renderer on the
/// discriminant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PreviewRes {
	/// File-backed entity. `language` is a tree-sitter grammar id
	/// (`"rust"`, `"python"`, ...) or `None` for plain text.
	File {
		path: String,
		content: String,
		language: Option<String>,
	},
	/// Generic entity body — Fact, Claim, Conclusion, etc.
	Text { content: String },
	/// Reason edge between two entities; rendered as a sentence.
	Edge {
		from_label: String,
		to_label: String,
		kind: EdgeKind,
		sentence: String,
	},
}

#[cfg(test)]
mod dto_serde_tests {
	use super::*;

	#[test]
	fn entity_ref_roundtrips_through_serde_json() {
		let original = EntityRef {
			id: "e1".into(),
			kind: EntityKindLite::Fact,
			status: EntityStatusLite::Active,
			scheme: "file".into(),
			label: "main.rs".into(),
			snippet: "fn main() {}".into(),
			score: 0.92,
			edges: vec![EdgeRef {
				from: "e1".into(),
				to: "e2".into(),
				kind: EdgeKind::Supports,
				text: "e1 provides the indexing mechanism that e2 depends on".into(),
				score: 0.87,
			}],
		};
		let json = serde_json::to_string(&original).unwrap();
		let back: EntityRef = serde_json::from_str(&json).unwrap();
		assert_eq!(back.id, original.id);
		assert_eq!(back.kind, original.kind);
		assert_eq!(back.scheme, original.scheme);
		assert!((back.score - original.score).abs() < f32::EPSILON);
		assert_eq!(back.edges.len(), 1);
		assert_eq!(back.edges[0].text, original.edges[0].text);
	}

	#[test]
	fn entity_ref_with_no_edges_roundtrips_json() {
		let json = r#"{"id":"e0","kind":"Fact","status":"Active","scheme":"inline","label":"x","snippet":"y","score":0.5}"#;
		let back: EntityRef = serde_json::from_str(json).unwrap();
		assert!(
			back.edges.is_empty(),
			"missing edges field defaults to empty vec"
		);
	}

	#[test]
	fn entity_ref_roundtrips_through_bincode() {
		let original = EntityRef {
			id: "e2".into(),
			kind: EntityKindLite::Question,
			status: EntityStatusLite::Superseded,
			scheme: "ticket".into(),
			label: "T-9".into(),
			snippet: "why?".into(),
			score: 0.1,
			edges: vec![],
		};
		let cfg = bincode::config::standard();
		let bytes = bincode::serde::encode_to_vec(&original, cfg).unwrap();
		let (back, _): (EntityRef, _) = bincode::serde::decode_from_slice(&bytes, cfg).unwrap();
		assert_eq!(back.id, original.id);
		assert_eq!(back.status, original.status);
	}

	#[test]
	fn preview_res_variants_roundtrip_json() {
		let cases = vec![
			PreviewRes::File {
				path: "a.rs".into(),
				content: "x".into(),
				language: Some("rust".into()),
			},
			PreviewRes::Text {
				content: "claim".into(),
			},
			PreviewRes::Edge {
				from_label: "A".into(),
				to_label: "B".into(),
				kind: EdgeKind::Supports,
				sentence: "A supports B.".into(),
			},
		];
		for c in cases {
			let s = serde_json::to_string(&c).unwrap();
			let back: PreviewRes = serde_json::from_str(&s).unwrap();
			assert_eq!(back, c, "PreviewRes survives a JSON round-trip");
		}
	}

	#[test]
	fn search_req_cancel_token_roundtrips_through_bincode() {
		let cfg = bincode::config::standard();
		for token in [None, Some(0u64), Some(42), Some(u64::MAX)] {
			let req = SearchReq {
				query: "q".into(),
				facets: vec![],
				k: 5,
				cancel_token: token,
			};
			let bytes = bincode::serde::encode_to_vec(&req, cfg).unwrap();
			let (back, _): (SearchReq, _) = bincode::serde::decode_from_slice(&bytes, cfg).unwrap();
			assert_eq!(
				back.cancel_token, token,
				"Option<u64> cancel_token survives bincode"
			);
		}
	}

	#[test]
	fn entity_ref_default_is_empty_with_sensible_kind_and_status() {
		let d = EntityRef::default();
		assert!(d.id.is_empty() && d.edges.is_empty());
		assert_eq!(
			d.kind,
			EntityKindLite::Claim,
			"default kind mirrors kern's Claim"
		);
		assert_eq!(
			d.status,
			EntityStatusLite::Active,
			"default status is Active"
		);
		assert_eq!(EntityRef::default(), d);
	}
}
