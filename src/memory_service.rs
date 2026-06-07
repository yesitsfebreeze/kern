//! `MemoryService` — HashMap-backed in-memory store used by Adjust
//! mode's `truncate_after` flow. Intentionally a HashMap shim — the
//! truncate-by-timestamp semantics don't need the full graph.

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
}
