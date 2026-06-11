use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::base::locks::lock_recovered;

pub struct RateClipper {
	state: Mutex<HashMap<String, PeerBucket>>,
	max_per_window: u64,
	window: Duration,
	dropped: AtomicU64,
}

#[derive(Clone, Copy)]
struct PeerBucket {
	count: u64,
	window_start: Instant,
}

impl RateClipper {
	pub fn new(max_per_window: u64, window: Duration) -> Self {
		Self {
			state: Mutex::new(HashMap::new()),
			max_per_window,
			window,
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
		assert!(!rc.admit_at("p", t0), "third call within the window is dropped");
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
		assert!(rc.admit_at("p", t1), "capacity restored after the window elapses");
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
