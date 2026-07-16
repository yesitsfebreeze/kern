//! Int8 / scalar quantisation of embedding **vectors**: store each f32 dimension
//! as one signed byte (4× smaller) for the on-disk and in-memory search index,
//! keeping the original f32 vector for rescoring. This is vector quantisation for
//! the index — not LLM-model quantisation.

use serde::{Deserialize, Serialize};

pub const INT8_MAX_ABS: f32 = 127.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[repr(u8)]
pub enum QuantizationMode {
	#[default]
	None = 0,
	Int8 = 1,
	/// 1-bit sign quantisation: one bit per dimension (8 dims/byte), ranked by
	/// Hamming distance for candidate generation and rescored with the retained
	/// f32 vector. ~32× smaller index vectors than f32. In-memory only — the
	/// on-disk projection (`StoredVec`) stays int8.
	Binary = 2,
}

impl QuantizationMode {
	pub fn parse(s: &str) -> Option<Self> {
		match s.trim().to_ascii_lowercase().as_str() {
			"none" | "f32" | "f64" | "off" => Some(Self::None),
			"int8" | "i8" => Some(Self::Int8),
			// `Binary` is deliberately NOT user-selectable yet: pure 1-bit Hamming
			// measures recall@10 ~0.33 (see `binary_recall_tracks_f64`), below int8's
			// 0.75. It is wired + tested internally (`with_mode`) but stays out of the
			// config surface until int8-rescore lifts recall to a usable floor.
			_ => None,
		}
	}

	pub fn as_str(self) -> &'static str {
		match self {
			Self::None => "none",
			Self::Int8 => "int8",
			Self::Binary => "binary",
		}
	}

	/// Storage cost per vector dimension, for size estimates only. Narrow `f32`
	/// because it feeds display/back-of-envelope math — keeping it narrow
	/// avoids a silent widening at the (printf-style) call sites.
	pub fn bytes_per_dim(self) -> f32 {
		match self {
			Self::None => 4.0,
			Self::Int8 => 1.0,
			Self::Binary => 0.125,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuantizedVec {
	pub mode: QuantizationMode,
	pub scale: f32,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub f: Vec<f32>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub q: Vec<i8>,
	/// Packed sign bits for `Binary` mode (8 dims/byte), empty otherwise.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub b: Vec<u8>,
	/// True dimension count for `Binary` mode (the packed last byte is padded, so
	/// `b.len() * 8` over-counts). Zero for other modes. Safe to add: `QuantizedVec`
	/// is in-memory only and never persisted (see `store::StoredVec`).
	#[serde(default)]
	pub dim_bits: usize,
}

impl QuantizedVec {
	pub fn encode(v: &[f32], mode: QuantizationMode) -> Self {
		match mode {
			QuantizationMode::None => Self {
				mode,
				scale: 0.0,
				f: v.to_vec(),
				q: Vec::new(),
				b: Vec::new(),
				dim_bits: 0,
			},
			QuantizationMode::Int8 => encode_int8(v),
			QuantizationMode::Binary => encode_binary(v),
		}
	}

	pub fn decode(&self) -> Vec<f32> {
		match self.mode {
			QuantizationMode::None => self.f.clone(),
			QuantizationMode::Int8 => self
				.q
				.iter()
				.map(|&qi| (qi as f32) * self.scale)
				.collect(),
			// Reconstruct ±1.0 per sign bit. Coarse by design: the search path
			// rescores with the retained f32 vector, so this is only a fallback
			// (e.g. the `_` arm of `quantized_cosine_distance`).
			QuantizationMode::Binary => (0..self.dim_bits)
				.map(|i| {
					if self.b[i / 8] & (1 << (i % 8)) != 0 {
						1.0
					} else {
						-1.0
					}
				})
				.collect(),
		}
	}

	pub fn dim(&self) -> usize {
		match self.mode {
			QuantizationMode::None => self.f.len(),
			QuantizationMode::Int8 => self.q.len(),
			QuantizationMode::Binary => self.dim_bits,
		}
	}
}

fn encode_int8(v: &[f32]) -> QuantizedVec {
	if v.is_empty() {
		return QuantizedVec {
			mode: QuantizationMode::Int8,
			scale: 0.0,
			f: Vec::new(),
			q: Vec::new(),
			b: Vec::new(),
			dim_bits: 0,
		};
	}
	let max_abs = v.iter().fold(0.0_f32, |m, &x| m.max(x.abs()));
	let scale = if max_abs == 0.0 {
		1.0_f32
	} else {
		max_abs / INT8_MAX_ABS
	};
	let inv = 1.0_f32 / scale;
	let q: Vec<i8> = v
		.iter()
		.map(|&x| {
			let scaled = x * inv;
			let rounded = scaled.round();
			rounded.clamp(-INT8_MAX_ABS, INT8_MAX_ABS) as i8
		})
		.collect();
	QuantizedVec {
		mode: QuantizationMode::Int8,
		scale,
		f: Vec::new(),
		q,
		b: Vec::new(),
		dim_bits: 0,
	}
}

/// Pack each dimension into one sign bit (1 iff `x >= 0.0`), 8 dims/byte.
fn encode_binary(v: &[f32]) -> QuantizedVec {
	let mut b = vec![0u8; v.len().div_ceil(8)];
	for (i, &x) in v.iter().enumerate() {
		if x >= 0.0 {
			b[i / 8] |= 1 << (i % 8);
		}
	}
	QuantizedVec {
		mode: QuantizationMode::Binary,
		scale: 0.0,
		f: Vec::new(),
		q: Vec::new(),
		b,
		dim_bits: v.len(),
	}
}

/// Cosine-distance estimate from two sign-bit vectors via Hamming distance.
/// For sign-random vectors the probability two dimensions disagree is `θ/π`,
/// so `θ ≈ π · hamming/dim` and the estimated cosine is `cos(θ)`. Returns a
/// distance `1 - cos(θ)` in `[0, 2]`, matching the scale of the float/int8 paths,
/// and is monotone in Hamming distance (all that candidate-gen ranking needs).
fn binary_cosine_distance(a: &QuantizedVec, b: &QuantizedVec) -> f64 {
	let dim = a.dim_bits.min(b.dim_bits);
	if dim == 0 || a.b.len() != b.b.len() {
		return 1.0;
	}
	let hamming: u32 = a
		.b
		.iter()
		.zip(&b.b)
		.map(|(x, y)| (x ^ y).count_ones())
		.sum();
	let theta = std::f64::consts::PI * (hamming as f64) / (dim as f64);
	1.0 - theta.cos()
}

pub fn quantized_cosine_distance(a: &QuantizedVec, b: &QuantizedVec) -> f64 {
	match (a.mode, b.mode) {
		(QuantizationMode::Int8, QuantizationMode::Int8) => int8_cosine_distance(&a.q, &b.q) as f64,
		(QuantizationMode::Binary, QuantizationMode::Binary) => binary_cosine_distance(a, b),
		_ => {
			let av = a.decode();
			let bv = b.decode();
			float_cosine_distance(&av, &bv)
		}
	}
}

pub fn float_cosine_distance(a: &[f32], b: &[f32]) -> f64 {
	if a.is_empty() || b.is_empty() || a.len() != b.len() {
		return 1.0;
	}
	// Delegate the dot/norm work to the shared `base::math` cosine kernel, which
	// runtime-selects an AVX2+FMA path and falls back to scalar. A zero-norm input
	// returns similarity 0.0 there, giving distance 1.0 here.
	1.0 - crate::base::math::cosine(a, b)
}

fn int8_cosine_distance(a: &[i8], b: &[i8]) -> f32 {
	let n = a.len();
	if n == 0 || n != b.len() {
		return 1.0;
	}
	let (dot, na, nb) = int8_dot_norms(a, b);
	if na == 0 || nb == 0 {
		return 1.0;
	}
	let denom = ((na as f32) * (nb as f32)).sqrt();
	let cos = ((dot as f32) / denom).clamp(-1.0, 1.0);
	1.0 - cos
}

fn int8_dot_norms(a: &[i8], b: &[i8]) -> (i32, i32, i32) {
	#[cfg(target_arch = "x86_64")]
	{
		if is_x86_feature_detected!("avx2") {
			return unsafe { int8_dot_norms_avx2(a, b) };
		}
	}
	int8_dot_norms_scalar(a, b)
}

fn int8_dot_norms_scalar(a: &[i8], b: &[i8]) -> (i32, i32, i32) {
	let (mut dot, mut na, mut nb) = (0i32, 0i32, 0i32);
	for (&ai, &bi) in a.iter().zip(b.iter()) {
		let (ai, bi) = (ai as i32, bi as i32);
		dot += ai * bi;
		na += ai * ai;
		nb += bi * bi;
	}
	(dot, na, nb)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn int8_dot_norms_avx2(a: &[i8], b: &[i8]) -> (i32, i32, i32) {
	use std::arch::x86_64::*;

	// SAFETY INVARIANT: callers pass equal-length slices, so `n = a.len() = b.len()`.
	// `chunks = n / 16`, `tail = chunks * 16`. Each iteration loads 16 bytes at
	// `off = i*16` where `off + 16 <= chunks*16 = tail <= n`, staying within both
	// slices. The scalar tail loop indexes `tail..n`, all `< n <= len`, so every
	// `get_unchecked` is in bounds. `cvtepi8_epi16` sign-extends the 16 i8 lanes to
	// i16; `madd_epi16` multiplies signed i16 pairwise into i32 (max |lane| = 128,
	// pair sum <= 32768) and we accumulate into i32 lanes — the same values and
	// range as the scalar reference, so results match exactly.
	let n = a.len();
	let chunks = n / 16;

	let mut vdot = _mm256_setzero_si256();
	let mut vna = _mm256_setzero_si256();
	let mut vnb = _mm256_setzero_si256();

	let pa = a.as_ptr();
	let pb = b.as_ptr();

	for i in 0..chunks {
		let off = i * 16;
		let a8 = _mm_loadu_si128(pa.add(off) as *const __m128i);
		let b8 = _mm_loadu_si128(pb.add(off) as *const __m128i);
		let a16 = _mm256_cvtepi8_epi16(a8);
		let b16 = _mm256_cvtepi8_epi16(b8);
		vdot = _mm256_add_epi32(vdot, _mm256_madd_epi16(a16, b16));
		vna = _mm256_add_epi32(vna, _mm256_madd_epi16(a16, a16));
		vnb = _mm256_add_epi32(vnb, _mm256_madd_epi16(b16, b16));
	}

	let mut dot = hsum_256_epi32(vdot);
	let mut na = hsum_256_epi32(vna);
	let mut nb = hsum_256_epi32(vnb);

	let tail = chunks * 16;
	for i in tail..n {
		let ai = *a.get_unchecked(i) as i32;
		let bi = *b.get_unchecked(i) as i32;
		dot += ai * bi;
		na += ai * ai;
		nb += bi * bi;
	}
	(dot, na, nb)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn hsum_256_epi32(v: std::arch::x86_64::__m256i) -> i32 {
	use std::arch::x86_64::*;
	let hi = _mm256_extracti128_si256(v, 1);
	let lo = _mm256_castsi256_si128(v);
	let sum128 = _mm_add_epi32(lo, hi);
	let hi64 = _mm_unpackhi_epi64(sum128, sum128);
	let sum64 = _mm_add_epi32(sum128, hi64);
	let hi32 = _mm_shuffle_epi32(sum64, 0b01);
	let sum32 = _mm_add_epi32(sum64, hi32);
	_mm_cvtsi128_si32(sum32)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn int8_round_trip_within_scale() {
		let v = vec![1.0f32, -2.0, 0.5, 0.0, -0.25];
		let qv = QuantizedVec::encode(&v, QuantizationMode::Int8);
		let d = qv.decode();
		assert_eq!(d.len(), v.len());
		for (orig, got) in v.iter().zip(&d) {
			assert!(
				(orig - got).abs() <= qv.scale + 1e-6,
				"{orig} vs {got} (scale {})",
				qv.scale
			);
		}
	}

	#[test]
	fn none_mode_is_lossless() {
		let v = vec![1.5f32, -0.3, 9.0];
		let qv = QuantizedVec::encode(&v, QuantizationMode::None);
		assert_eq!(qv.decode(), v);
	}

	#[test]
	fn empty_and_zero_vectors() {
		let empty = QuantizedVec::encode(&[], QuantizationMode::Int8);
		assert_eq!(empty.dim(), 0);
		assert!(empty.decode().is_empty());

		let zero = QuantizedVec::encode(&[0.0, 0.0, 0.0], QuantizationMode::Int8);
		assert!(zero.q.iter().all(|&q| q == 0));
		assert_eq!(zero.decode(), vec![0.0, 0.0, 0.0]);
	}

	#[test]
	fn int8_cosine_identical_is_zero_orthogonal_is_one() {
		let a = QuantizedVec::encode(&[1.0, 2.0, 3.0], QuantizationMode::Int8);
		let b = QuantizedVec::encode(&[1.0, 2.0, 3.0], QuantizationMode::Int8);
		assert!(quantized_cosine_distance(&a, &b) < 1e-3);

		let x = QuantizedVec::encode(&[1.0, 0.0], QuantizationMode::Int8);
		let y = QuantizedVec::encode(&[0.0, 1.0], QuantizationMode::Int8);
		assert!((quantized_cosine_distance(&x, &y) - 1.0).abs() < 1e-3);
	}

	#[test]
	fn mixed_mode_falls_back_to_decoded_float() {
		let a = QuantizedVec::encode(&[1.0, 2.0, 3.0], QuantizationMode::Int8);
		let b = QuantizedVec::encode(&[1.0, 2.0, 3.0], QuantizationMode::None);
		assert!(quantized_cosine_distance(&a, &b) < 1e-2);
	}

	#[test]
	fn mixed_mode_exactly_matches_the_decoded_float_distance() {
		// The `< 1e-2` check above only proves the result is SMALL (and can't be
		// tighter — int8 is lossy, so a same-content mixed pair never reaches < eps).
		// The precise contract is that the fallback arm decodes BOTH operands and
		// delegates to float_cosine_distance — so the result must equal that exactly,
		// and be the same whichever operand is the quantized one (order-symmetric).
		let int8 = QuantizedVec::encode(&[1.0, -2.0, 3.0, 0.5], QuantizationMode::Int8);
		let none = QuantizedVec::encode(&[1.0, -2.0, 3.0, 0.5], QuantizationMode::None);
		let expected = float_cosine_distance(&int8.decode(), &none.decode());

		assert_eq!(
			quantized_cosine_distance(&int8, &none),
			expected,
			"int8 vs none == decoded float"
		);
		assert_eq!(
			quantized_cosine_distance(&none, &int8),
			expected,
			"none vs int8 is symmetric"
		);
	}

	#[test]
	fn float_cosine_edge_cases() {
		assert_eq!(float_cosine_distance(&[], &[]), 1.0);
		assert_eq!(float_cosine_distance(&[1.0, 2.0], &[1.0]), 1.0); // len mismatch
		assert_eq!(float_cosine_distance(&[0.0, 0.0], &[1.0, 1.0]), 1.0); // zero vec
		assert!(float_cosine_distance(&[1.0, 1.0], &[1.0, 1.0]) < 1e-6); // identical
	}

	#[test]
	fn mode_parse_round_trip() {
		assert_eq!(
			QuantizationMode::parse("int8"),
			Some(QuantizationMode::Int8)
		);
		assert_eq!(
			QuantizationMode::parse(" NONE "),
			Some(QuantizationMode::None)
		);
		assert_eq!(QuantizationMode::parse("bogus"), None);
		assert_eq!(QuantizationMode::Int8.as_str(), "int8");
		// Binary is intentionally NOT parseable from config yet (pure 1-bit recall
		// ~0.33 < int8 0.75), but its display + size accounting are defined.
		assert_eq!(
			QuantizationMode::parse("binary"),
			None,
			"not config-exposed until rescore"
		);
		assert_eq!(QuantizationMode::Binary.as_str(), "binary");
		assert_eq!(QuantizationMode::Binary.bytes_per_dim(), 0.125);
	}

	#[test]
	fn binary_packs_one_sign_bit_per_dim() {
		// >=0 -> 1, <0 -> 0. 10 dims -> 2 bytes (ceil(10/8)), dim_bits records 10.
		let v = vec![1.0f32, -1.0, 0.0, -0.5, 2.0, -3.0, 0.1, -0.1, 5.0, -5.0];
		let qv = QuantizedVec::encode(&v, QuantizationMode::Binary);
		assert_eq!(
			qv.dim(),
			10,
			"dim_bits is the true dimension, not b.len()*8"
		);
		assert_eq!(qv.b.len(), 2, "10 dims pack into ceil(10/8)=2 bytes");
		// bit i set iff v[i] >= 0: indices 0,2,4,6,8 set in byte 0/1.
		// byte0 bits {0,2,4,6} = 0b01010101 = 0x55; byte1 bit {8->bit0} = 0b01 = 0x01.
		assert_eq!(qv.b[0], 0b0101_0101, "low byte sign pattern");
		assert_eq!(qv.b[1], 0b0000_0001, "high byte: only dim 8 (>=0) set");
	}

	#[test]
	fn binary_decode_reconstructs_signs() {
		let v = vec![3.0f32, -2.0, 0.0, -7.0];
		let qv = QuantizedVec::encode(&v, QuantizationMode::Binary);
		assert_eq!(
			qv.decode(),
			vec![1.0, -1.0, 1.0, -1.0],
			"0.0 counts as + (>=0)"
		);
	}

	#[test]
	fn binary_distance_zero_for_identical_and_monotone_in_angle() {
		// Identical sign patterns -> Hamming 0 -> distance 0.
		let a = QuantizedVec::encode(&[1.0, 1.0, 1.0, 1.0], QuantizationMode::Binary);
		let b = QuantizedVec::encode(&[1.0, 1.0, 1.0, 1.0], QuantizationMode::Binary);
		assert!(
			quantized_cosine_distance(&a, &b).abs() < 1e-12,
			"identical signs -> 0"
		);

		// Opposed sign patterns -> Hamming = dim -> cos(pi) = -1 -> distance 2.
		let c = QuantizedVec::encode(&[-1.0, -1.0, -1.0, -1.0], QuantizationMode::Binary);
		assert!(
			(quantized_cosine_distance(&a, &c) - 2.0).abs() < 1e-12,
			"all bits differ -> 2"
		);

		// Half the bits differ -> Hamming/dim = 0.5 -> cos(pi/2)=0 -> distance 1.
		let d = QuantizedVec::encode(&[1.0, 1.0, -1.0, -1.0], QuantizationMode::Binary);
		assert!(
			(quantized_cosine_distance(&a, &d) - 1.0).abs() < 1e-12,
			"half differ -> 1"
		);
	}

	/// The AVX2 int8 dot/norm kernel must agree bit-for-bit with the scalar
	/// reference across random vectors and the edge cases that stress the widen +
	/// pairwise-madd path: lengths spanning the 16-wide chunk boundary and its
	/// tail, and extreme i8 values (±127 and -128, which encode never emits but the
	/// kernel must still handle identically to scalar).
	#[cfg(target_arch = "x86_64")]
	#[test]
	fn int8_avx2_dot_norms_match_scalar_reference() {
		if !is_x86_feature_detected!("avx2") {
			return; // no SIMD on this host; scalar path already exercised elsewhere
		}
		// Cheap deterministic LCG so the test needs no rng dependency.
		let mut state = 0x2545_f491_4f6c_dd1d_u64;
		let mut next_i8 = || {
			state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
			(state >> 33) as i8
		};
		// Lengths around the chunk boundary (16) plus a longer multi-chunk case.
		for &len in &[0usize, 1, 7, 15, 16, 17, 31, 33, 64, 100] {
			let a: Vec<i8> = (0..len).map(|_| next_i8()).collect();
			let b: Vec<i8> = (0..len).map(|_| next_i8()).collect();
			let scalar = int8_dot_norms_scalar(&a, &b);
			// SAFETY: guarded by the runtime avx2 feature check above; a.len()==b.len().
			let simd = unsafe { int8_dot_norms_avx2(&a, &b) };
			assert_eq!(scalar, simd, "len {len}: avx2 {simd:?} vs scalar {scalar:?}");
		}
		// Extreme lanes: all +127, all -128, and a mix, over a chunk+tail length.
		for pattern in [
			vec![127i8; 20],
			vec![-128i8; 20],
			(0..20).map(|i| if i % 2 == 0 { 127 } else { -128 }).collect(),
		] {
			let scalar = int8_dot_norms_scalar(&pattern, &pattern);
			// SAFETY: avx2 checked above; equal-length inputs.
			let simd = unsafe { int8_dot_norms_avx2(&pattern, &pattern) };
			assert_eq!(scalar, simd, "extreme lanes: avx2 {simd:?} vs scalar {scalar:?}");
		}
	}

	#[test]
	fn binary_hamming_ranking_tracks_true_cosine() {
		// The point of binary candidate-gen: closer-by-cosine must rank nearer-by-Hamming.
		// query ~ near shares more sign bits than query ~ far.
		let query = vec![1.0f32, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
		let near = vec![1.0f32, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, -1.0]; // 1 sign flip
		let far = vec![-1.0f32, -1.0, -1.0, -1.0, 1.0, 1.0, 1.0, 1.0]; // 4 sign flips
		let q = QuantizedVec::encode(&query, QuantizationMode::Binary);
		let n = QuantizedVec::encode(&near, QuantizationMode::Binary);
		let f = QuantizedVec::encode(&far, QuantizationMode::Binary);
		assert!(
			quantized_cosine_distance(&q, &n) < quantized_cosine_distance(&q, &f),
			"fewer sign flips -> smaller Hamming distance"
		);
	}
}
