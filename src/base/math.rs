use super::constants::*;
use super::types::{Kern, ReasonKind, EntityKind};
use super::util;

pub fn cosine(a: &[f64], b: &[f64]) -> f64 {
	#[cfg(target_arch = "x86_64")]
	{
		if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") {
			return unsafe { cosine_avx2(a, b) };
		}
	}
	cosine_scalar(a, b)
}

fn cosine_scalar(a: &[f64], b: &[f64]) -> f64 {
	let (mut dot, mut na, mut nb) = (0.0, 0.0, 0.0);
	for (ai, bi) in a.iter().zip(b.iter()) {
		dot += ai * bi;
		na += ai * ai;
		nb += bi * bi;
	}
	if na == 0.0 || nb == 0.0 {
		return 0.0;
	}
	dot / (na.sqrt() * nb.sqrt())
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn cosine_avx2(a: &[f64], b: &[f64]) -> f64 {
	use std::arch::x86_64::*;

	let n = a.len().min(b.len());
	let chunks = n / 4;
	let rem = n % 4;

	let mut vdot = _mm256_setzero_pd();
	let mut vna = _mm256_setzero_pd();
	let mut vnb = _mm256_setzero_pd();

	let pa = a.as_ptr();
	let pb = b.as_ptr();

	for i in 0..chunks {
		let off = i * 4;
		let va = _mm256_loadu_pd(pa.add(off));
		let vb = _mm256_loadu_pd(pb.add(off));
		vdot = _mm256_fmadd_pd(va, vb, vdot);
		vna = _mm256_fmadd_pd(va, va, vna);
		vnb = _mm256_fmadd_pd(vb, vb, vnb);
	}

	let mut dot = hsum_256_pd(vdot);
	let mut na = hsum_256_pd(vna);
	let mut nb = hsum_256_pd(vnb);

	let tail = chunks * 4;
	for i in 0..rem {
		let ai = *a.get_unchecked(tail + i);
		let bi = *b.get_unchecked(tail + i);
		dot += ai * bi;
		na += ai * ai;
		nb += bi * bi;
	}

	if na == 0.0 || nb == 0.0 {
		return 0.0;
	}
	dot / (na.sqrt() * nb.sqrt())
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn hsum_256_pd(v: std::arch::x86_64::__m256d) -> f64 {
	use std::arch::x86_64::*;
	let high = _mm256_extractf128_pd(v, 1);
	let low = _mm256_castpd256_pd128(v);
	let sum128 = _mm_add_pd(low, high);
	let hi64 = _mm_unpackhi_pd(sum128, sum128);
	let total = _mm_add_sd(sum128, hi64);
	_mm_cvtsd_f64(total)
}

pub fn cosine_distance(a: &[f64], b: &[f64]) -> f64 {
	1.0 - cosine(a, b)
}

pub fn average_vec(a: &[f64], b: &[f64]) -> Vec<f64> {
	a.iter()
		.zip(b.iter())
		.map(|(ai, bi)| (ai + bi) / 2.0)
		.collect()
}

pub fn reason_id(from: &str, to: &str, kind: ReasonKind, text: &str, to_net_id: &str) -> String {
	util::content_hash(&format!(
		"{}\x00{}\x00{}\x00{}\x00{}",
		from, to, kind as i32, text, to_net_id
	))
}

pub fn adjacent_reasons(kern: &Kern, reason_id: &str) -> Vec<String> {
	let r = match kern.reasons.get(reason_id) {
		Some(r) => r,
		None => return Vec::new(),
	};
	let mut seen = std::collections::HashSet::new();
	let mut out = Vec::new();
	for tid in [&r.from, &r.to] {
		if tid.is_empty() {
			continue;
		}
		for rids in [kern.by_from.get(tid.as_str()), kern.by_to.get(tid.as_str())]
			.into_iter()
			.flatten()
		{
			for rid in rids {
				if rid != reason_id && seen.insert(rid.clone()) {
					out.push(rid.clone());
				}
			}
		}
	}
	out
}

#[derive(Debug, Clone, Copy)]
pub struct OnlineSoftmax {
	m: f64,
	s: f64,
}

impl Default for OnlineSoftmax {
	fn default() -> Self {
		Self::new()
	}
}

impl OnlineSoftmax {
	pub fn new() -> Self {
		Self {
			m: f64::NEG_INFINITY,
			s: 0.0,
		}
	}

	pub fn update(&mut self, x: f64) {
		if !x.is_finite() {
			return;
		}
		let m_new = self.m.max(x);
		let carry = if self.m.is_finite() {
			self.s * (self.m - m_new).exp()
		} else {
			0.0
		};
		self.s = carry + (x - m_new).exp();
		self.m = m_new;
	}

	pub fn is_empty(&self) -> bool {
		self.s == 0.0 && !self.m.is_finite()
	}

	pub fn running_max(&self) -> f64 {
		self.m
	}

	/// Log-sum-exp of all observed scores: `m + ln(s)`.
	///
	/// This is deliberately a *pooling* operator, not a max. A single
	/// observation is the identity (`x + ln(1) = x`), but an item observed `k`
	/// times at the same score `x` finalizes to `x + ln(k)` — a corroboration
	/// boost rewarding entities surfaced via multiple retrieval paths (e.g. both
	/// the seed list and the beam in `retrieval::merge`). The result is a
	/// relevance magnitude, not a probability, so values above 1.0 are expected
	/// and fine; downstream only ranks by it and applies a multiplicative
	/// confidence plus additive boosts. Use [`running_max`](Self::running_max)
	/// instead when best-score-wins (no corroboration) is wanted.
	pub fn finalize(&self) -> f64 {
		if self.is_empty() {
			return f64::NEG_INFINITY;
		}
		self.m + self.s.ln()
	}
}

pub fn softmax_merge_scores<I, K>(iter: I) -> std::collections::HashMap<K, f64>
where
	I: IntoIterator<Item = (K, f64)>,
	K: std::hash::Hash + Eq,
{
	let mut acc: std::collections::HashMap<K, OnlineSoftmax> = std::collections::HashMap::new();
	for (k, v) in iter {
		acc.entry(k).or_default().update(v);
	}
	acc.into_iter().map(|(k, s)| (k, s.finalize())).collect()
}

pub fn clamp_confidence(conf: f64, source: &str) -> (f64, EntityKind) {
	let mut conf = if conf <= 0.0 {
		DEFAULT_CONFIDENCE
	} else {
		conf
	};
	if conf < 0.01 {
		conf = 0.01;
	}
	if source != USER_SOURCE && conf > MAX_AI_CONFIDENCE {
		conf = MAX_AI_CONFIDENCE;
	}
	if conf > 1.0 {
		conf = 1.0;
	}
	let kind = if conf >= FACT_CONFIDENCE {
		EntityKind::Fact
	} else {
		EntityKind::Claim
	};
	(conf, kind)
}

#[cfg(test)]
mod online_softmax_tests {
	use super::OnlineSoftmax;

	#[test]
	fn empty_finalizes_to_neg_infinity() {
		assert_eq!(OnlineSoftmax::new().finalize(), f64::NEG_INFINITY);
	}

	#[test]
	fn single_observation_is_identity() {
		let mut s = OnlineSoftmax::new();
		s.update(0.7);
		assert!((s.finalize() - 0.7).abs() < 1e-12);
	}

	#[test]
	fn two_equal_observations_add_ln2() {
		let mut s = OnlineSoftmax::new();
		s.update(0.5);
		s.update(0.5);
		assert!((s.finalize() - (0.5 + 2.0_f64.ln())).abs() < 1e-12);
	}

	#[test]
	fn corroborated_item_can_outrank_higher_single_observation() {
		// Intentional pooling: an item seen twice at 0.8 (0.8 + ln2 ~= 1.49)
		// outranks an item seen once at 0.9. Pins the design decision so a
		// future switch to running_max is a deliberate, test-breaking change.
		let mut corroborated = OnlineSoftmax::new();
		corroborated.update(0.8);
		corroborated.update(0.8);
		let mut single = OnlineSoftmax::new();
		single.update(0.9);
		assert!(corroborated.finalize() > single.finalize());
		// running_max would reverse this (0.8 < 0.9).
		assert!(corroborated.running_max() < single.running_max());
	}
}
