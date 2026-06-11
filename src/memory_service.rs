//! `MemoryService` — HashMap-backed in-memory store used by Adjust
//! mode's `truncate_after` flow. Intentionally a HashMap shim — the
//! truncate-by-timestamp semantics don't need the full graph.
//!
//! HashMap (not BTreeMap) is deliberate: every access is a point operation —
//! upsert and lookup by `key`, plus `truncate_after`'s O(n) full `retain` scan —
//! and nothing here needs ordered iteration or range queries, so HashMap's O(1)
//! ops are the right fit; a BTreeMap's key ordering would be unused overhead.
//! A future maintainer should not "tidy" this into a BTreeMap without a reason.

use crate::base::locks::lock_recovered;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct MemoryEntry {
	pub ts_ms: u64,
	pub key: String,
	pub text: String,
}

#[derive(Default)]
pub struct MemoryService {
	entries: Mutex<HashMap<String, MemoryEntry>>,
}

impl MemoryService {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn insert(&self, e: MemoryEntry) {
		let mut g = lock_recovered(&self.entries);
		g.insert(e.key.clone(), e);
	}

	/// Drop entries with `ts_ms > input`. Returns the number removed so
	/// callers can surface a trace line for visibility.
	pub fn truncate_after(&self, ts_ms: u64) -> usize {
		let mut g = lock_recovered(&self.entries);
		let before = g.len();
		g.retain(|_, e| e.ts_ms <= ts_ms);
		before - g.len()
	}

	pub fn len(&self) -> usize {
		lock_recovered(&self.entries).len()
	}

	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}

	/// Owned snapshot of every entry, taken under a single lock — for debugging
	/// and inspection without lending out the guard or re-locking per element.
	/// Order is unspecified (HashMap iteration order).
	pub fn snapshot(&self) -> Vec<MemoryEntry> {
		lock_recovered(&self.entries).values().cloned().collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn truncate_drops_newer_only() {
		let s = MemoryService::new();
		s.insert(MemoryEntry { ts_ms: 10, key: "a".into(), text: "x".into() });
		s.insert(MemoryEntry { ts_ms: 20, key: "b".into(), text: "y".into() });
		s.insert(MemoryEntry { ts_ms: 30, key: "c".into(), text: "z".into() });
		assert_eq!(s.truncate_after(20), 1);
		assert_eq!(s.len(), 2);
	}

	#[test]
	fn insert_same_key_upserts_in_place() {
		// HashMap upsert is intentional: re-inserting a key REPLACES, never appends.
		let s = MemoryService::new();
		s.insert(MemoryEntry { ts_ms: 10, key: "k".into(), text: "old".into() });
		s.insert(MemoryEntry { ts_ms: 20, key: "k".into(), text: "new".into() });

		assert_eq!(s.len(), 1, "same key overwrites rather than duplicating");
		let snap = s.snapshot();
		assert_eq!(snap.len(), 1);
		assert_eq!(snap[0].text, "new", "later insert wins");
		assert_eq!(snap[0].ts_ms, 20);
	}

	#[test]
	fn snapshot_returns_every_entry() {
		let s = MemoryService::new();
		s.insert(MemoryEntry { ts_ms: 1, key: "a".into(), text: "x".into() });
		s.insert(MemoryEntry { ts_ms: 2, key: "b".into(), text: "y".into() });
		let mut keys: Vec<String> = s.snapshot().into_iter().map(|e| e.key).collect();
		keys.sort();
		assert_eq!(keys, vec!["a".to_string(), "b".to_string()]);
	}
}
