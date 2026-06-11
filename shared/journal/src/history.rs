use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use time::{OffsetDateTime, UtcOffset};

use crate::entry::{Entry, Kind, SCHEMA_VERSION};

#[derive(Debug, Clone, Default)]
pub struct Filter {
	pub kind: Option<Kind>,
	pub key: Option<String>,
	pub since_ms: Option<u64>,
	pub until_ms: Option<u64>,
	pub limit: Option<u64>,
}

impl Filter {
	pub fn all() -> Self {
		Self::default()
}

	pub fn kind(k: Kind) -> Self {
		Self {
			kind: Some(k),
			..Self::default()
		}
}
}

pub struct History {
	conn: Mutex<Connection>,
}

impl History {
	pub fn open(project_root: &Path) -> rusqlite::Result<Self> {
		let dir = project_root.join(".kern").join("journal");
		if let Err(e) = std::fs::create_dir_all(&dir) {
			return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(e)));
		}
		let path = dir.join("history.db");
		let conn = Connection::open(&path)?;
		Self::init(&conn)?;
		Ok(Self {
			conn: Mutex::new(conn),
		})
}

	pub fn open_in_memory() -> rusqlite::Result<Self> {
		let conn = Connection::open_in_memory()?;
		Self::init(&conn)?;
		Ok(Self {
			conn: Mutex::new(conn),
		})
}

	fn init(conn: &Connection) -> rusqlite::Result<()> {
		conn.pragma_update(None, "journal_mode", "WAL").ok();
		conn.pragma_update(None, "synchronous", "NORMAL").ok();
		conn.execute_batch(
			"CREATE TABLE IF NOT EXISTS entries (
				id       INTEGER PRIMARY KEY AUTOINCREMENT,
				ts_ms    INTEGER NOT NULL,
				day      TEXT    NOT NULL,
				kind     TEXT    NOT NULL,
				key      TEXT    NOT NULL,
				payload  TEXT    NOT NULL
			);
			CREATE INDEX IF NOT EXISTS idx_day_kind ON entries(day, kind);
			CREATE INDEX IF NOT EXISTS idx_kind_key ON entries(kind, key);
			CREATE INDEX IF NOT EXISTS idx_ts ON entries(ts_ms);",
		)
}

	pub fn bulk_insert(&self, entries: &[Entry]) -> rusqlite::Result<()> {
		if entries.is_empty() {
			return Ok(());
		}
		let mut conn = self.conn.lock().expect("history mutex poisoned");
		let tx = conn.transaction()?;
		{
			let mut stmt = tx.prepare(
				"INSERT INTO entries (ts_ms, day, kind, key, payload) VALUES (?, ?, ?, ?, ?)",
			)?;
			for e in entries {
				let kind_s = kind_tag(&e.kind);
				let day = day_for(e.ts_ms);
				let payload = serde_json::to_string(&e.payload)
					.map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
				stmt.execute(params![e.ts_ms as i64, day, kind_s, e.key, payload])?;
			}
		}
		tx.commit()
	}

	pub fn query(&self, filter: Filter) -> rusqlite::Result<Vec<Entry>> {
		let conn = self.conn.lock().expect("history mutex poisoned");

		let mut sql = String::from("SELECT ts_ms, kind, key, payload FROM entries");
		let mut clauses: Vec<&'static str> = Vec::new();
		let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

		if let Some(k) = filter.kind.as_ref() {
			clauses.push("kind = ?");
			args.push(Box::new(kind_tag(k).to_string()));
		}
		if let Some(key) = filter.key.as_ref() {
			clauses.push("key = ?");
			args.push(Box::new(key.clone()));
		}
		if let Some(since) = filter.since_ms {
			clauses.push("ts_ms >= ?");
			args.push(Box::new(since as i64));
		}
		if let Some(until) = filter.until_ms {
			clauses.push("ts_ms < ?");
			args.push(Box::new(until as i64));
		}
		if !clauses.is_empty() {
			sql.push_str(" WHERE ");
			sql.push_str(&clauses.join(" AND "));
		}
		sql.push_str(" ORDER BY ts_ms ASC");
		if let Some(limit) = filter.limit {
			sql.push_str(&format!(" LIMIT {limit}"));
		}

		let mut stmt = conn.prepare(&sql)?;
		let rows = stmt.query_map(params_from_iter(args.iter().map(|b| b.as_ref())), |row| {
			let ts_ms: i64 = row.get(0)?;
			let kind_s: String = row.get(1)?;
			let key: String = row.get(2)?;
			let payload_s: String = row.get(3)?;
			let payload: serde_json::Value = serde_json::from_str(&payload_s).map_err(|e| {
				rusqlite::Error::FromSqlConversionFailure(
					3,
					rusqlite::types::Type::Text,
					Box::new(e),
				)
			})?;
			let kind = kind_from_tag(&kind_s, &payload).ok_or_else(|| {
				rusqlite::Error::FromSqlConversionFailure(
					1,
					rusqlite::types::Type::Text,
					format!("unknown kind: {kind_s}").into(),
				)
			})?;
			Ok(Entry {
				v: SCHEMA_VERSION,
				ts_ms: ts_ms as u64,
				kind,
				key,
				payload,
			})
		})?;

		let mut out = Vec::new();
		for r in rows {
			out.push(r?);
		}
		Ok(out)
}

	pub fn count_by_key(
		&self,
		kind: &Kind,
		since_ms: Option<u64>,
	) -> rusqlite::Result<Vec<(String, u64)>> {
		let conn = self.conn.lock().expect("history mutex poisoned");
		// Single prepared statement with an optional `ts_ms >= ?` clause, mirroring
		// the dynamic-WHERE pattern in `query()` (was two near-identical branches).
		let mut sql = String::from("SELECT key, COUNT(*) FROM entries WHERE kind = ?");
		let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(kind_tag(kind).to_string())];
		if let Some(since) = since_ms {
			sql.push_str(" AND ts_ms >= ?");
			args.push(Box::new(since as i64));
		}
		sql.push_str(" GROUP BY key ORDER BY 2 DESC");

		let mut stmt = conn.prepare(&sql)?;
		let rows: rusqlite::Result<Vec<(String, i64)>> = stmt
			.query_map(params_from_iter(args.iter().map(|b| b.as_ref())), |r| {
				Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
			})?
			.collect();
		Ok(rows?.into_iter().map(|(k, n)| (k, n as u64)).collect())
}

	pub fn prune_before(&self, day: &str) -> rusqlite::Result<usize> {
		let conn = self.conn.lock().expect("history mutex poisoned");
		let n = conn.execute("DELETE FROM entries WHERE day < ?", params![day])?;
		if n > 1000 {
			let _ = conn.execute_batch("VACUUM");
		}
		Ok(n)
}

	pub fn retain_days(&self, retain_days: u32) -> rusqlite::Result<usize> {
		let now_ms = crate::entry::now_ms();
		let cutoff_ms = now_ms.saturating_sub((retain_days as u64) * 86_400_000);
		let cutoff_day = day_for(cutoff_ms);
		self.prune_before(&cutoff_day)
}

	pub fn len(&self) -> rusqlite::Result<u64> {
		let conn = self.conn.lock().expect("history mutex poisoned");
		let n: i64 = conn
			.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
			.optional()?
			.unwrap_or(0);
		Ok(n as u64)
}

	/// Whether the history holds no entries.
	pub fn is_empty(&self) -> rusqlite::Result<bool> {
		Ok(self.len()? == 0)
	}
}

fn kind_tag(k: &Kind) -> &'static str {
	match k {
		Kind::User => "user",
		Kind::Assistant => "assistant",
		Kind::Final => "final",
		Kind::TurnStart => "turn_start",
		Kind::TurnEnd => "turn_end",
		Kind::Usage => "usage",
		Kind::ToolCall => "tool_call",
		Kind::RecipeInvoke => "recipe_invoke",
		Kind::PluginCall => "plugin_call",
		Kind::Error => "error",
		Kind::Ask => "ask",
		Kind::Answer => "answer",
		Kind::Goal => "goal",
		Kind::GoalSnapshot => "goal_snapshot",
		Kind::Milestone => "milestone",
		Kind::Edit { .. } => "edit",
		Kind::Fork { .. } => "fork",
		Kind::RpcSend => "rpc_send",
		Kind::RpcRecv => "rpc_recv",
		Kind::RpcError => "rpc_error",
		Kind::Log => "log",
		Kind::PlanStep => "plan_step",
		Kind::PlanProposal => "plan_proposal",
		Kind::EntityTouched => "entity_touched",
		Kind::ForkOpen { .. } => "fork_open",
		Kind::ForkResume { .. } => "fork_resume",
		Kind::ForkClose { .. } => "fork_close",
	}
}

fn kind_from_tag(s: &str, payload: &serde_json::Value) -> Option<Kind> {
	Some(match s {
		"user" => Kind::User,
		"assistant" => Kind::Assistant,
		"final" => Kind::Final,
		"turn_start" => Kind::TurnStart,
		"turn_end" => Kind::TurnEnd,
		"usage" => Kind::Usage,
		"tool_call" => Kind::ToolCall,
		"recipe_invoke" => Kind::RecipeInvoke,
		"plugin_call" => Kind::PluginCall,
		"error" => Kind::Error,
		"ask" => Kind::Ask,
		"answer" => Kind::Answer,
		"goal" => Kind::Goal,
		"goal_snapshot" => Kind::GoalSnapshot,
		"milestone" => Kind::Milestone,
		// Sqlite stores Edit/Fork data in the `payload` column rather than
		// the variant tag, so the row reader pulls the inner fields back
		// out at decode time. Missing fields surface as zero/empty so the
		// query still returns rather than fail-hard on legacy rows.
		"edit" => Kind::Edit {
			target_ts_ms: payload.get("target_ts_ms").and_then(|v| v.as_u64()).unwrap_or(0),
			new_text: payload.get("new_text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
		},
		"fork" => Kind::Fork {
			from_ts_ms: payload.get("from_ts_ms").and_then(|v| v.as_u64()).unwrap_or(0),
			new_fork_id: payload.get("new_fork_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
		},
		"rpc_send" => Kind::RpcSend,
		"rpc_recv" => Kind::RpcRecv,
		"rpc_error" => Kind::RpcError,
		"log" => Kind::Log,
		"plan_step" => Kind::PlanStep,
		"plan_proposal" => Kind::PlanProposal,
		"entity_touched" => Kind::EntityTouched,
		// Fork lifecycle: like Edit/Fork above, fork_id and optional parent
		// live in the payload column. Default to empty strings / None when a
		// row predates the schema bump so old journals still replay.
		"fork_open" => Kind::ForkOpen {
			fork_id: payload.get("fork_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
			parent: payload
				.get("parent")
				.and_then(|v| v.as_str())
				.map(|s| s.to_string()),
		},
		"fork_resume" => Kind::ForkResume {
			fork_id: payload.get("fork_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
		},
		"fork_close" => Kind::ForkClose {
			fork_id: payload.get("fork_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
		},
		_ => return None,
	})
}

fn day_for(ts_ms: u64) -> String {
	let nanos = (ts_ms as i128) * 1_000_000;
	let Ok(utc) = OffsetDateTime::from_unix_timestamp_nanos(nanos) else {
		return "1970-01-01".to_string();
	};
	let dt = match UtcOffset::current_local_offset() {
		Ok(off) => utc.to_offset(off),
		Err(_) => utc,
	};
	dt.date().to_string()
}

impl crate::day_journal::HistorySink for History {
	fn bulk_insert(
		&self,
		entries: &[Entry],
	) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
		History::bulk_insert(self, entries).map_err(|e| Box::new(e) as _)
}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn entry(ts_ms: u64, kind: Kind, key: &str, payload: serde_json::Value) -> Entry {
		Entry { v: SCHEMA_VERSION, ts_ms, kind, key: key.into(), payload }
	}

	#[test]
	fn bulk_insert_then_query_round_trips_ordered_by_time() {
		let h = History::open_in_memory().unwrap();
		assert!(h.is_empty().unwrap());
		h.bulk_insert(&[
			entry(200, Kind::Assistant, "b", serde_json::json!({"t": "yo"})),
			entry(100, Kind::User, "a", serde_json::json!({"t": "hi"})),
		])
		.unwrap();
		assert_eq!(h.len().unwrap(), 2);
		let all = h.query(Filter::all()).unwrap();
		assert_eq!(all.len(), 2);
		assert_eq!(all[0].ts_ms, 100, "ORDER BY ts_ms ASC");
		assert_eq!(all[1].key, "b");
	}

	#[test]
	fn query_filters_by_kind_and_since_window() {
		let h = History::open_in_memory().unwrap();
		h.bulk_insert(&[
			entry(100, Kind::User, "a", serde_json::json!({})),
			entry(300, Kind::User, "b", serde_json::json!({})),
			entry(200, Kind::Assistant, "c", serde_json::json!({})),
		])
		.unwrap();
		assert_eq!(h.query(Filter::kind(Kind::User)).unwrap().len(), 2);
		let mut f = Filter::kind(Kind::User);
		f.since_ms = Some(150);
		let recent = h.query(f).unwrap();
		assert_eq!(recent.len(), 1);
		assert_eq!(recent[0].key, "b");
	}

	#[test]
	fn count_by_key_groups_descending_and_respects_since() {
		let h = History::open_in_memory().unwrap();
		h.bulk_insert(&[
			entry(100, Kind::User, "a", serde_json::json!({})),
			entry(150, Kind::User, "a", serde_json::json!({})),
			entry(300, Kind::User, "b", serde_json::json!({})),
		])
		.unwrap();
		let counts = h.count_by_key(&Kind::User, None).unwrap();
		assert_eq!(counts[0], ("a".to_string(), 2), "highest count first");
		assert_eq!(counts[1], ("b".to_string(), 1));
		// since filter drops the two ts=100/150 'a' rows.
		assert_eq!(h.count_by_key(&Kind::User, Some(200)).unwrap(), vec![("b".to_string(), 1)]);
	}

	#[test]
	fn payload_carrying_kinds_round_trip_through_decode() {
		let h = History::open_in_memory().unwrap();
		h.bulk_insert(&[
			entry(
				100,
				Kind::Edit { target_ts_ms: 42, new_text: "x".into() },
				"e",
				serde_json::json!({"target_ts_ms": 42, "new_text": "x"}),
			),
			entry(
				200,
				Kind::ForkOpen { fork_id: "f1".into(), parent: Some("p".into()) },
				"f",
				serde_json::json!({"fork_id": "f1", "parent": "p"}),
			),
		])
		.unwrap();
		let all = h.query(Filter::all()).unwrap();
		assert!(matches!(&all[0].kind, Kind::Edit { target_ts_ms: 42, new_text } if new_text == "x"));
		assert!(matches!(&all[1].kind, Kind::ForkOpen { fork_id, parent: Some(p) } if fork_id == "f1" && p == "p"));
	}

	/// A payload carrying the inner fields the decoder needs for payload-bearing
	/// kinds (Edit/Fork/Fork*); other kinds ignore it.
	fn payload_for(k: &Kind) -> serde_json::Value {
		match k {
			Kind::Edit { target_ts_ms, new_text } => {
				serde_json::json!({ "target_ts_ms": target_ts_ms, "new_text": new_text })
			}
			Kind::Fork { from_ts_ms, new_fork_id } => {
				serde_json::json!({ "from_ts_ms": from_ts_ms, "new_fork_id": new_fork_id })
			}
			Kind::ForkOpen { fork_id, parent } => {
				serde_json::json!({ "fork_id": fork_id, "parent": parent })
			}
			Kind::ForkResume { fork_id } => serde_json::json!({ "fork_id": fork_id }),
			Kind::ForkClose { fork_id } => serde_json::json!({ "fork_id": fork_id }),
			_ => serde_json::json!({}),
		}
	}

	#[test]
	fn every_kind_tag_round_trips_through_kind_from_tag() {
		// Exhaustiveness guard: this match has NO `_` arm, so adding a new Kind
		// variant fails to compile here until it's acknowledged — which forces it
		// into `all` below and thus through the round-trip check (catching a tag
		// added to kind_tag but missing its kind_from_tag decode arm).
		fn _exhaustive(k: &Kind) {
			match k {
				Kind::User | Kind::Assistant | Kind::Final | Kind::TurnStart | Kind::TurnEnd
				| Kind::Usage | Kind::ToolCall | Kind::RecipeInvoke | Kind::PluginCall | Kind::Error
				| Kind::Ask | Kind::Answer | Kind::Goal | Kind::GoalSnapshot | Kind::Milestone
				| Kind::Edit { .. } | Kind::Fork { .. } | Kind::RpcSend | Kind::RpcRecv
				| Kind::RpcError | Kind::Log | Kind::PlanStep | Kind::PlanProposal
				| Kind::EntityTouched | Kind::ForkOpen { .. } | Kind::ForkResume { .. }
				| Kind::ForkClose { .. } => {}
			}
		}

		let all: Vec<Kind> = vec![
			Kind::User,
			Kind::Assistant,
			Kind::Final,
			Kind::TurnStart,
			Kind::TurnEnd,
			Kind::Usage,
			Kind::ToolCall,
			Kind::RecipeInvoke,
			Kind::PluginCall,
			Kind::Error,
			Kind::Ask,
			Kind::Answer,
			Kind::Goal,
			Kind::GoalSnapshot,
			Kind::Milestone,
			Kind::Edit { target_ts_ms: 1, new_text: "t".into() },
			Kind::Fork { from_ts_ms: 2, new_fork_id: "f".into() },
			Kind::RpcSend,
			Kind::RpcRecv,
			Kind::RpcError,
			Kind::Log,
			Kind::PlanStep,
			Kind::PlanProposal,
			Kind::EntityTouched,
			Kind::ForkOpen { fork_id: "fo".into(), parent: Some("p".into()) },
			Kind::ForkResume { fork_id: "fr".into() },
			Kind::ForkClose { fork_id: "fc".into() },
		];
		assert_eq!(all.len(), 27, "list every Kind variant exactly once (see _exhaustive)");

		for k in &all {
			let tag = kind_tag(k);
			let decoded = kind_from_tag(tag, &payload_for(k)).unwrap_or_else(|| {
				panic!("kind_from_tag has no arm for tag {tag:?} — kind_tag and kind_from_tag drifted")
			});
			// No PartialEq on Kind needed: re-tag the decoded value and require the
			// tag to be stable, proving the decoder maps the tag back to its variant.
			assert_eq!(kind_tag(&decoded), tag, "tag not stable across decode for {tag:?}");
		}
	}

	#[test]
	fn prune_before_deletes_older_days_only() {
		let h = History::open_in_memory().unwrap();
		h.bulk_insert(&[
			entry(0, Kind::User, "old", serde_json::json!({})), // 1970-01-01
			entry(crate::entry::now_ms(), Kind::User, "new", serde_json::json!({})),
		])
		.unwrap();
		let removed = h.prune_before("2000-01-01").unwrap();
		assert_eq!(removed, 1, "only the 1970 row predates 2000-01-01");
		let remaining = h.query(Filter::all()).unwrap();
		assert_eq!(remaining.len(), 1);
		assert_eq!(remaining[0].key, "new");
	}
}
