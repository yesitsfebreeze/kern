use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;
use std::time::Instant;

use crate::base::constants::{LEDGER_ROUTING_TTL, LEDGER_THOUGHT_TTL};
use crate::base::locks::{read_recovered, write_recovered};

const DEFAULT_LEDGER_CAP: usize = 10_000;

struct Entry {
	addr: String,
	expires: Instant,
}

/// A capped routing table with O(log n) soonest-expiry eviction. `map` is the
/// lookup index (key → entry); `by_expiry` mirrors it keyed by `(expiry, key)`
/// so the soonest-expiring entry is `by_expiry`'s first element — found and
/// removed in O(log n) instead of the old O(n) min-scan. The `(Instant, String)`
/// key makes eviction order total and deterministic even when two entries share
/// an `Instant`. INVARIANT: every mutation touches both structures in lockstep,
/// so `by_expiry` never holds a stale `(expiry, key)` for a removed/overwritten
/// entry.
#[derive(Default)]
struct Index {
	map: HashMap<String, Entry>,
	by_expiry: BTreeMap<(Instant, String), ()>,
}

impl Index {
	/// Insert or overwrite `key`, first evicting soonest-expiring entries so the
	/// map stays within `cap`. Overwriting drops the prior expiry-index entry
	/// first, so it neither goes stale nor spuriously evicts another entry.
	fn insert(&mut self, key: String, addr: String, expires: Instant, cap: usize) {
		if let Some(old) = self.map.remove(&key) {
			self.by_expiry.remove(&(old.expires, key.clone()));
		}
		while self.map.len() >= cap {
			let Some((oldest_exp, oldest_key)) = self.by_expiry.keys().next().cloned() else {
				break;
			};
			self.by_expiry.remove(&(oldest_exp, oldest_key.clone()));
			self.map.remove(&oldest_key);
		}
		self.by_expiry.insert((expires, key.clone()), ());
		self.map.insert(key, Entry { addr, expires });
	}

	/// Address for `key` iff present and not past its TTL as of `now`.
	fn lookup(&self, key: &str, now: Instant) -> Option<String> {
		self.map.get(key).and_then(|e| live_addr(e, now))
	}
}

/// TTL'd routing hints learned from gossip: "which peer can serve X". Two
/// independent maps, each capped and evicted by soonest-expiry:
/// - `entities`: thought id -> peer address (where to fetch a specific thought),
///   TTL [`LEDGER_THOUGHT_TTL`].
/// - `routing`: kern id -> peer address (which peer owns/serves a kern's sphere),
///   TTL [`LEDGER_ROUTING_TTL`].
///
/// All entries are advisory and expire: a [`lookup_thought`](Ledger::lookup_thought)
/// or [`lookup_routing`](Ledger::lookup_routing) past the TTL returns `None` (the
/// stale entry is simply ignored, swept lazily on the next capacity eviction).
pub struct Ledger {
	entities: RwLock<Index>,
	routing: RwLock<Index>,
	max_entries: AtomicUsize,
}

impl Ledger {
	pub fn new() -> Self {
		Self {
			entities: RwLock::new(Index::default()),
			routing: RwLock::new(Index::default()),
			max_entries: AtomicUsize::new(DEFAULT_LEDGER_CAP),
		}
	}

	pub fn set_max_entries(&self, cap: usize) {
		self.max_entries.store(cap.max(1), Ordering::Relaxed);
	}

	fn cap(&self) -> usize {
		self.max_entries.load(Ordering::Relaxed)
	}

	/// Record that thought `id` can be fetched from `addr`, valid for
	/// [`LEDGER_THOUGHT_TTL`]. May evict the soonest-expiring entry if at capacity.
	pub fn put_thought(&self, id: &str, addr: &str) {
		let expires = Instant::now() + LEDGER_THOUGHT_TTL;
		write_recovered(&self.entities).insert(id.to_string(), addr.to_string(), expires, self.cap());
	}

	/// Record that kern `kern_id` is served by `addr`, valid for
	/// [`LEDGER_ROUTING_TTL`]. May evict the soonest-expiring entry if at capacity.
	pub fn put_routing(&self, kern_id: &str, addr: &str) {
		let expires = Instant::now() + LEDGER_ROUTING_TTL;
		write_recovered(&self.routing).insert(
			kern_id.to_string(),
			addr.to_string(),
			expires,
			self.cap(),
		);
	}

	/// Peer address for thought `id`, or `None` if unknown or past its TTL.
	pub fn lookup_thought(&self, id: &str) -> Option<String> {
		read_recovered(&self.entities).lookup(id, Instant::now())
	}

	/// Peer address serving kern `kern_id`, or `None` if unknown or past its TTL.
	pub fn lookup_routing(&self, kern_id: &str) -> Option<String> {
		read_recovered(&self.routing).lookup(kern_id, Instant::now())
	}
}

/// The shared liveness check behind both lookups: return the entry's address iff
/// it has not yet expired as of `now`. Pulled out as a free fn so the TTL
/// semantics are unit-testable without waiting on a real wall-clock TTL.
fn live_addr(e: &Entry, now: Instant) -> Option<String> {
	(e.expires > now).then(|| e.addr.clone())
}

impl Default for Ledger {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::time::Duration;

	#[test]
	fn routing_put_then_lookup_round_trips() {
		let l = Ledger::new();
		assert_eq!(l.lookup_routing("k1"), None, "unknown kern -> None");
		l.put_routing("k1", "127.0.0.1:7400");
		assert_eq!(l.lookup_routing("k1"), Some("127.0.0.1:7400".to_string()));
	}

	#[test]
	fn thought_put_then_lookup_round_trips() {
		let l = Ledger::new();
		l.put_thought("t1", "127.0.0.1:7401");
		assert_eq!(l.lookup_thought("t1"), Some("127.0.0.1:7401".to_string()));
		assert_eq!(l.lookup_thought("missing"), None);
	}

	#[test]
	fn live_addr_returns_addr_before_expiry_and_none_after() {
		let base = Instant::now();
		let e = Entry {
			addr: "peer".into(),
			expires: base + Duration::from_secs(10),
		};
		// Querying before the deadline yields the address.
		assert_eq!(live_addr(&e, base), Some("peer".to_string()));
		// Querying past the deadline yields None.
		assert_eq!(live_addr(&e, base + Duration::from_secs(20)), None);
	}

	#[test]
	fn eviction_holds_map_at_capacity() {
		let l = Ledger::new();
		l.set_max_entries(2);
		l.put_routing("a", "1");
		l.put_routing("b", "2");
		l.put_routing("c", "3"); // at cap -> evicts the soonest-expiring of a/b

		let live = ["a", "b", "c"]
			.iter()
			.filter(|k| l.lookup_routing(k).is_some())
			.count();
		assert_eq!(live, 2, "cap=2 holds at most two entries");
		// "a" was inserted first, so it has the soonest expiry (and, on an equal
		// Instant, the lexicographically smallest key) — it is the one evicted.
		assert_eq!(
			l.lookup_routing("a"),
			None,
			"soonest-expiring entry is evicted"
		);
		assert_eq!(l.lookup_routing("b"), Some("2".to_string()));
		assert_eq!(l.lookup_routing("c"), Some("3".to_string()));
	}

	#[test]
	fn overwriting_a_key_does_not_evict_another_entry() {
		// Regression: the old evict-then-insert path ran eviction while at cap even
		// when the put was an overwrite, so re-putting "a" could evict "b". The
		// expiry-index path removes the prior "a" first, so "b" survives.
		let l = Ledger::new();
		l.set_max_entries(2);
		l.put_routing("a", "1");
		l.put_routing("b", "2");
		l.put_routing("a", "1b"); // overwrite — must not displace "b"
		assert_eq!(
			l.lookup_routing("b"),
			Some("2".to_string()),
			"overwrite must not evict b"
		);
		assert_eq!(
			l.lookup_routing("a"),
			Some("1b".to_string()),
			"a updated in place"
		);
	}

	#[test]
	fn set_max_entries_floors_at_one() {
		let l = Ledger::new();
		l.set_max_entries(0);
		assert_eq!(l.cap(), 1, "cap is floored at 1 so inserts can still land");
	}
}
