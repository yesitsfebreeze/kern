use sha2::{Digest, Sha256};

pub fn content_hash(s: &str) -> String {
	let hash = Sha256::digest(s.as_bytes());
	hex::encode(hash)
}

/// Hand-rolled lowercase hex encoder. Deliberately NOT the `hex` crate:
/// `content_hash` is the only hex consumer in kern, so a six-line local encoder
/// keeps one extra crate (and its transitive surface) out of the supply chain
/// for a single call site. If hex use ever spreads, switch to the crate.
mod hex {
	const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

	pub fn encode(bytes: impl AsRef<[u8]>) -> String {
		let bytes = bytes.as_ref();
		let mut s = String::with_capacity(bytes.len() * 2);
		for &b in bytes {
			s.push(HEX_CHARS[(b >> 4) as usize] as char);
			s.push(HEX_CHARS[(b & 0x0f) as usize] as char);
		}
		s
	}
}

pub fn short_id(id: &str) -> &str {
	match id.char_indices().nth(12) {
		Some((byte_pos, _)) => &id[..byte_pos],
		None => id,
	}
}

pub fn truncate(s: &str, max: usize) -> String {
	match s.char_indices().nth(max) {
		Some((byte_pos, _)) => format!("{}...", &s[..byte_pos]),
		None => s.to_string(),
	}
}

/// Total order over `PartialOrd` values, treating incomparable pairs (NaN)
/// as `Equal`. Replaces the `a.partial_cmp(&b).unwrap_or(Ordering::Equal)`
/// idiom scattered across the sort/rank paths.
pub fn cmp_partial<T: PartialOrd>(a: &T, b: &T) -> std::cmp::Ordering {
	a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
}

/// Deterministic ranking order for a top-k result set: higher `score` first,
/// ties broken by `id` ascending. NaN/incomparable scores fall back to `Equal`
/// (via [`cmp_partial`]). When the `id`s are unique this is a STRICT total order,
/// which is what makes a `select_nth`/`truncate` top-k reproducible across runs
/// despite HashMap/scan source order.
///
/// Single source of truth for the score-desc-id-asc tiebreak shared by
/// `fuse::rrf`, `pagerank`, `search::merge_hits`, `LexicalIndex::search`, and
/// `Store::cold_search`. Adding the tiebreak here once stops new ranking sites
/// from silently regressing to nondeterministic source-order ties.
pub fn cmp_rank<S: PartialOrd>(a_score: S, a_id: &str, b_score: S, b_id: &str) -> std::cmp::Ordering {
	cmp_partial(&b_score, &a_score).then_with(|| a_id.cmp(b_id))
}

/// Nearest-rank percentile of an ascending-SORTED slice. `p` is a fraction in
/// `[0, 1]` (`0.95` = p95). `None` only for an empty slice; `p <= 0` -> first
/// element, `p >= 1` -> last, otherwise the element at 1-based rank
/// `ceil(p * len)`. Generic so latency/timing percentiles (`u128`, `f64`, …)
/// share one implementation instead of re-deriving the index math per call site.
pub fn percentile_sorted<T: Copy>(sorted: &[T], p: f64) -> Option<T> {
	if sorted.is_empty() {
		return None;
	}
	if p <= 0.0 {
		return Some(sorted[0]);
	}
	if p >= 1.0 {
		return Some(sorted[sorted.len() - 1]);
	}
	let rank = (p * sorted.len() as f64).ceil() as usize;
	Some(sorted[rank.clamp(1, sorted.len()) - 1])
}

/// Wall-clock nanoseconds since the Unix epoch. Single source for the
/// `SystemTime::now().duration_since(UNIX_EPOCH)` stamp used to mint
/// gossip message ids.
pub fn now_nanos() -> u128 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_nanos()
}

/// Build the LLM prompt asking why two entities are related. Single source
/// for the prompt text and the 500-char truncation budget, shared by the
/// link/enrich paths in commands, mcp, and tick.
pub fn explain_relationship_prompt(a: &str, b: &str) -> String {
	format!(
		"Write one sentence describing the specific connection between these two pieces of knowledge. \
		Name the exact concept, mechanism, cause, or logical dependency that links them. \
		Do NOT use vague words like \"related\", \"similar\", \"connected\", or \"both deal with\".\n\n\
		A: {}\n\nB: {}\n\nConnection:",
		truncate(a, 500),
		truncate(b, 500),
	)
}

pub fn uuid_v4() -> String {
	use rand::RngExt;
	let mut rng = rand::rng();
	let mut b = [0u8; 16];
	rng.fill(&mut b);
	b[6] = (b[6] & 0x0f) | 0x40;
	b[8] = (b[8] & 0x3f) | 0x80;
	format!(
		"{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
		u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
		u16::from_be_bytes([b[4], b[5]]),
		u16::from_be_bytes([b[6], b[7]]),
		u16::from_be_bytes([b[8], b[9]]),
		u64::from_be_bytes([0, 0, b[10], b[11], b[12], b[13], b[14], b[15]]),
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn hex_encode_is_lowercase_two_chars_per_byte() {
		assert_eq!(hex::encode([0x00, 0xff, 0x10, 0xab]), "00ff10ab");
		assert_eq!(hex::encode([]), "");
	}

	#[test]
	fn percentile_sorted_is_nearest_rank_with_edges_and_generic_types() {
		let xs: Vec<f64> = (1..=10).map(|i| i as f64).collect();
		assert_eq!(percentile_sorted(&xs, 0.0), Some(1.0), "p<=0 -> first");
		assert_eq!(percentile_sorted(&xs, 1.0), Some(10.0), "p>=1 -> last");
		assert_eq!(percentile_sorted(&xs, 0.5), Some(5.0), "ceil(0.5*10)=5 -> xs[4]");
		assert_eq!(percentile_sorted(&xs, 0.95), Some(10.0));
		assert_eq!(percentile_sorted::<f64>(&[], 0.5), None, "empty -> None");
		// Generic over the element type (u128 nanos, the locomo latency case).
		let ns: Vec<u128> = vec![10, 20, 30, 40, 50];
		assert_eq!(percentile_sorted(&ns, 0.5), Some(30u128));
		assert_eq!(percentile_sorted(&ns, 0.95), Some(50u128));
	}

	#[test]
	fn cmp_rank_orders_by_score_desc_then_id_asc() {
		use std::cmp::Ordering;
		// Higher score ranks first regardless of id.
		assert_eq!(cmp_rank(0.9_f64, "z", 0.1, "a"), Ordering::Less);
		assert_eq!(cmp_rank(0.1_f64, "a", 0.9, "z"), Ordering::Greater);
		// Equal score -> id ascending decides (a before b).
		assert_eq!(cmp_rank(0.5_f64, "a", 0.5, "b"), Ordering::Less);
		assert_eq!(cmp_rank(0.5_f64, "b", 0.5, "a"), Ordering::Greater);
		// Fully equal -> Equal.
		assert_eq!(cmp_rank(0.5_f64, "a", 0.5, "a"), Ordering::Equal);
		// NaN score falls back to the id tiebreak rather than panicking.
		assert_eq!(cmp_rank(f64::NAN, "a", f64::NAN, "b"), Ordering::Less);
		// Works for f32 scores too (lexical BM25 path).
		assert_eq!(cmp_rank(2.0_f32, "a", 1.0_f32, "z"), Ordering::Less);
	}

	#[test]
	fn content_hash_is_deterministic_64_char_lowercase_hex() {
		let h = content_hash("kern");
		assert_eq!(h.len(), 64, "sha256 -> 32 bytes -> 64 hex chars");
		assert!(h.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
		assert_eq!(h, content_hash("kern"), "deterministic");
		assert_ne!(h, content_hash("kern2"), "distinct inputs differ");
	}

	#[test]
	fn short_id_caps_at_12_chars_and_is_boundary_safe() {
		assert_eq!(short_id("0123456789abcdef"), "0123456789ab"); // 16 -> first 12
		assert_eq!(short_id("abc"), "abc"); // shorter than 12 -> whole
		assert_eq!(short_id("0123456789ab"), "0123456789ab"); // exactly 12 -> whole
		// Multibyte: slicing must land on a char boundary, never panic.
		let s = short_id("ααααααααααααββ"); // each α is 2 bytes
		assert_eq!(s.chars().count(), 12);
	}

	#[test]
	fn truncate_appends_ellipsis_only_when_cut() {
		assert_eq!(truncate("hello", 10), "hello", "under max -> unchanged");
		assert_eq!(truncate("hello world", 5), "hello...", "over max -> cut + ellipsis");
		// Char-boundary safe on multibyte input.
		assert_eq!(truncate("αβγδε", 3), "αβγ...");
	}

	#[test]
	fn cmp_partial_orders_and_treats_nan_as_equal() {
		use std::cmp::Ordering;
		assert_eq!(cmp_partial(&1.0, &2.0), Ordering::Less);
		assert_eq!(cmp_partial(&2.0, &1.0), Ordering::Greater);
		assert_eq!(cmp_partial(&1.0, &1.0), Ordering::Equal);
		assert_eq!(cmp_partial(&f64::NAN, &1.0), Ordering::Equal, "NaN is incomparable -> Equal");
	}

	#[test]
	fn uuid_v4_has_correct_layout_version_and_variant() {
		let u = uuid_v4();
		let groups: Vec<&str> = u.split('-').collect();
		assert_eq!(
			groups.iter().map(|g| g.len()).collect::<Vec<_>>(),
			vec![8, 4, 4, 4, 12],
			"5 dash-separated groups of 8-4-4-4-12"
		);
		assert!(u.bytes().all(|c| c == b'-' || c.is_ascii_hexdigit()));
		// Version nibble: first char of the 3rd group is '4'.
		assert_eq!(&groups[2][0..1], "4", "RFC4122 version 4");
		// Variant: first char of the 4th group is one of 8/9/a/b.
		assert!(matches!(&groups[3][0..1], "8" | "9" | "a" | "b"), "RFC4122 variant bits");
		assert_ne!(uuid_v4(), uuid_v4(), "two mints differ (random)");
	}

	#[test]
	fn now_nanos_is_after_epoch() {
		assert!(now_nanos() > 0);
	}
}
