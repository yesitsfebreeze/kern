use super::constants::*;
use super::types::{EntityKind, Kern, ReasonKind};
use super::util;

pub fn cosine(a: &[f32], b: &[f32]) -> f64 {
	#[cfg(target_arch = "x86_64")]
	{
		if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") {
			return unsafe { cosine_avx2(a, b) };
		}
	}
	cosine_scalar(a, b)
}

fn cosine_scalar(a: &[f32], b: &[f32]) -> f64 {
	let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
	for (ai, bi) in a.iter().zip(b.iter()) {
		dot += ai * bi;
		na += ai * ai;
		nb += bi * bi;
	}
	if na == 0.0 || nb == 0.0 {
		return 0.0;
	}
	(dot as f64) / ((na as f64).sqrt() * (nb as f64).sqrt())
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn cosine_avx2(a: &[f32], b: &[f32]) -> f64 {
	use std::arch::x86_64::*;

	// SAFETY INVARIANT for every unchecked access below: `n = min(a.len, b.len)`,
	// `chunks = n / 8`, `rem = n % 8`, `tail = chunks * 8`. Therefore:
	//  - the loaded chunks span offsets `0..tail` and each `loadu_ps` reads 8
	//    lanes at `off = i*8` where `off + 8 <= chunks*8 = tail <= n`, so it stays
	//    within both slices (`tail <= a.len()` and `tail <= b.len()`);
	//  - the tail loop indexes `tail + i` for `i in 0..rem`, and
	//    `tail + rem = chunks*8 + n%8 = n <= a.len()` (and `<= b.len()`),
	//    so `get_unchecked(tail + i)` is always in bounds.
	let n = a.len().min(b.len());
	let chunks = n / 8;
	let rem = n % 8;

	let mut vdot = _mm256_setzero_ps();
	let mut vna = _mm256_setzero_ps();
	let mut vnb = _mm256_setzero_ps();

	let pa = a.as_ptr();
	let pb = b.as_ptr();

	for i in 0..chunks {
		let off = i * 8;
		// In bounds: off + 8 <= chunks*8 = tail <= n <= len of both slices.
		let va = _mm256_loadu_ps(pa.add(off));
		let vb = _mm256_loadu_ps(pb.add(off));
		vdot = _mm256_fmadd_ps(va, vb, vdot);
		vna = _mm256_fmadd_ps(va, va, vna);
		vnb = _mm256_fmadd_ps(vb, vb, vnb);
	}

	let mut dot = hsum_256_ps(vdot);
	let mut na = hsum_256_ps(vna);
	let mut nb = hsum_256_ps(vnb);

	let tail = chunks * 8;
	for i in 0..rem {
		// In bounds: tail + i < tail + rem = n <= len of both slices.
		let ai = *a.get_unchecked(tail + i);
		let bi = *b.get_unchecked(tail + i);
		dot += ai * bi;
		na += ai * ai;
		nb += bi * bi;
	}

	if na == 0.0 || nb == 0.0 {
		return 0.0;
	}
	(dot as f64) / ((na as f64).sqrt() * (nb as f64).sqrt())
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn hsum_256_ps(v: std::arch::x86_64::__m256) -> f32 {
	use std::arch::x86_64::*;
	let high = _mm256_extractf128_ps(v, 1);
	let low = _mm256_castps256_ps128(v);
	let sum128 = _mm_add_ps(low, high);
	let hi64 = _mm_movehl_ps(sum128, sum128);
	let sum64 = _mm_add_ps(sum128, hi64);
	let hi32 = _mm_shuffle_ps(sum64, sum64, 0b01);
	let total = _mm_add_ss(sum64, hi32);
	_mm_cvtss_f32(total)
}

pub fn cosine_distance(a: &[f32], b: &[f32]) -> f64 {
	1.0 - cosine(a, b)
}

pub fn average_vec(a: &[f32], b: &[f32]) -> Vec<f32> {
	a.iter()
		.zip(b.iter())
		.map(|(ai, bi)| (ai + bi) / 2.0)
		.collect()
}

/// L2-normalize `v` in place to unit Euclidean norm. A zero vector (norm 0) is
/// left unchanged rather than producing NaNs from a divide-by-zero. General
/// vector utility — lives here beside the other `base::math` primitives so every
/// retrieval/fusion path shares one implementation.
pub fn l2_normalize(v: &mut [f32]) {
	let norm = v.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>().sqrt() as f32;
	if norm > 0.0 {
		for x in v.iter_mut() {
			*x /= norm;
		}
	}
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
mod cosine_tests {
	use super::*;

	#[test]
	fn identical_vectors_are_one_orthogonal_are_zero() {
		assert!((cosine(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]) - 1.0).abs() < 1e-6);
		assert!(
			cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6,
			"orthogonal -> 0"
		);
	}

	#[test]
	fn zero_norm_inputs_return_zero_not_nan() {
		// na == 0 or nb == 0 short-circuits before the divide, so no NaN.
		assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
		assert_eq!(cosine(&[1.0, 1.0], &[0.0, 0.0]), 0.0);
		assert_eq!(cosine(&[0.0, 0.0], &[0.0, 0.0]), 0.0);
	}

	#[test]
	fn mismatched_lengths_compare_the_shared_prefix() {
		// n = min(len) = 2; the extra dim on the longer vector is ignored.
		let c = cosine(&[1.0, 0.0, 9.0], &[1.0, 0.0]);
		assert!(
			(c - 1.0).abs() < 1e-6,
			"shared prefix is identical -> 1.0, got {c}"
		);
		// Empty slice -> n = 0 -> both norms 0 -> 0.0 (no panic).
		assert_eq!(cosine(&[], &[1.0, 2.0]), 0.0);
	}

	/// On a machine with AVX2+FMA the public `cosine` takes the SIMD path; assert
	/// it agrees with the scalar reference across lengths that exercise BOTH the
	/// 8-wide chunk loop and the unchecked tail (e.g. 17 = 2*8 + 1).
	#[cfg(target_arch = "x86_64")]
	#[test]
	fn avx2_path_matches_scalar_reference() {
		if !(is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma")) {
			return; // no SIMD on this host; scalar already covered above
		}
		for len in [0usize, 1, 7, 8, 9, 15, 16, 17, 33, 100] {
			let a: Vec<f32> = (0..len).map(|i| i as f32 * 0.1 - 0.5).collect();
			let b: Vec<f32> = (0..len).map(|i| (len - i) as f32 * 0.2 + 0.3).collect();
			let scalar = cosine_scalar(&a, &b);
			// SAFETY: guarded by the runtime avx2+fma feature check above.
			let simd = unsafe { cosine_avx2(&a, &b) };
			assert!(
				(scalar - simd).abs() < 1e-5,
				"len {len}: avx2 {simd} vs scalar {scalar}"
			);
		}
	}
}

#[cfg(test)]
mod l2_normalize_tests {
	use super::l2_normalize;

	#[test]
	fn scales_to_unit_norm() {
		let mut v = vec![3.0f32, 4.0]; // norm 5
		l2_normalize(&mut v);
		assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
		let norm = v.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>().sqrt();
		assert!((norm - 1.0).abs() < 1e-6);
	}

	#[test]
	fn zero_vector_is_left_unchanged() {
		let mut v = vec![0.0f32, 0.0, 0.0];
		l2_normalize(&mut v);
		assert_eq!(v, vec![0.0, 0.0, 0.0], "no divide-by-zero / NaN");
	}

	#[test]
	fn empty_slice_is_a_noop() {
		let mut v: Vec<f32> = vec![];
		l2_normalize(&mut v);
		assert!(v.is_empty());
	}
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
