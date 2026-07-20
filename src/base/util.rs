use sha2::{Digest, Sha256};

// Keeps in-memory Vec<f32> byte-identical to the legacy Vec<f64> wire shape:
// write widens f32→f64, read narrows f64→f32. Changing this desyncs every store.
pub mod vec_f64_compat {
	use serde::{Deserialize, Deserializer, Serializer};

	pub fn serialize<S: Serializer>(v: &[f32], s: S) -> Result<S::Ok, S::Error> {
		s.collect_seq(v.iter().map(|&x| x as f64))
	}

	pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f32>, D::Error> {
		Ok(
			Vec::<f64>::deserialize(d)?
				.into_iter()
				.map(|x| x as f32)
				.collect(),
		)
	}
}

pub fn content_hash(s: &str) -> String {
	let hash = Sha256::digest(s.as_bytes());
	hex::encode(hash)
}

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

pub fn cmp_partial<T: PartialOrd>(a: &T, b: &T) -> std::cmp::Ordering {
	a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
}

// Score desc, id asc — the single ranking tiebreak; use at every ranking site
// or top-k regresses to nondeterministic order.
pub fn cmp_rank<S: PartialOrd>(
	a_score: S,
	a_id: &str,
	b_score: S,
	b_id: &str,
) -> std::cmp::Ordering {
	cmp_partial(&b_score, &a_score).then_with(|| a_id.cmp(b_id))
}

// Input must be ascending-sorted; p is a fraction in [0, 1].
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

pub fn now_nanos() -> u128 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_nanos()
}

pub fn now_ms() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.as_millis() as u64)
		.unwrap_or(0)
}

pub fn now_secs() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.as_secs())
		.unwrap_or(0)
}

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
		assert_eq!(
			percentile_sorted(&xs, 0.5),
			Some(5.0),
			"ceil(0.5*10)=5 -> xs[4]"
		);
		assert_eq!(percentile_sorted(&xs, 0.95), Some(10.0));
		assert_eq!(percentile_sorted::<f64>(&[], 0.5), None, "empty -> None");
		let ns: Vec<u128> = vec![10, 20, 30, 40, 50];
		assert_eq!(percentile_sorted(&ns, 0.5), Some(30u128));
		assert_eq!(percentile_sorted(&ns, 0.95), Some(50u128));
	}

	#[test]
	fn cmp_rank_orders_by_score_desc_then_id_asc() {
		use std::cmp::Ordering;
		assert_eq!(cmp_rank(0.9_f64, "z", 0.1, "a"), Ordering::Less);
		assert_eq!(cmp_rank(0.1_f64, "a", 0.9, "z"), Ordering::Greater);
		assert_eq!(cmp_rank(0.5_f64, "a", 0.5, "b"), Ordering::Less);
		assert_eq!(cmp_rank(0.5_f64, "b", 0.5, "a"), Ordering::Greater);
		assert_eq!(cmp_rank(0.5_f64, "a", 0.5, "a"), Ordering::Equal);
		assert_eq!(cmp_rank(f64::NAN, "a", f64::NAN, "b"), Ordering::Less);
		assert_eq!(cmp_rank(2.0_f32, "a", 1.0_f32, "z"), Ordering::Less);
	}

	#[test]
	fn content_hash_is_deterministic_64_char_lowercase_hex() {
		let h = content_hash("kern");
		assert_eq!(h.len(), 64, "sha256 -> 32 bytes -> 64 hex chars");
		assert!(h
			.bytes()
			.all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
		assert_eq!(h, content_hash("kern"), "deterministic");
		assert_ne!(h, content_hash("kern2"), "distinct inputs differ");
	}

	#[test]
	fn short_id_caps_at_12_chars_and_is_boundary_safe() {
		assert_eq!(short_id("0123456789abcdef"), "0123456789ab");
		assert_eq!(short_id("abc"), "abc");
		assert_eq!(short_id("0123456789ab"), "0123456789ab");
		let s = short_id("ααααααααααααββ");
		assert_eq!(s.chars().count(), 12);
	}

	#[test]
	fn truncate_appends_ellipsis_only_when_cut() {
		assert_eq!(truncate("hello", 10), "hello", "under max -> unchanged");
		assert_eq!(
			truncate("hello world", 5),
			"hello...",
			"over max -> cut + ellipsis"
		);
		assert_eq!(truncate("αβγδε", 3), "αβγ...");
	}

	#[test]
	fn cmp_partial_orders_and_treats_nan_as_equal() {
		use std::cmp::Ordering;
		assert_eq!(cmp_partial(&1.0, &2.0), Ordering::Less);
		assert_eq!(cmp_partial(&2.0, &1.0), Ordering::Greater);
		assert_eq!(cmp_partial(&1.0, &1.0), Ordering::Equal);
		assert_eq!(
			cmp_partial(&f64::NAN, &1.0),
			Ordering::Equal,
			"NaN is incomparable -> Equal"
		);
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
		assert_eq!(&groups[2][0..1], "4", "RFC4122 version 4");
		assert!(
			matches!(&groups[3][0..1], "8" | "9" | "a" | "b"),
			"RFC4122 variant bits"
		);
		assert_ne!(uuid_v4(), uuid_v4(), "two mints differ (random)");
	}

	#[test]
	fn now_nanos_is_after_epoch() {
		assert!(now_nanos() > 0);
	}
}

#[cfg(test)]
mod vec_f64_compat_tests {
	use serde::{Deserialize, Serialize};

	#[derive(Serialize, Deserialize, PartialEq, Debug)]
	struct Legacy {
		name: String,
		v: Vec<f64>,
		tail: u32,
	}

	#[derive(Serialize, Deserialize, PartialEq, Debug)]
	struct Current {
		name: String,
		#[serde(with = "super::vec_f64_compat")]
		v: Vec<f32>,
		tail: u32,
	}

	fn cfg() -> impl bincode::config::Config {
		bincode::config::standard()
	}

	#[test]
	fn shimmed_f32_encodes_byte_identically_to_legacy_f64() {
		let legacy = Legacy {
			name: "e".into(),
			v: vec![0.5, -1.25, 3.0],
			tail: 7,
		};
		let current = Current {
			name: "e".into(),
			v: vec![0.5, -1.25, 3.0],
			tail: 7,
		};
		let lb = bincode::serde::encode_to_vec(&legacy, cfg()).unwrap();
		let cb = bincode::serde::encode_to_vec(&current, cfg()).unwrap();
		assert_eq!(
			lb, cb,
			"wire bytes must match the legacy f64 layout exactly"
		);
	}

	#[test]
	fn legacy_f64_bytes_decode_into_f32_with_following_fields_intact() {
		let legacy = Legacy {
			name: "old-row".into(),
			v: vec![0.1, -0.2, 0.3],
			tail: 42,
		};
		let bytes = bincode::serde::encode_to_vec(&legacy, cfg()).unwrap();
		let (cur, _): (Current, _) = bincode::serde::decode_from_slice(&bytes, cfg()).unwrap();
		assert_eq!(cur.name, "old-row");
		assert_eq!(
			cur.tail, 42,
			"field after the vector decodes at the right offset"
		);
		for (got, want) in cur.v.iter().zip([0.1f32, -0.2, 0.3]) {
			assert!((got - want).abs() < 1e-7, "{got} vs {want}");
		}
	}

	#[test]
	fn f32_round_trip_is_lossless() {
		let current = Current {
			name: "rt".into(),
			v: vec![0.1f32, f32::MIN_POSITIVE, 12345.678, -0.0, 1.0e-30],
			tail: 0,
		};
		let bytes = bincode::serde::encode_to_vec(&current, cfg()).unwrap();
		let (back, _): (Current, _) = bincode::serde::decode_from_slice(&bytes, cfg()).unwrap();
		assert_eq!(back, current);
	}
}
