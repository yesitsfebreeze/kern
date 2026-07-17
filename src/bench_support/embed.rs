//! Feature-hashing embedding STUB for benchmarks only: cosine reflects token overlap, not meaning. Never wire into production.

use crate::base::util::content_hash;

pub const DIM: usize = 512;

pub fn embed(text: &str) -> Vec<f32> {
	let mut v = vec![0.0f32; DIM];
	for tok in tokenize(text) {
		let h = content_hash(&tok);
		let bytes = h.as_bytes();
		for chunk in 0..4 {
			let base = chunk * 4;
			let slot = (hex_u32(&bytes[base..base + 4]) as usize) % DIM;
			let sign = if (bytes[base + 4] & 1) == 0 {
				1.0
			} else {
				-1.0
			};
			v[slot] += sign;
		}
	}
	crate::base::math::l2_normalize(&mut v);
	v
}

fn tokenize(text: &str) -> Vec<String> {
	text
		.split(|c: char| !c.is_alphanumeric())
		.filter(|s| !s.is_empty())
		.map(|s| s.to_lowercase())
		.collect()
}

fn hex_u32(bytes: &[u8]) -> u32 {
	let mut n = 0u32;
	for &b in bytes {
		let v = match b {
			b'0'..=b'9' => b - b'0',
			b'a'..=b'f' => b - b'a' + 10,
			_ => 0,
		};
		n = (n << 4) | v as u32;
	}
	n
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::math::cosine;

	#[test]
	fn output_is_unit_length() {
		let v = embed("the quick brown fox");
		assert_eq!(v.len(), DIM);
		let norm = v
			.iter()
			.map(|&x| (x as f64) * (x as f64))
			.sum::<f64>()
			.sqrt();
		assert!((norm - 1.0).abs() < 1e-9, "L2 norm ~1, got {norm}");
	}

	#[test]
	fn deterministic_and_tokenization_is_case_punct_insensitive() {
		assert_eq!(embed("hello world"), embed("hello world"), "deterministic");
		assert_eq!(
			embed("Hello, World!"),
			embed("hello world"),
			"case/punct folded"
		);
	}

	#[test]
	fn empty_or_tokenless_input_is_a_zero_vector() {
		assert_eq!(embed(""), vec![0.0; DIM]);
		assert_eq!(embed("   !!! "), vec![0.0; DIM]);
	}

	#[test]
	fn identical_token_sets_match_and_disjoint_sets_diverge() {
		let base = embed("alpha beta gamma");
		let same = embed("gamma alpha beta");
		let diff = embed("delta epsilon zeta");
		assert!(
			(cosine(&base, &same) - 1.0).abs() < 1e-9,
			"same token set -> cosine 1.0"
		);
		assert!(
			cosine(&base, &diff) < cosine(&base, &same),
			"disjoint tokens less similar"
		);
	}
}
