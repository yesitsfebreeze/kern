//! A binary max-heap over [`HeapItem`]s keyed on `score`, used to drive the beam
//! in retrieval expansion (always pop the highest-scoring frontier node next).
//!
//! Why not `std::collections::BinaryHeap`? The ordering key is an `f64` `score`,
//! which is not `Ord` (NaN), and each item carries a `chain: Vec<String>` payload
//! that must NOT participate in ordering. `BinaryHeap` would force wrapping every
//! item in an `Ord` newtype that compares only `score` through a total-order
//! float shim while ignoring the chain — the newtype dance costs more than this
//! hand-rolled heap, which sifts directly on `score` and stores the payload
//! inline. `score` is assumed finite (cosine / blended retrieval scores).

#[derive(Clone, Debug)]
pub struct HeapItem {
	pub entity_id: String,
	pub score: f64,
	pub chain: Vec<String>,
}

pub struct BeamHeap {
	items: Vec<HeapItem>,
}

impl Default for BeamHeap {
	fn default() -> Self {
		Self::new()
	}
}

impl BeamHeap {
	pub fn new() -> Self {
		Self { items: Vec::new() }
	}

	/// Pre-allocate room for `n` items. Use when the seed/beam width is known up
	/// front to avoid reallocations as the heap fills.
	pub fn with_capacity(n: usize) -> Self {
		Self {
			items: Vec::with_capacity(n),
		}
	}

	pub fn push(&mut self, item: HeapItem) {
		self.items.push(item);
		let mut i = self.items.len() - 1;
		while i > 0 {
			let p = (i - 1) / 2;
			if self.items[i].score <= self.items[p].score {
				break;
			}
			self.items.swap(i, p);
			i = p;
		}
	}

	pub fn pop(&mut self) -> Option<HeapItem> {
		if self.items.is_empty() {
			return None;
		}
		let n = self.items.len() - 1;
		self.items.swap(0, n);
		let top = self.items.pop().unwrap();
		let sz = self.items.len();
		let mut i = 0;
		loop {
			let (l, r) = (2 * i + 1, 2 * i + 2);
			let mut s = i;
			if l < sz && self.items[l].score > self.items[s].score {
				s = l;
			}
			if r < sz && self.items[r].score > self.items[s].score {
				s = r;
			}
			if s == i {
				break;
			}
			self.items.swap(i, s);
			i = s;
		}
		Some(top)
	}

	pub fn len(&self) -> usize {
		self.items.len()
	}

	pub fn is_empty(&self) -> bool {
		self.items.is_empty()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn item(id: &str, score: f64) -> HeapItem {
		HeapItem {
			entity_id: id.into(),
			score,
			chain: vec![id.into()],
		}
	}

	#[test]
	fn pop_from_empty_is_none() {
		let mut h = BeamHeap::new();
		assert!(h.pop().is_none());
		assert!(h.is_empty());
		assert_eq!(h.len(), 0);
	}

	#[test]
	fn push_into_empty_then_pop_returns_it() {
		let mut h = BeamHeap::new();
		h.push(item("a", 0.5));
		assert_eq!(h.len(), 1);
		assert!(!h.is_empty());
		let top = h.pop().expect("one item");
		assert_eq!(top.entity_id, "a");
		assert_eq!(top.score, 0.5);
		assert!(h.is_empty(), "popping the only item empties the heap");
	}

	#[test]
	fn pop_returns_the_max_score_first() {
		let mut h = BeamHeap::new();
		h.push(item("lo", 0.1));
		h.push(item("hi", 0.9));
		h.push(item("mid", 0.5));
		assert_eq!(h.pop().unwrap().entity_id, "hi", "highest score on top");
	}

	#[test]
	fn pop_all_yields_descending_score_order() {
		let mut h = BeamHeap::new();
		// Insert in a deliberately jumbled order.
		for (id, s) in [("c", 0.3), ("a", 0.9), ("e", 0.1), ("b", 0.7), ("d", 0.2)] {
			h.push(item(id, s));
		}
		let mut scores = Vec::new();
		while let Some(it) = h.pop() {
			scores.push(it.score);
		}
		assert_eq!(
			scores,
			vec![0.9, 0.7, 0.3, 0.2, 0.1],
			"max-heap pops in descending score"
		);
		assert!(h.is_empty());
	}

	#[test]
	fn ties_do_not_lose_items() {
		let mut h = BeamHeap::new();
		h.push(item("a", 0.5));
		h.push(item("b", 0.5));
		h.push(item("c", 0.5));
		let mut n = 0;
		while h.pop().is_some() {
			n += 1;
		}
		assert_eq!(n, 3, "equal-score items are all returned");
	}

	#[test]
	fn with_capacity_behaves_like_new() {
		let mut h = BeamHeap::with_capacity(8);
		assert!(h.is_empty());
		h.push(item("a", 0.4));
		h.push(item("b", 0.8));
		assert_eq!(
			h.pop().unwrap().entity_id,
			"b",
			"preallocation doesn't change ordering"
		);
		assert_eq!(h.pop().unwrap().entity_id, "a");
	}
}
