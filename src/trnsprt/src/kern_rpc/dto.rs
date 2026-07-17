//! DTOs for [`KernRpc`](super::KernRpc) — mirror types that intentionally do
//! NOT depend on the `kern` crate; kern translates at the wire boundary.

use serde::{Deserialize, Serialize};

pub use crate::search::dto::{
	EdgeKind, EntityKindLite, EntityRef, EntityStatusLite, NeighborsReq, NeighborsRes,
};

/// Lightweight mirror of `kern::Source`, one variant per URI scheme. Optional
/// fields collapse to `""` on the wire (matches the kern-side `Default`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SourceLite {
	File {
		path: String,
		#[serde(default)]
		section: String,
		#[serde(default)]
		title: String,
		#[serde(default)]
		author: String,
		#[serde(default)]
		url: String,
	},
	Ticket {
		system: String,
		object_id: String,
		#[serde(default)]
		section: String,
		#[serde(default)]
		title: String,
		#[serde(default)]
		author: String,
		#[serde(default)]
		url: String,
	},
	Session {
		session_id: String,
		#[serde(default)]
		section: String,
		#[serde(default)]
		title: String,
	},
	Agent {
		agent: String,
		#[serde(default)]
		object_id: String,
		#[serde(default)]
		title: String,
	},
	Inline {
		#[serde(default)]
		hash: String,
		#[serde(default)]
		section: String,
	},
}

impl Default for SourceLite {
	fn default() -> Self {
		SourceLite::Inline {
			hash: String::new(),
			section: String::new(),
		}
	}
}

impl SourceLite {
	/// Stable URI scheme tag — matches `kern::Source::scheme`.
	pub fn scheme(&self) -> &'static str {
		match self {
			SourceLite::File { .. } => "file",
			SourceLite::Ticket { .. } => "ticket",
			SourceLite::Session { .. } => "session",
			SourceLite::Agent { .. } => "agent",
			SourceLite::Inline { .. } => "inline",
		}
	}
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QueryReq {
	pub text: String,
	/// Number of hits to return. Server clamps to a sane maximum.
	pub k: u32,
	/// Same wire strings as MCP `query.mode` (`"hybrid"` | `"vector"` |
	/// `"lexical"`); empty defaults to `"hybrid"`.
	#[serde(default)]
	pub mode: String,
	/// If true, kern attempts an LLM-synthesised answer alongside hits.
	#[serde(default)]
	pub answer: bool,
	/// Optional kind filter (lower-case label, e.g. `"fact"`).
	#[serde(default)]
	pub kind: String,
	/// Optional source-scheme filter (e.g. `"file"`).
	#[serde(default)]
	pub source: String,
	/// Cancellation/freshness token, mirrors `SearchSvc::SearchReq`.
	#[serde(default)]
	pub cancel_token: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QueryRes {
	/// Ranked entity hits ([`EntityRef`] shared with `SearchSvc`).
	pub hits: Vec<EntityRef>,
	/// LLM answer when requested; empty when no LLM is configured server-side.
	#[serde(default)]
	pub answer: String,
	/// True iff this response was for the most-recent `cancel_token`
	/// the server has seen. Mirrors `SearchRes::fresh`.
	#[serde(default = "default_true")]
	pub fresh: bool,
}

/// Missing `fresh` on the wire means "not stale" — bool's derived `Default`
/// (`false`) would invert that.
fn default_true() -> bool {
	true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IngestReq {
	pub text: String,
	pub source: SourceLite,
	pub kind: EntityKindLite,
	/// Descriptor classifier for the ingest pipeline; `None` skips routing.
	#[serde(default)]
	pub descriptor: Option<String>,
	/// Confidence in [0.0, 1.0]. Server clamps to its agent-source
	/// ceiling (Fact tier requires user-source).
	#[serde(default)]
	pub conf: f64,
	/// If true, block until ingest commits; otherwise queue and return the
	/// content-hash doc id immediately — stable and resolvable on a later read.
	#[serde(default)]
	pub sync: bool,
}

impl Default for IngestReq {
	fn default() -> Self {
		Self {
			text: String::new(),
			source: SourceLite::default(),
			kind: EntityKindLite::Claim,
			descriptor: None,
			conf: 0.0,
			sync: false,
		}
	}
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IngestRes {
	/// New entity id; when `sync=false`, the content-hash doc id returned
	/// before the pipeline has committed.
	pub entity_id: String,
	/// One of `"queued" | "ingested" | "duplicate" | "rejected"` —
	/// matches kern's `ingest::outcome::Status::as_str`.
	pub status: String,
	/// Optional pipeline note (rejection reason, dedup pointer, etc.).
	#[serde(default)]
	pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkReq {
	pub from_id: String,
	pub to_id: String,
	/// Mapped server-side to kern's `ReasonKind`; non-1:1 kinds map to the
	/// closest match, with the original kind-name kept in the edge text.
	pub reason_kind: EdgeKind,
	/// Free-text explanation of the relationship.
	#[serde(default)]
	pub text: String,
}

impl Default for LinkReq {
	fn default() -> Self {
		Self {
			from_id: String::new(),
			to_id: String::new(),
			reason_kind: EdgeKind::References,
			text: String::new(),
		}
	}
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LinkRes {
	pub reason_id: String,
}

/// Caller-context snapshot carried into a replicated fork. `byte_range` is
/// `[start, end)` over the underlying source bytes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Anchor {
	pub entity_id: String,
	pub source_uri: String,
	pub byte_range: (u64, u64),
	pub selection: Option<String>,
}

/// Hard-delete an entity by id. The id is matched by prefix server-side
/// (matches the existing kern `tool_forget` semantics).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ForgetReq {
	pub id: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ForgetRes {
	/// True iff an entity with that id (or prefix) was found and removed.
	pub removed: bool,
}

/// Decay confidence on an entity by id (prefix-matched). Mirrors the
/// kern `tool_degrade` MCP path.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DegradeReq {
	pub id: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DegradeRes {
	/// True iff the entity was found and its confidence decayed.
	pub applied: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HealthRes {
	/// True when the daemon is up and the store is loaded.
	pub ok: bool,
	/// Currently-active store data_dir (canonical path string).
	#[serde(default)]
	pub data_dir: String,
	/// Total kerns loaded across all attached stores.
	#[serde(default)]
	pub kerns: u64,
	/// Total entities loaded across all attached stores.
	#[serde(default)]
	pub entities: u64,
}

/// `action` is `"list"` (default), `"add"` (needs name+text), or
/// `"remove"` (needs name).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AnchorReq {
	pub action: String,
	pub name: String,
	pub text: String,
}

/// The anchor tool's JSON result, serialized as a string for transport.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AnchorRes {
	pub result: String,
}

/// One of `"add"` or `"rm"`. Matches the existing kern descriptor CLI.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DescriptorReq {
	pub action: String,
	pub name: String,
	#[serde(default)]
	pub description: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DescriptorRes {}

/// Fire a stigmergic pulse through the root kern.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PulseReq {
	/// Pulse strength. `1.0` is the conventional default.
	#[serde(default = "default_pulse_strength")]
	pub strength: f64,
}

fn default_pulse_strength() -> f64 {
	1.0
}

impl Default for PulseReq {
	fn default() -> Self {
		Self {
			strength: default_pulse_strength(),
		}
	}
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PulseRes {}

/// Generic MCP tool dispatch for the `kern mcp` proxy. `args` is the raw
/// `tools/call.params.arguments` object, forwarded verbatim.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CallToolReq {
	pub name: String,
	#[serde(default)]
	pub args: serde_json::Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CallToolRes {
	/// MCP envelope as emitted by the daemon-side
	/// `mcp::Server::call_tool` — `{ "content": [...], "isError": bool }`.
	pub envelope: serde_json::Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ListToolsReq {}

/// The daemon's live `tools/list`: each entry is a raw MCP tool-schema JSON
/// object exactly as `mcp::Server::tools_list` advertises it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ListToolsRes {
	pub tools: Vec<serde_json::Value>,
}

#[cfg(test)]
mod dto_serde_tests {
	use super::*;

	#[test]
	fn source_lite_roundtrips_through_serde_json() {
		let cases = vec![
			SourceLite::File {
				path: "src/main.rs".into(),
				section: String::new(),
				title: String::new(),
				author: String::new(),
				url: String::new(),
			},
			SourceLite::Ticket {
				system: "github".into(),
				object_id: "T-9".into(),
				section: String::new(),
				title: String::new(),
				author: String::new(),
				url: String::new(),
			},
			SourceLite::Session {
				session_id: "s-1".into(),
				section: String::new(),
				title: String::new(),
			},
			SourceLite::Agent {
				agent: "audit".into(),
				object_id: "o".into(),
				title: String::new(),
			},
			SourceLite::Inline {
				hash: "h".into(),
				section: String::new(),
			},
		];
		for c in cases {
			let s = serde_json::to_string(&c).unwrap();
			let _back: SourceLite = serde_json::from_str(&s).unwrap();
		}
	}

	#[test]
	fn ingest_req_roundtrips_through_bincode() {
		let original = IngestReq {
			text: "hello".into(),
			source: SourceLite::Inline {
				hash: "h".into(),
				section: String::new(),
			},
			kind: EntityKindLite::Claim,
			descriptor: Some("note".into()),
			conf: 0.5,
			sync: true,
		};
		let cfg = bincode::config::standard();
		let bytes = bincode::serde::encode_to_vec(&original, cfg).unwrap();
		let (back, _): (IngestReq, _) = bincode::serde::decode_from_slice(&bytes, cfg).unwrap();
		assert_eq!(back.text, original.text);
		assert_eq!(back.kind, original.kind);
		assert_eq!(back.descriptor.as_deref(), Some("note"));
	}

	#[test]
	fn link_req_roundtrips_through_serde_json() {
		let original = LinkReq {
			from_id: "a".into(),
			to_id: "b".into(),
			reason_kind: EdgeKind::Supports,
			text: "A supports B".into(),
		};
		let s = serde_json::to_string(&original).unwrap();
		let back: LinkReq = serde_json::from_str(&s).unwrap();
		assert_eq!(back.from_id, original.from_id);
		assert_eq!(back.reason_kind, original.reason_kind);
	}

	#[test]
	fn anchor_roundtrips_through_serde_json_and_bincode() {
		let original = Anchor {
			entity_id: "e-1".into(),
			source_uri: "file:///tmp/x.rs".into(),
			byte_range: (10, 42),
			selection: Some("hello".into()),
		};
		let s = serde_json::to_string(&original).unwrap();
		let back: Anchor = serde_json::from_str(&s).unwrap();
		assert_eq!(back, original);

		let cfg = bincode::config::standard();
		let bytes = bincode::serde::encode_to_vec(&original, cfg).unwrap();
		let (back, _): (Anchor, _) = bincode::serde::decode_from_slice(&bytes, cfg).unwrap();
		assert_eq!(back, original);
	}

	#[test]
	fn query_req_default_fresh_is_true() {
		let s = "{\"hits\":[],\"answer\":\"\"}";
		let back: QueryRes = serde_json::from_str(s).unwrap();
		assert!(back.fresh, "missing `fresh` should default to true");
	}

	#[test]
	fn query_req_roundtrips_through_json_and_bincode_with_cancel_token() {
		let original = QueryReq {
			text: "borrow checker".into(),
			k: 7,
			mode: "hybrid".into(),
			answer: true,
			kind: "fact".into(),
			source: "file".into(),
			cancel_token: Some(99),
		};

		let s = serde_json::to_string(&original).unwrap();
		let back: QueryReq = serde_json::from_str(&s).unwrap();
		assert_eq!(back.cancel_token, Some(99));
		assert_eq!(back.k, 7);
		assert!(back.answer);
		assert_eq!(back.kind, "fact");

		let cfg = bincode::config::standard();
		let bytes = bincode::serde::encode_to_vec(&original, cfg).unwrap();
		let (back2, _): (QueryReq, _) = bincode::serde::decode_from_slice(&bytes, cfg).unwrap();
		assert_eq!(back2.cancel_token, Some(99));
		assert_eq!(back2.mode, "hybrid");
		assert_eq!(back2.source, "file");

		let none_req = QueryReq {
			cancel_token: None,
			..Default::default()
		};
		let s = serde_json::to_string(&none_req).unwrap();
		let back3: QueryReq = serde_json::from_str(&s).unwrap();
		assert_eq!(back3.cancel_token, None);
	}
}
