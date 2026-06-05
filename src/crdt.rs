use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GCounter {
	slots: BTreeMap<String, u64>,
}

impl GCounter {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn increment(&mut self, replica: &str, by: u64) {
		if by == 0 {
			return;
		}
		*self.slots.entry(replica.to_string()).or_insert(0) += by;
	}

	pub fn value(&self) -> u64 {
		self.slots.values().sum()
	}

	pub fn value_i32(&self) -> i32 {
		self.value().min(i32::MAX as u64) as i32
	}

	pub fn merge(&mut self, other: &GCounter) -> bool {
		let mut changed = false;
		for (k, &v) in &other.slots {
			let cur = self.slots.get(k).copied().unwrap_or(0);
			if v > cur {
				self.slots.insert(k.clone(), v);
				changed = true;
			}
		}
		changed
	}

	pub fn slots(&self) -> &BTreeMap<String, u64> {
		&self.slots
	}
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PnCounter {
	pos: GCounter,
	neg: GCounter,
}

impl PnCounter {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn increment(&mut self, replica: &str, by: u64) {
		self.pos.increment(replica, by);
	}

	pub fn decrement(&mut self, replica: &str, by: u64) {
		self.neg.increment(replica, by);
	}

	pub fn value(&self) -> i64 {
		let p = self.pos.value();
		let n = self.neg.value();
		(p as i128 - n as i128).clamp(i64::MIN as i128, i64::MAX as i128) as i64
	}

	pub fn value_i32(&self) -> i32 {
		self.value().clamp(i32::MIN as i64, i32::MAX as i64) as i32
	}

	pub fn merge(&mut self, other: &PnCounter) -> bool {
		let a = self.pos.merge(&other.pos);
		let b = self.neg.merge(&other.neg);
		a || b
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Build a GCounter with a single replica slot set to an absolute value —
	/// the shape an inbound CRDT delta is merged as.
	fn slot(replica: &str, value: u64) -> GCounter {
		let mut g = GCounter::new();
		g.increment(replica, value);
		g
	}

	#[test]
	fn merge_is_per_slot_max() {
		let mut a = slot("r1", 5);
		a.merge(&slot("r1", 3)); // smaller -> no change
		assert_eq!(a.value(), 5);
		a.merge(&slot("r1", 9)); // larger -> wins
		assert_eq!(a.value(), 9);
	}

	#[test]
	fn merge_is_commutative_and_order_independent() {
		// Three absolute-total deltas across two replicas, applied in two
		// different orders (with a duplicate), must converge to the same state.
		let deltas = [slot("r1", 4), slot("r2", 7), slot("r1", 6)];

		let mut a = GCounter::new();
		for d in [&deltas[0], &deltas[1], &deltas[2], &deltas[1]] {
			a.merge(d); // includes a duplicate of r2=7
		}

		let mut b = GCounter::new();
		for d in [&deltas[2], &deltas[1], &deltas[0]] {
			b.merge(d); // reverse order
		}

		assert_eq!(a, b, "merge must be order- and duplicate-independent");
		assert_eq!(a.value(), 6 + 7); // max(r1)=6, r2=7
	}

	#[test]
	fn merge_is_idempotent() {
		let mut a = slot("r1", 5);
		let snapshot = a.clone();
		assert!(!a.merge(&slot("r1", 5)), "re-merging same value is a no-op");
		assert_eq!(a, snapshot);
	}
}
