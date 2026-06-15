use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::base::locks::lock_recovered;

pub struct RateClipper {
	state: Mutex<HashMap<String, PeerBucket>>,
	max_per_window: u64,
	window: Duration,
	/// Soft cap on tracked peer buckets; stale buckets are swept once reached.
	peer_cap: usize,
	dropped: AtomicU64,
}

#[derive(Clone, Copy)]
struct PeerBucket {
	count: u64,
	window_start: Instant,
}

impl RateClipper {
	pub fn new(max_per_window: u64, window: Duration) -> Self {
		Self::with_peer_cap(
			max_per_window,
			window,
			crate::base::constants::GOSSIP_SYBIL_PEER_CAP,
		)
	}

	fn with_peer_cap(max_per_window: u64, window: Duration, peer_cap: usize) -> Self {
		Self {
			state: Mutex::new(HashMap::new()),
			max_per_window,
			window,
			peer_cap,
			dropped: AtomicU64::new(0),
		}
	}

	pub fn admit(&self, peer: &str) -> bool {
		self.admit_at(peer, Instant::now())
	}

	pub fn admit_at(&self, peer: &str, now: Instant) -> bool {
		if self.max_per_window == 0 {
			return true;
		}
		let mut state = lock_recovered(&self.state);
		// Bound memory against a sybil flood of distinct forged peer ids: the map
		// keys on the caller-supplied peer id and would otherwise grow forever.
		// A bucket whose window has fully elapsed holds no live rate-limit state —
		// the next admit for that peer resets it regardless — so evicting stale
		// buckets is semantically free (no change to the limiting decision). Sweep
		// only when the map crosses the cap, so the common path stays O(1). Live
		// buckets are never evicted, so an attacker under active limiting cannot
		// force their own bucket to reset by flooding new ids.
		if state.len() >= self.peer_cap {
			state.retain(|_, b| now.duration_since(b.window_start) < self.window);
		}
		let bucket = state.entry(peer.to_string()).or_insert(PeerBucket {
			count: 0,
			window_start: now,
		});
		if now.duration_since(bucket.window_start) >= self.window {
			bucket.count = 0;
			bucket.window_start = now;
		}
		if bucket.count >= self.max_per_window {
			self.dropped.fetch_add(1, Ordering::Relaxed);
			return false;
		}
		bucket.count += 1;
		true
	}

	pub fn dropped_count(&self) -> u64 {
		self.dropped.load(Ordering::Relaxed)
	}

	#[cfg(test)]
	fn bucket_count(&self) -> usize {
		self.state.lock().unwrap().len()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn max_per_window_zero_admits_everything() {
		let rc = RateClipper::new(0, Duration::from_secs(1));
		for _ in 0..100 {
			assert!(rc.admit("p"));
		}
		assert_eq!(rc.dropped_count(), 0, "the zero-cap fast path never drops");
	}

	#[test]
	fn admits_up_to_cap_then_drops_within_window() {
		let rc = RateClipper::new(2, Duration::from_secs(10));
		let t0 = Instant::now();
		assert!(rc.admit_at("p", t0));
		assert!(rc.admit_at("p", t0));
		assert!(
			!rc.admit_at("p", t0),
			"third call within the window is dropped"
		);
		assert_eq!(rc.dropped_count(), 1);
	}

	#[test]
	fn capacity_is_restored_after_the_window_elapses() {
		let rc = RateClipper::new(1, Duration::from_secs(5));
		let t0 = Instant::now();
		assert!(rc.admit_at("p", t0));
		assert!(!rc.admit_at("p", t0), "over cap in the same window");
		// Advance past the window: the bucket resets.
		let t1 = t0 + Duration::from_secs(6);
		assert!(
			rc.admit_at("p", t1),
			"capacity restored after the window elapses"
		);
	}

	#[test]
	fn stale_buckets_are_reclaimed_when_peer_cap_is_reached() {
		// Memory bound: once the bucket map hits the cap, buckets whose window has
		// fully elapsed are reclaimed (they hold no live state). Two peers go stale,
		// then a third distinct peer triggers the sweep.
		let rc = RateClipper::with_peer_cap(5, Duration::from_secs(5), 2);
		let t0 = Instant::now();
		assert!(rc.admit_at("a", t0));
		assert!(rc.admit_at("b", t0));
		assert_eq!(rc.bucket_count(), 2, "two buckets tracked");
		// Past the window: a and b are stale. A new peer at cap triggers retain.
		let t1 = t0 + Duration::from_secs(6);
		assert!(rc.admit_at("c", t1));
		assert_eq!(
			rc.bucket_count(),
			1,
			"stale a,b reclaimed; only live c remains"
		);
	}

	#[test]
	fn live_buckets_survive_the_sweep_so_limiting_is_not_reset_by_a_flood() {
		// Security: a live bucket must NOT be evicted by the cap sweep, else an
		// attacker could reset their own limiter by flooding new peer ids.
		let rc = RateClipper::with_peer_cap(1, Duration::from_secs(10), 2);
		let t0 = Instant::now();
		assert!(rc.admit_at("a", t0));
		assert!(
			!rc.admit_at("a", t0),
			"a is over its cap (live limiter engaged)"
		);
		assert!(rc.admit_at("b", t0)); // map now at cap, both a and b live
		assert!(rc.admit_at("c", t0)); // triggers the sweep; a,b are live -> kept
		assert!(
			!rc.admit_at("a", t0),
			"a's live limiter survived the flood sweep and still rejects"
		);
	}

	#[test]
	fn buckets_are_independent_per_peer() {
		let rc = RateClipper::new(1, Duration::from_secs(10));
		let t0 = Instant::now();
		assert!(rc.admit_at("a", t0));
		assert!(rc.admit_at("b", t0), "a different peer has its own bucket");
		assert!(!rc.admit_at("a", t0), "peer a is now over its cap");
	}
}
