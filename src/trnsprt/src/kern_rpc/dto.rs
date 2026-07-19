use serde::{Deserialize, Serialize};

pub use crate::search::dto::{
	EdgeKind, EntityKindLite, EntityRef, EntityStatusLite, NeighborsReq, NeighborsRes,
};

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
	pub k: u32,
	#[serde(default)]
	pub mode: String,
	#[serde(default)]
	pub answer: bool,
	#[serde(default)]
	pub kind: String,
	#[serde(default)]
	pub source: String,
	#[serde(default)]
	pub cancel_token: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QueryRes {
	pub hits: Vec<EntityRef>,
	#[serde(default)]
	pub answer: String,
	#[serde(default = "default_true")]
	pub fresh: bool,
}

// Missing `fresh` on the wire means "not stale" — bool's derived Default (false) would invert that.
fn default_true() -> bool {
	true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IngestReq {
	pub text: String,
	pub source: SourceLite,
	pub kind: EntityKindLite,
	#[serde(default)]
	pub descriptor: Option<String>,
	#[serde(default)]
	pub conf: f64,
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
	pub entity_id: String,
	pub status: String,
	#[serde(default)]
	pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkReq {
	pub from_id: String,
	pub to_id: String,
	pub reason_kind: EdgeKind,
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

// `byte_range` is `[start, end)` over the underlying source bytes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Graviton {
	pub entity_id: String,
	pub source_uri: String,
	pub byte_range: (u64, u64),
	pub selection: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ForgetReq {
	pub id: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ForgetRes {
	pub removed: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DegradeReq {
	pub id: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DegradeRes {
	pub applied: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HealthRes {
	pub ok: bool,
	#[serde(default)]
	pub data_dir: String,
	#[serde(default)]
	pub kerns: u64,
	#[serde(default)]
	pub entities: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GravitonReq {
	pub action: String,
	pub name: String,
	pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GravitonRes {
	pub result: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DescriptorReq {
	pub action: String,
	pub name: String,
	#[serde(default)]
	pub description: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DescriptorRes {}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PulseReq {
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CallToolReq {
	pub name: String,
	#[serde(default)]
	pub args: serde_json::Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CallToolRes {
	pub envelope: serde_json::Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ListToolsReq {}

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
	fn graviton_roundtrips_through_serde_json_and_bincode() {
		let original = Graviton {
			entity_id: "e-1".into(),
			source_uri: "file:///tmp/x.rs".into(),
			byte_range: (10, 42),
			selection: Some("hello".into()),
		};
		let s = serde_json::to_string(&original).unwrap();
		let back: Graviton = serde_json::from_str(&s).unwrap();
		assert_eq!(back, original);

		let cfg = bincode::config::standard();
		let bytes = bincode::serde::encode_to_vec(&original, cfg).unwrap();
		let (back, _): (Graviton, _) = bincode::serde::decode_from_slice(&bytes, cfg).unwrap();
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
