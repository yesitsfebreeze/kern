//! Journal entry and its event-kind taxonomy.
//!
//! [`Kind`] tags every [`Entry`]. Most variants are unit-like — their detail
//! lives in `Entry.payload` as JSON — but five carry their data INLINE in the
//! variant itself: `Edit { target_ts_ms, new_text }`, `Fork { from_ts_ms,
//! new_fork_id }`, and the fork-lifecycle trio `ForkOpen { fork_id, parent }` /
//! `ForkResume { fork_id }` / `ForkClose { fork_id }`. Inline data lets a
//! consumer replay a journal slice without a second lookup into `payload`.
//!
//! That inline data has a maintenance consequence: the SQLite history stores
//! `kind` as a short text tag, not the full serde enum, so `history.rs`'s
//! `kind_tag` / `kind_from_tag` round-trip those inline fields by hand. Any
//! change to an inline-data variant's shape must be mirrored there. The serde
//! round-trip is covered by the tests in this file; the tag round-trip lives in
//! `history.rs`.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SCHEMA_VERSION: u32 = 5;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
	// Turn-level (agnt-originating).
	User,
	Assistant,
	Final,
	TurnStart,
	TurnEnd,
	Usage,
	ToolCall,
	RecipeInvoke,
	PluginCall,
	Error,
	Ask,
	Answer,
	Goal,
	GoalSnapshot,
	Milestone,
	// Adjust mode: rewrite of a prior message and conversation forks
	// taken from a chosen point in history. Carry payload data inline
	// so consumers can replay journal slices without joining `payload`.
	Edit { target_ts_ms: u64, new_text: String },
	Fork { from_ts_ms: u64, new_fork_id: String },
	// Cross-process plumbing. Emitted by the tarpc client/server middleware
	// and ad-hoc by any binary that wants to surface activity in the
	// shared journal.
	RpcSend,
	RpcRecv,
	RpcError,
	Log,
	// Slice G — rolling plan. `PlanStep` is the canonical step record; the
	// orchestrator emits these as it builds the plan. `PlanProposal` is a
	// sub-agent suggestion awaiting orchestrator review. Payloads carry the
	// `PlanStepPayload` / `PlanProposalPayload` structs from `events.rs`.
	PlanStep,
	PlanProposal,
	// Slice I — recents MRU. Emitted whenever a user/agent action touches
	// an entity (`Open|Drill|Mention|AgentRead|AgentWrite|FsWrite`). The
	// a client replays these on cold start to seed its in-memory MRU
	// ring. Payload schema lives in `events::EntityTouchedPayload`.
	EntityTouched,
	// Slice K — agnt fork lifecycle. `ForkOpen` marks fork creation (with
	// optional parent fork id), `ForkResume` marks rehydration after pause,
	// `ForkClose` marks termination. The kern `SessionMirror` tails these
	// to ingest each fork as a `Document` entity with
	// `source = Source::Session { session_id = fork_id }` so sessions are
	// searchable via the `:session` facet. Distinct
	// from the existing `Fork` variant (which records a branch-from-history
	// point during `Edit`/adjust-mode rewrites and carries a `from_ts_ms`).
	ForkOpen { fork_id: String, parent: Option<String> },
	ForkResume { fork_id: String },
	ForkClose { fork_id: String },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Entry {
	pub v: u32,
	pub ts_ms: u64,
	pub kind: Kind,
	pub key: String,
	pub payload: serde_json::Value,
}

impl Entry {
	pub fn new(kind: Kind, key: impl Into<String>, payload: serde_json::Value) -> Self {
		Self {
			v: SCHEMA_VERSION,
			ts_ms: now_ms(),
			kind,
			key: key.into(),
			payload,
		}
	}
}

pub trait Sink: Send + Sync {
	fn emit(&self, entry: Entry);
}

pub struct NullSink;

impl Sink for NullSink {
	fn emit(&self, _entry: Entry) {}
}

pub fn now_ms() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|d| d.as_millis() as u64)
		.unwrap_or(0)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Serialize an `Entry` carrying `kind`, deserialize it back, and return the
	/// recovered kind after asserting the envelope fields survived intact.
	fn roundtrip_kind(kind: Kind) -> Kind {
		let entry = Entry::new(kind, "k", serde_json::json!({ "x": 1 }));
		let bytes = serde_json::to_vec(&entry).expect("serialize");
		let back: Entry = serde_json::from_slice(&bytes).expect("deserialize");
		assert_eq!(back.v, SCHEMA_VERSION);
		assert_eq!(back.key, "k");
		assert_eq!(back.payload, serde_json::json!({ "x": 1 }));
		back.kind
	}

	#[test]
	fn entry_round_trips_inline_data_variants() {
		// The inline-data variants carry their fields in the enum, so a serde
		// regression (e.g. a renamed field) would silently corrupt replay.
		assert_eq!(
			roundtrip_kind(Kind::Edit { target_ts_ms: 42, new_text: "fixed".into() }),
			Kind::Edit { target_ts_ms: 42, new_text: "fixed".into() }
		);
		assert_eq!(
			roundtrip_kind(Kind::Fork { from_ts_ms: 7, new_fork_id: "nf".into() }),
			Kind::Fork { from_ts_ms: 7, new_fork_id: "nf".into() }
		);
		assert_eq!(
			roundtrip_kind(Kind::ForkOpen { fork_id: "f1".into(), parent: Some("p".into()) }),
			Kind::ForkOpen { fork_id: "f1".into(), parent: Some("p".into()) }
		);
		// `parent: None` is a distinct serde shape worth covering too.
		assert_eq!(
			roundtrip_kind(Kind::ForkOpen { fork_id: "f2".into(), parent: None }),
			Kind::ForkOpen { fork_id: "f2".into(), parent: None }
		);
	}

	#[test]
	fn entry_round_trips_a_unit_variant() {
		assert_eq!(roundtrip_kind(Kind::TurnStart), Kind::TurnStart);
	}
}
