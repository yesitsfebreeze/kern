use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LwwRegister<T: Clone + Default + PartialEq> {
	value: T,
	lamport: u64,
	producer: String,
}

impl<T: Clone + Default + PartialEq> LwwRegister<T> {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn set(&mut self, value: T, lamport: u64, producer: &str) {
		self.value = value;
		self.lamport = lamport;
		self.producer = producer.to_string();
	}

	pub fn value(&self) -> &T {
		&self.value
	}

	pub fn lamport(&self) -> u64 {
		self.lamport
	}

	pub fn producer(&self) -> &str {
		&self.producer
	}

	pub fn merge(&mut self, other: &Self) -> bool {
		if (other.lamport, &other.producer) > (self.lamport, &self.producer) {
			self.value = other.value.clone();
			self.lamport = other.lamport;
			self.producer = other.producer.clone();
			true
		} else {
			false
		}
	}
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrSet<T: Ord + Clone + Default> {
	adds: BTreeMap<T, BTreeSet<(String, u64)>>,
	tombstones: BTreeSet<(T, String, u64)>,
}

impl<T: Ord + Clone + Default> OrSet<T> {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn add(&mut self, value: T, replica: &str, lamport: u64) {
		let tag = (replica.to_string(), lamport);
		if !self
			.tombstones
			.contains(&(value.clone(), tag.0.clone(), tag.1))
		{
			self.adds.entry(value).or_default().insert(tag);
		}
	}

	pub fn remove(&mut self, value: &T) {
		if let Some(tags) = self.adds.get(value) {
			for (replica, lamport) in tags {
				self
					.tombstones
					.insert((value.clone(), replica.clone(), *lamport));
			}
		}
		self.adds.remove(value);
	}

	pub fn contains(&self, value: &T) -> bool {
		self.adds.get(value).is_some_and(|tags| {
			tags.iter().any(|tag| {
				!self
					.tombstones
					.contains(&(value.clone(), tag.0.clone(), tag.1))
			})
		})
	}

	pub fn values(&self) -> Vec<T> {
		self
			.adds
			.keys()
			.filter(|v| self.contains(v))
			.cloned()
			.collect()
	}

	pub fn merge(&mut self, other: &Self) -> bool {
		let mut changed = false;
		for (value, tags) in &other.adds {
			for tag in tags {
				if !self
					.tombstones
					.contains(&(value.clone(), tag.0.clone(), tag.1))
					&& self
						.adds
						.entry(value.clone())
						.or_default()
						.insert(tag.clone())
				{
					changed = true;
				}
			}
		}
		let tomb_before = self.tombstones.len();
		self.tombstones.extend(other.tombstones.iter().cloned());
		if self.tombstones.len() != tomb_before {
			changed = true;
		}
		changed
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn slot(replica: &str, value: u64) -> GCounter {
		let mut g = GCounter::new();
		g.increment(replica, value);
		g
	}

	#[test]
	fn merge_is_per_slot_max() {
		let mut a = slot("r1", 5);
		a.merge(&slot("r1", 3));
		assert_eq!(a.value(), 5);
		a.merge(&slot("r1", 9));
		assert_eq!(a.value(), 9);
	}

	#[test]
	fn merge_is_commutative_and_order_independent() {
		let deltas = [slot("r1", 4), slot("r2", 7), slot("r1", 6)];

		let mut a = GCounter::new();
		for d in [&deltas[0], &deltas[1], &deltas[2], &deltas[1]] {
			a.merge(d);
		}

		let mut b = GCounter::new();
		for d in [&deltas[2], &deltas[1], &deltas[0]] {
			b.merge(d);
		}

		assert_eq!(a, b, "merge must be order- and duplicate-independent");
		assert_eq!(a.value(), 6 + 7);
	}

	#[test]
	fn merge_is_idempotent() {
		let mut a = slot("r1", 5);
		let snapshot = a.clone();
		assert!(!a.merge(&slot("r1", 5)), "re-merging same value is a no-op");
		assert_eq!(a, snapshot);
	}

	#[test]
	fn lww_merge_takes_higher_lamport() {
		let mut a = LwwRegister::<f64>::new();
		a.set(0.3, 1, "r1");
		let mut b = LwwRegister::<f64>::new();
		b.set(0.7, 2, "r2");
		assert!(a.merge(&b));
		assert_eq!(*a.value(), 0.7);
		assert_eq!(a.lamport(), 2);
	}

	#[test]
	fn lww_merge_ties_break_by_producer() {
		let mut a = LwwRegister::<f64>::new();
		a.set(0.3, 5, "r1");
		let mut b = LwwRegister::<f64>::new();
		b.set(0.7, 5, "r2");
		assert!(a.merge(&b));
		assert_eq!(
			*a.value(),
			0.7,
			"same lamport, lexicographically higher producer wins"
		);
	}

	#[test]
	fn lww_merge_is_idempotent() {
		let mut a = LwwRegister::<f64>::new();
		a.set(0.5, 3, "r1");
		let snap = a.clone();
		assert!(!a.merge(&snap));
		assert_eq!(a, snap);
	}

	#[test]
	fn lww_default_state_loses_to_any_write() {
		let mut a = LwwRegister::<f64>::new();
		let mut b = LwwRegister::<f64>::new();
		b.set(0.9, 1, "r1");
		assert!(a.merge(&b));
		assert_eq!(*a.value(), 0.9);
	}

	#[test]
	fn orset_add_and_contains() {
		let mut s = OrSet::<String>::new();
		s.add("hello".into(), "r1", 1);
		s.add("world".into(), "r2", 1);
		assert!(s.contains(&"hello".into()));
		assert!(s.contains(&"world".into()));
		assert!(!s.contains(&"missing".into()));
	}

	#[test]
	fn orset_merge_unions_adds() {
		let mut a = OrSet::<String>::new();
		a.add("x".into(), "r1", 1);
		let mut b = OrSet::<String>::new();
		b.add("y".into(), "r2", 1);
		assert!(a.merge(&b));
		assert!(a.contains(&"x".into()));
		assert!(a.contains(&"y".into()));
	}

	#[test]
	fn orset_merge_is_idempotent() {
		let mut a = OrSet::<String>::new();
		a.add("x".into(), "r1", 1);
		let snap = a.clone();
		assert!(!a.merge(&snap));
		assert_eq!(a, snap);
	}

	#[test]
	fn orset_remove_then_re_add_from_other_replica_survives() {
		let mut a = OrSet::<String>::new();
		a.add("x".into(), "r1", 1);
		a.remove(&"x".into());
		assert!(!a.contains(&"x".into()));
		let mut b = OrSet::<String>::new();
		b.add("x".into(), "r2", 1);
		assert!(a.merge(&b));
		assert!(
			a.contains(&"x".into()),
			"re-add from another replica survives the tombstone"
		);
	}

	#[test]
	fn orset_concurrent_adds_converge() {
		let mut a = OrSet::<String>::new();
		a.add("x".into(), "r1", 1);
		a.add("y".into(), "r1", 2);
		let mut b = OrSet::<String>::new();
		b.add("y".into(), "r2", 1);
		b.add("z".into(), "r2", 2);
		a.merge(&b);
		b.merge(&a.clone());
		assert_eq!(a, b, "concurrent adds converge to the same state");
		let mut vals = a.values();
		vals.sort();
		assert_eq!(
			vals,
			vec!["x".to_string(), "y".to_string(), "z".to_string()]
		);
	}

	#[test]
	fn orset_remove_tombstones_propagate_via_merge() {
		let mut a = OrSet::<String>::new();
		a.add("x".into(), "r1", 1);
		a.remove(&"x".into());
		let mut b = OrSet::<String>::new();
		b.add("x".into(), "r1", 1);
		assert!(b.merge(&a));
		assert!(
			!b.contains(&"x".into()),
			"tombstone propagated via merge removes the element"
		);
	}
}
