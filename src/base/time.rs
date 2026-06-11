//! Minimal time parsing for the base layer.
//!
//! A dependency-free RFC3339 reader used by the retrieval/MCP filter path
//! (`since` / `before` / `valid_at`). Lives in `base` rather than `mcp` because
//! it has no MCP coupling and any transport/CLI layer needs the same parse.

/// Parse the fixed-offset `YYYY-MM-DDTHH:MM:SS` prefix of an RFC3339 timestamp
/// into a [`SystemTime`](std::time::SystemTime). The timezone suffix (`Z` /
/// `±hh:mm`) and sub-second fraction are ignored — callers only need
/// second-granularity wall-clock instants for filter bounds.
///
/// Returns `Err(())` for any malformed input (short-after-trim, non-ASCII /
/// multi-byte in the fixed slice region, non-numeric fields, or a pre-epoch
/// result) rather than panicking — the input is reachable from untrusted MCP
/// `since`/`before`/`valid_at` arguments.
pub fn parse_rfc3339(s: &str) -> Result<std::time::SystemTime, ()> {
	let s = s.trim();
	// All fixed-offset slices below read bytes 0..19. Validate length AFTER
	// trimming and require those bytes to be ASCII so the slicing can never
	// panic on a short-after-trim or multi-byte UTF-8 input (reachable from
	// untrusted MCP `since`/`before`/`valid_at` args).
	if s.len() < 19 || !s.as_bytes()[..19].is_ascii() {
		return Err(());
	}
	let year: i32 = s[0..4].parse().map_err(|_| ())?;
	let month: u32 = s[5..7].parse().map_err(|_| ())?;
	let day: u32 = s[8..10].parse().map_err(|_| ())?;
	let hour: u32 = s[11..13].parse().map_err(|_| ())?;
	let min: u32 = s[14..16].parse().map_err(|_| ())?;
	let sec: u32 = s[17..19].parse().map_err(|_| ())?;

	fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
		let y = if m <= 2 { y - 1 } else { y } as i64;
		let m = m as i64;
		let d = d as i64;
		let era = if y >= 0 { y } else { y - 399 } / 400;
		let yoe = y - era * 400;
		let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
		let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
		era * 146097 + doe - 719468
	}

	let days = days_from_civil(year, month, day);
	let secs = days * 86400 + hour as i64 * 3600 + min as i64 * 60 + sec as i64;
	if secs < 0 {
		return Err(());
	}
	Ok(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64))
}

#[cfg(test)]
mod tests {
	use super::parse_rfc3339;

	#[test]
	fn valid_timestamps_parse() {
		assert!(parse_rfc3339("2026-06-05T09:00:00Z").is_ok());
		// 19 chars, no timezone suffix.
		assert!(parse_rfc3339("2026-06-05T09:00:00").is_ok());
		// Surrounding whitespace is trimmed.
		assert!(parse_rfc3339("  2026-06-05T09:00:00Z  ").is_ok());
	}

	#[test]
	fn short_after_trim_is_err_not_panic() {
		// >=20 bytes untrimmed, but trims to far fewer than 19 chars.
		assert_eq!(parse_rfc3339("   2026   "), Err(()));
		assert_eq!(parse_rfc3339("                    "), Err(())); // 20 spaces
		assert_eq!(parse_rfc3339(""), Err(()));
	}

	#[test]
	fn multibyte_in_slice_region_is_err_not_panic() {
		// 'é' (2 bytes) inside the first 19 bytes would put a str slice on a
		// non-char-boundary; must return Err, not panic.
		assert_eq!(parse_rfc3339("20é6-06-05T09:00:00Z"), Err(()));
		// Multibyte right at a split point.
		assert_eq!(parse_rfc3339("2026-06-05T09:00:0😀"), Err(()));
	}

	#[test]
	fn malformed_digits_are_err() {
		assert_eq!(parse_rfc3339("YYYY-06-05T09:00:00Z"), Err(()));
	}

	#[test]
	fn epoch_and_known_instant_compute_correctly() {
		use std::time::{Duration, UNIX_EPOCH};
		assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Ok(UNIX_EPOCH));
		// 2000-01-01T00:00:00Z = 946684800 unix seconds.
		assert_eq!(
			parse_rfc3339("2000-01-01T00:00:00Z"),
			Ok(UNIX_EPOCH + Duration::from_secs(946684800))
		);
	}
}
