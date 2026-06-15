//! Deterministic hash-based embedding STUB — benchmarks only.
//!
//! Maps text to a fixed-[`DIM`] vector by feature-hashing each token into signed
//! slots, then L2-normalizing. It is NOT a semantic embedder: there is no
//! learned model, so cosine similarity reflects token *overlap*, not meaning.
//! The bench harness (`build.rs`, `replay.rs`) uses it to exercise the
//! retrieval/index path at scale without a live Ollama embedder. Never wire this
//! into production retrieval.

use crate::base::util::content_hash;

// 512 (not 64): each token deposits 4 signed values, so a ~10-token document
// writes ~40 slots. Into 64 that is ~40% collisions, which drowned the
// token-overlap signal and made the dense leg near-noise (mean recall@10 0.45
// on synthetic.json). At 512 collisions drop below ~8%, so cosine faithfully
// tracks token overlap and the harness reaches recall@10 1.0 — i.e. retrieval
// surfaces every token-overlapping relevant doc, with no residual hashing
// artifact. Still bench-only; never a substitute for a real semantic model.
pub const DIM: usize = 512;

pub fn embed(text: &str) -> Vec<f64> {
	let mut v = vec![0.0f64; DIM];
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
	// Shared primitive — same L2 normalization the retrieval path uses; no local
	// re-implementation (was a duplicate of base::math::l2_normalize).
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
		let norm: f64 = v.iter().map(|x| x * x).sum::<f64>().sqrt();
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
		// No tokens deposited -> norm 0 -> l2_normalize leaves zeros (no NaN).
		assert_eq!(embed(""), vec![0.0; DIM]);
		assert_eq!(embed("   !!! "), vec![0.0; DIM]);
	}

	#[test]
	fn identical_token_sets_match_and_disjoint_sets_diverge() {
		let base = embed("alpha beta gamma");
		// Same tokens, different order -> identical vector (sum is order-free).
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
