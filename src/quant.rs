use serde::{Deserialize, Serialize};

pub const INT8_MAX_ABS: f32 = 127.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[repr(u8)]
pub enum QuantizationMode {
	#[default]
	None = 0,
	Int8 = 1,
}

impl QuantizationMode {
	pub fn parse(s: &str) -> Option<Self> {
		match s.trim().to_ascii_lowercase().as_str() {
			"none" | "f64" | "off" => Some(Self::None),
			"int8" | "i8" => Some(Self::Int8),
			_ => None,
		}
	}

	pub fn as_str(self) -> &'static str {
		match self {
			Self::None => "none",
			Self::Int8 => "int8",
		}
	}

	/// Storage cost per vector dimension, for size estimates only. `f32` (not
	/// `f64`) because it feeds display/back-of-envelope math — keeping it narrow
	/// avoids a silent widening at the (printf-style) call sites.
	pub fn bytes_per_dim(self) -> f32 {
		match self {
			Self::None => 8.0,
			Self::Int8 => 1.0,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuantizedVec {
	pub mode: QuantizationMode,
	pub scale: f32,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub f: Vec<f64>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub q: Vec<i8>,
}

impl QuantizedVec {
	pub fn encode(v: &[f64], mode: QuantizationMode) -> Self {
		match mode {
			QuantizationMode::None => Self {
				mode,
				scale: 0.0,
				f: v.to_vec(),
				q: Vec::new(),
			},
			QuantizationMode::Int8 => encode_int8(v),
		}
	}

	pub fn decode(&self) -> Vec<f64> {
		match self.mode {
			QuantizationMode::None => self.f.clone(),
			QuantizationMode::Int8 => self
				.q
				.iter()
				.map(|&qi| (qi as f64) * (self.scale as f64))
				.collect(),
		}
	}

	pub fn dim(&self) -> usize {
		match self.mode {
			QuantizationMode::None => self.f.len(),
			QuantizationMode::Int8 => self.q.len(),
		}
	}
}

fn encode_int8(v: &[f64]) -> QuantizedVec {
	if v.is_empty() {
		return QuantizedVec {
			mode: QuantizationMode::Int8,
			scale: 0.0,
			f: Vec::new(),
			q: Vec::new(),
		};
	}
	let max_abs = v.iter().fold(0.0_f64, |m, &x| m.max(x.abs()));
	let scale = if max_abs == 0.0 {
		1.0_f32
	} else {
		(max_abs as f32) / INT8_MAX_ABS
	};
	let inv = 1.0_f32 / scale;
	let q: Vec<i8> = v
		.iter()
		.map(|&x| {
			let scaled = (x as f32) * inv;
			let rounded = scaled.round();
			rounded.clamp(-INT8_MAX_ABS, INT8_MAX_ABS) as i8
		})
		.collect();
	QuantizedVec {
		mode: QuantizationMode::Int8,
		scale,
		f: Vec::new(),
		q,
	}
}

pub fn quantized_cosine_distance(a: &QuantizedVec, b: &QuantizedVec) -> f64 {
	match (a.mode, b.mode) {
		(QuantizationMode::Int8, QuantizationMode::Int8) => {
			int8_cosine_distance(&a.q, &b.q) as f64
		}
		_ => {
			let av = a.decode();
			let bv = b.decode();
			f64_cosine_distance(&av, &bv)
		}
	}
}

pub fn f64_cosine_distance(a: &[f64], b: &[f64]) -> f64 {
	if a.is_empty() || b.is_empty() || a.len() != b.len() {
		return 1.0;
	}
	let mut dot = 0.0_f64;
	let mut na = 0.0_f64;
	let mut nb = 0.0_f64;
	// Hot path. This dot/norm accumulation over two equal-length f64 slices is a
	// prime autovectorisation target; it is kept as a plain scalar loop (which the
	// compiler auto-vectorises under -O) for portability. A future contributor
	// wanting explicit SIMD should add a `#[cfg(target_feature = "avx2")]`
	// specialisation here and fall back to this loop otherwise.
	for i in 0..a.len() {
		dot += a[i] * b[i];
		na += a[i] * a[i];
		nb += b[i] * b[i];
	}
	let denom = (na * nb).sqrt();
	if denom == 0.0 {
		return 1.0;
	}
	let cos = (dot / denom).clamp(-1.0, 1.0);
	1.0 - cos
}

fn int8_cosine_distance(a: &[i8], b: &[i8]) -> f32 {
	let n = a.len();
	if n == 0 || n != b.len() {
		return 1.0;
	}
	let mut dot: i32 = 0;
	let mut na: i32 = 0;
	let mut nb: i32 = 0;
	for i in 0..n {
		let ai = a[i] as i32;
		let bi = b[i] as i32;
		dot += ai * bi;
		na += ai * ai;
		nb += bi * bi;
	}
	if na == 0 || nb == 0 {
		return 1.0;
	}
	let denom = ((na as f32) * (nb as f32)).sqrt();
	let cos = ((dot as f32) / denom).clamp(-1.0, 1.0);
	1.0 - cos
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn int8_round_trip_within_scale() {
		let v = vec![1.0, -2.0, 0.5, 0.0, -0.25];
		let qv = QuantizedVec::encode(&v, QuantizationMode::Int8);
		let d = qv.decode();
		assert_eq!(d.len(), v.len());
		for (orig, got) in v.iter().zip(&d) {
			assert!(
				(orig - got).abs() <= qv.scale as f64 + 1e-9,
				"{orig} vs {got} (scale {})",
				qv.scale
			);
		}
	}

	#[test]
	fn none_mode_is_lossless() {
		let v = vec![1.5, -0.3, 9.0];
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
	fn mixed_mode_falls_back_to_decoded_f64() {
		let a = QuantizedVec::encode(&[1.0, 2.0, 3.0], QuantizationMode::Int8);
		let b = QuantizedVec::encode(&[1.0, 2.0, 3.0], QuantizationMode::None);
		assert!(quantized_cosine_distance(&a, &b) < 1e-2);
	}

	#[test]
	fn mixed_mode_exactly_matches_the_decoded_f64_distance() {
		// The `< 1e-2` check above only proves the result is SMALL (and can't be
		// tighter — int8 is lossy, so a same-content mixed pair never reaches < eps).
		// The precise contract is that the fallback arm decodes BOTH operands and
		// delegates to f64_cosine_distance — so the result must equal that exactly,
		// and be the same whichever operand is the quantized one (order-symmetric).
		let int8 = QuantizedVec::encode(&[1.0, -2.0, 3.0, 0.5], QuantizationMode::Int8);
		let none = QuantizedVec::encode(&[1.0, -2.0, 3.0, 0.5], QuantizationMode::None);
		let expected = f64_cosine_distance(&int8.decode(), &none.decode());

		assert_eq!(quantized_cosine_distance(&int8, &none), expected, "int8 vs none == decoded f64");
		assert_eq!(quantized_cosine_distance(&none, &int8), expected, "none vs int8 is symmetric");
	}

	#[test]
	fn f64_cosine_edge_cases() {
		assert_eq!(f64_cosine_distance(&[], &[]), 1.0);
		assert_eq!(f64_cosine_distance(&[1.0, 2.0], &[1.0]), 1.0); // len mismatch
		assert_eq!(f64_cosine_distance(&[0.0, 0.0], &[1.0, 1.0]), 1.0); // zero vec
		assert!(f64_cosine_distance(&[1.0, 1.0], &[1.0, 1.0]) < 1e-12); // identical
	}

	#[test]
	fn mode_parse_round_trip() {
		assert_eq!(QuantizationMode::parse("int8"), Some(QuantizationMode::Int8));
		assert_eq!(QuantizationMode::parse(" NONE "), Some(QuantizationMode::None));
		assert_eq!(QuantizationMode::parse("bogus"), None);
		assert_eq!(QuantizationMode::Int8.as_str(), "int8");
	}
}
