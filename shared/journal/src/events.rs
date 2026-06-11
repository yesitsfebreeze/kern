//! Typed payload structs for the rolling-plan journal events introduced
//! in slice G of the orchestrator TUI plan. Each struct round-trips through
//! `serde_json::Value` on `Entry::payload`, so callers stay free to read
//! the raw JSON if they don't want to depend on these types.

use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Lifecycle of a plan step. `Pending` is the default for newly-emitted
/// steps that haven't been picked up yet; `Active` is in-progress;
/// `Done` and `Blocked` are terminal.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
	#[default]
	Pending,
	Active,
	Done,
	Blocked,
}

/// JSON-serialised payload for `Kind::PlanStep`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PlanStepPayload {
	pub id: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub parent: Option<String>,
	pub status: PlanStatus,
	pub body: String,
	/// Wall-clock timestamp in milliseconds since the unix epoch. Mirrors
	/// `Entry.ts_ms` so consumers that ingest the payload directly (without
	/// the surrounding `Entry`) keep ordering information.
	pub ts_ms: u64,
}

/// JSON-serialised payload for `Kind::PlanProposal`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PlanProposalPayload {
	pub id: String,
	pub body: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub source_fork: Option<String>,
	pub ts_ms: u64,
}

/// Rolling-plan event surfaced to the TUI plan model. `From<&Entry>` parses
/// the `Entry.payload` JSON; consumers that already have a typed payload
/// can construct the variant directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanEvent {
	Step {
		id: String,
		parent: Option<String>,
		status: PlanStatus,
		body: String,
		ts: SystemTime,
	},
	Proposal {
		id: String,
		body: String,
		source_fork: Option<String>,
		ts: SystemTime,
	},
}

/// JSON-serialised payload for `Kind::TurnStart`. `turn_id` is additive — older
/// entries that pre-date the field deserialise with an empty string so replay
/// stays safe without bumping `SCHEMA_VERSION`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TurnStartPayload {
	#[serde(default)]
	pub turn_id: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub fork_id: Option<String>,
	pub ts_ms: u64,
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub phase: String,
}

/// JSON-serialised payload for `Kind::TurnEnd`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TurnEndPayload {
	#[serde(default)]
	pub turn_id: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub fork_id: Option<String>,
	pub ts_ms: u64,
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub outcome: String,
}

/// JSON-serialised payload for `Kind::Final`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FinalPayload {
	#[serde(default)]
	pub turn_id: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub fork_id: Option<String>,
	pub ts_ms: u64,
	#[serde(default)]
	pub text: String,
}

/// JSON-serialised payload for `Kind::ToolCall`. `phase` is `"start"` or `"end"`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ToolCallPayload {
	#[serde(default)]
	pub turn_id: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub fork_id: Option<String>,
	pub ts_ms: u64,
	pub name: String,
	#[serde(default)]
	pub args_json: serde_json::Value,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub result: Option<String>,
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub phase: String,
}

/// Generate the `from_entry` parser for a single-payload event view: gate on
/// the journal [`Kind`](super::Kind), deserialize `entry.payload` into the
/// payload struct, then map its fields into the typed event. The turn/tool
/// event views are structurally identical bar that field mapping, so it stays
/// explicit in the closure while the gate + deserialize boilerplate lives once.
macro_rules! impl_from_entry {
	($event:ty, $payload:ty, $kind:pat, |$p:ident| $build:expr) => {
		impl $event {
			pub fn from_entry(entry: &super::Entry) -> Option<Self> {
				if !matches!(entry.kind, $kind) {
					return None;
				}
				let $p: $payload = serde_json::from_value(entry.payload.clone()).ok()?;
				Some($build)
			}
		}
	};
}

/// Typed view of a `turn_start` journal entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnStartEvent {
	pub turn_id: String,
	pub fork_id: Option<String>,
	pub phase: String,
	pub ts: SystemTime,
}

impl_from_entry!(TurnStartEvent, TurnStartPayload, super::Kind::TurnStart, |p| Self {
	turn_id: p.turn_id,
	fork_id: p.fork_id,
	phase: p.phase,
	ts: system_time_from_ms(p.ts_ms),
});

/// Typed view of a `turn_end` journal entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnEndEvent {
	pub turn_id: String,
	pub fork_id: Option<String>,
	pub outcome: String,
	pub ts: SystemTime,
}

impl_from_entry!(TurnEndEvent, TurnEndPayload, super::Kind::TurnEnd, |p| Self {
	turn_id: p.turn_id,
	fork_id: p.fork_id,
	outcome: p.outcome,
	ts: system_time_from_ms(p.ts_ms),
});

/// Typed view of a `final` journal entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalEvent {
	pub turn_id: String,
	pub fork_id: Option<String>,
	pub text: String,
	pub ts: SystemTime,
}

impl_from_entry!(FinalEvent, FinalPayload, super::Kind::Final, |p| Self {
	turn_id: p.turn_id,
	fork_id: p.fork_id,
	text: p.text,
	ts: system_time_from_ms(p.ts_ms),
});

/// Typed view of a `tool_call` journal entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallEvent {
	pub turn_id: String,
	pub fork_id: Option<String>,
	pub name: String,
	pub args_json: serde_json::Value,
	pub result: Option<String>,
	pub phase: String,
	pub ts: SystemTime,
}

impl_from_entry!(ToolCallEvent, ToolCallPayload, super::Kind::ToolCall, |p| Self {
	turn_id: p.turn_id,
	fork_id: p.fork_id,
	name: p.name,
	args_json: p.args_json,
	result: p.result,
	phase: p.phase,
	ts: system_time_from_ms(p.ts_ms),
});

/// Discrete touch op recorded against an entity (slice I). A client
/// uses these to seed its MRU recents ring; the same enum is logged into
/// the shared journal so cross-process activity surfaces in the ring after
/// replay.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TouchOp {
	Open,
	Drill,
	Mention,
	AgentRead,
	AgentWrite,
	FsWrite,
}

/// JSON-serialised payload for `Kind::EntityTouched`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct EntityTouchedPayload {
	pub entity_id: String,
	pub op: TouchOp,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub fork_id: Option<String>,
	pub ts_ms: u64,
}

/// Typed view of an `entity_touched` journal entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityTouchedEvent {
	pub entity_id: String,
	pub op: TouchOp,
	pub fork_id: Option<String>,
	pub ts: SystemTime,
}

impl EntityTouchedEvent {
	/// Try to parse a journal `Entry` into an `EntityTouchedEvent`. Returns
	/// `None` for non-touch kinds or when the payload is malformed.
	pub fn from_entry(entry: &super::Entry) -> Option<Self> {
		if !matches!(entry.kind, super::Kind::EntityTouched) {
			return None;
		}
		let p: EntityTouchedPayload = serde_json::from_value(entry.payload.clone()).ok()?;
		Some(Self {
			entity_id: p.entity_id,
			op: p.op,
			fork_id: p.fork_id,
			ts: system_time_from_ms(p.ts_ms),
		})
	}
}

/// Convert a unix-millisecond stamp back to `SystemTime` without panicking
/// when the value is zero or in the future. Saturates instead of wrapping.
pub fn system_time_from_ms(ts_ms: u64) -> SystemTime {
	UNIX_EPOCH
		.checked_add(Duration::from_millis(ts_ms))
		.unwrap_or(UNIX_EPOCH)
}

impl PlanEvent {
	/// Try to parse a journal `Entry` into a `PlanEvent`. Returns `None`
	/// for non-plan kinds or when the payload doesn't match the expected
	/// schema.
	pub fn from_entry(entry: &super::Entry) -> Option<PlanEvent> {
		match entry.kind {
			super::Kind::PlanStep => {
				let p: PlanStepPayload = serde_json::from_value(entry.payload.clone()).ok()?;
				Some(PlanEvent::Step {
					id: p.id,
					parent: p.parent,
					status: p.status,
					body: p.body,
					ts: system_time_from_ms(p.ts_ms),
				})
			}
			super::Kind::PlanProposal => {
				let p: PlanProposalPayload = serde_json::from_value(entry.payload.clone()).ok()?;
				Some(PlanEvent::Proposal {
					id: p.id,
					body: p.body,
					source_fork: p.source_fork,
					ts: system_time_from_ms(p.ts_ms),
				})
			}
			_ => None,
		}
	}

	pub fn ts(&self) -> SystemTime {
		match self {
			PlanEvent::Step { ts, .. } | PlanEvent::Proposal { ts, .. } => *ts,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Entry, Kind};

	#[test]
	fn roundtrip_plan_step() {
		let p = PlanStepPayload {
			id: "s1".into(),
			parent: None,
			status: PlanStatus::Active,
			body: "audit token expiry".into(),
			ts_ms: 1234,
		};
		let v = serde_json::to_value(&p).unwrap();
		let entry = Entry::new(Kind::PlanStep, "plan", v);
		let ev = PlanEvent::from_entry(&entry).unwrap();
		match ev {
			PlanEvent::Step { id, status, body, .. } => {
				assert_eq!(id, "s1");
				assert_eq!(status, PlanStatus::Active);
				assert_eq!(body, "audit token expiry");
			}
			_ => panic!("wrong variant"),
		}
	}

	#[test]
	fn roundtrip_plan_proposal() {
		let p = PlanProposalPayload {
			id: "p1".into(),
			body: "swap < for <=".into(),
			source_fork: Some("audit".into()),
			ts_ms: 5,
		};
		let v = serde_json::to_value(&p).unwrap();
		let entry = Entry::new(Kind::PlanProposal, "plan", v);
		let ev = PlanEvent::from_entry(&entry).unwrap();
		match ev {
			PlanEvent::Proposal { id, source_fork, .. } => {
				assert_eq!(id, "p1");
				assert_eq!(source_fork.as_deref(), Some("audit"));
			}
			_ => panic!("wrong variant"),
		}
	}

	#[test]
	fn roundtrip_entity_touched() {
		let p = EntityTouchedPayload {
			entity_id: "f1".into(),
			op: TouchOp::Drill,
			fork_id: Some("audit".into()),
			ts_ms: 99,
		};
		let v = serde_json::to_value(&p).unwrap();
		let entry = Entry::new(Kind::EntityTouched, "f1", v);
		let ev = EntityTouchedEvent::from_entry(&entry).expect("parses");
		assert_eq!(ev.entity_id, "f1");
		assert_eq!(ev.op, TouchOp::Drill);
		assert_eq!(ev.fork_id.as_deref(), Some("audit"));
	}

	#[test]
	fn entity_touched_rejects_non_touch_entry() {
		let entry = Entry::new(Kind::Log, "x", serde_json::Value::Null);
		assert!(EntityTouchedEvent::from_entry(&entry).is_none());
	}

	#[test]
	fn roundtrip_turn_start() {
		let p = TurnStartPayload {
			turn_id: "t1".into(),
			fork_id: Some("audit".into()),
			ts_ms: 10,
			phase: "begin".into(),
		};
		let v = serde_json::to_value(&p).unwrap();
		let entry = Entry::new(Kind::TurnStart, "t", v);
		let ev = TurnStartEvent::from_entry(&entry).expect("parses");
		assert_eq!(ev.turn_id, "t1");
		assert_eq!(ev.fork_id.as_deref(), Some("audit"));
		assert_eq!(ev.phase, "begin");
	}

	#[test]
	fn roundtrip_turn_end() {
		let p = TurnEndPayload {
			turn_id: "t2".into(),
			fork_id: None,
			ts_ms: 20,
			outcome: "ok".into(),
		};
		let v = serde_json::to_value(&p).unwrap();
		let entry = Entry::new(Kind::TurnEnd, "t", v);
		let ev = TurnEndEvent::from_entry(&entry).expect("parses");
		assert_eq!(ev.turn_id, "t2");
		assert_eq!(ev.outcome, "ok");
	}

	#[test]
	fn roundtrip_final() {
		let p = FinalPayload {
			turn_id: "t3".into(),
			fork_id: None,
			ts_ms: 30,
			text: "done".into(),
		};
		let v = serde_json::to_value(&p).unwrap();
		let entry = Entry::new(Kind::Final, "t", v);
		let ev = FinalEvent::from_entry(&entry).expect("parses");
		assert_eq!(ev.turn_id, "t3");
		assert_eq!(ev.text, "done");
	}

	#[test]
	fn roundtrip_tool_call() {
		let p = ToolCallPayload {
			turn_id: "t4".into(),
			fork_id: Some("audit".into()),
			ts_ms: 40,
			name: "read".into(),
			args_json: serde_json::json!({"path": "/x"}),
			result: Some("ok".into()),
			phase: "end".into(),
		};
		let v = serde_json::to_value(&p).unwrap();
		let entry = Entry::new(Kind::ToolCall, "t", v);
		let ev = ToolCallEvent::from_entry(&entry).expect("parses");
		assert_eq!(ev.turn_id, "t4");
		assert_eq!(ev.name, "read");
		assert_eq!(ev.phase, "end");
		assert_eq!(ev.result.as_deref(), Some("ok"));
	}

	#[test]
	fn turn_start_default_turn_id_when_missing() {
		// Backward-replay: payload from before the field existed.
		let v = serde_json::json!({"ts_ms": 1});
		let entry = Entry::new(Kind::TurnStart, "t", v);
		let ev = TurnStartEvent::from_entry(&entry).expect("parses");
		assert_eq!(ev.turn_id, "");
	}

	#[test]
	fn non_plan_entry_returns_none() {
		let entry = Entry::new(Kind::Log, "k", serde_json::Value::Null);
		assert!(PlanEvent::from_entry(&entry).is_none());
	}

	#[test]
	fn event_parsers_reject_fork_lifecycle_kinds() {
		// Fork lifecycle kinds carry their data in the `Kind` itself (entry.rs)
		// and have no payload-struct event view here, so every turn/tool parser
		// must decline them rather than mis-claim a fork entry. (The Kind tag
		// round-trip for these variants is covered in history.rs.)
		for kind in [
			Kind::ForkOpen { fork_id: "f".into(), parent: Some("p".into()) },
			Kind::ForkResume { fork_id: "f".into() },
			Kind::ForkClose { fork_id: "f".into() },
		] {
			let entry = Entry::new(kind, "f", serde_json::Value::Null);
			assert!(TurnStartEvent::from_entry(&entry).is_none());
			assert!(TurnEndEvent::from_entry(&entry).is_none());
			assert!(FinalEvent::from_entry(&entry).is_none());
			assert!(ToolCallEvent::from_entry(&entry).is_none());
			assert!(EntityTouchedEvent::from_entry(&entry).is_none());
		}
	}
}
