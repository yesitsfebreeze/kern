use std::time::SystemTime;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct HeatConfig {
	/// Heat half-life in **seconds**: the elapsed time over which an entity's
	/// heat decays to half via [`decayed`]. Default one week.
	pub half_life_secs: u64,
	/// Heat added per access (an entity being read/retrieved). A **dimensionless
	/// heat unit**, not a ratio or duration — it is summed onto current heat by
	/// [`deposit`], then decays over `half_life_secs`.
	pub deposit_access: f32,
	/// Heat added per graph traversal that passes through an entity. Same
	/// dimensionless heat unit as `deposit_access`; lower by default because a
	/// pass-through is weaker evidence of relevance than a direct access.
	pub deposit_traversal: f32,
}

impl Default for HeatConfig {
	fn default() -> Self {
		Self {
			half_life_secs: 7 * 24 * 60 * 60,
			deposit_access: 1.0,
			deposit_traversal: 0.5,
		}
	}
}

pub fn decayed(heat: f32, since: Option<SystemTime>, now: SystemTime, half_life_secs: u64) -> f32 {
	if heat <= 0.0 {
		return 0.0;
	}
	let Some(since) = since else {
		return heat;
	};
	let dt = match now.duration_since(since) {
		Ok(d) => d.as_secs_f64(),
		Err(_) => return heat,
	};
	let t = (half_life_secs as f64).max(1.0);
	let lambda = std::f64::consts::LN_2 / t;
	(heat as f64 * (-lambda * dt).exp()) as f32
}

pub fn deposit(
	heat: f32,
	since: Option<SystemTime>,
	now: SystemTime,
	half_life_secs: u64,
	deposit: f32,
) -> f32 {
	decayed(heat, since, now, half_life_secs) + deposit
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::time::Duration;

	const HL: u64 = 100; // 100-second half-life for readable arithmetic.

	#[test]
	fn decayed_zero_or_negative_heat_is_zero() {
		let now = SystemTime::now();
		assert_eq!(decayed(0.0, Some(now), now, HL), 0.0);
		assert_eq!(decayed(-5.0, Some(now), now, HL), 0.0, "guard clamps non-positive heat");
	}

	#[test]
	fn decayed_none_since_returns_heat_unchanged() {
		// No timestamp to decay from -> heat passes through untouched.
		assert_eq!(decayed(3.0, None, SystemTime::now(), HL), 3.0);
	}

	#[test]
	fn decayed_clock_skew_returns_heat_unchanged() {
		// `since` is in the FUTURE relative to `now` -> duration_since is Err;
		// rather than extrapolate, heat is returned as-is.
		let now = SystemTime::now();
		let since = now + Duration::from_secs(60);
		assert_eq!(decayed(4.0, Some(since), now, HL), 4.0);
	}

	#[test]
	fn decayed_one_half_life_halves_the_heat() {
		let since = SystemTime::UNIX_EPOCH;
		let now = since + Duration::from_secs(HL); // exactly one half-life elapsed
		let got = decayed(8.0, Some(since), now, HL);
		assert!((got - 4.0).abs() < 1e-4, "one half-life halves 8 -> ~4, got {got}");
		// Two half-lives -> a quarter.
		let now2 = since + Duration::from_secs(2 * HL);
		let got2 = decayed(8.0, Some(since), now2, HL);
		assert!((got2 - 2.0).abs() < 1e-4, "two half-lives -> ~2, got {got2}");
	}

	#[test]
	fn decayed_zero_half_life_is_clamped_to_one_second() {
		// half_life_secs 0 would divide by zero; the .max(1.0) guard avoids NaN
		// and just decays very fast over a 1s effective half-life.
		let since = SystemTime::UNIX_EPOCH;
		let now = since + Duration::from_secs(10);
		let got = decayed(8.0, Some(since), now, 0);
		assert!(got.is_finite() && got >= 0.0, "no NaN/inf for zero half-life, got {got}");
		assert!(got < 0.01, "10s over a clamped 1s half-life decays heavily, got {got}");
	}

	#[test]
	fn deposit_adds_on_top_of_the_decayed_value() {
		let since = SystemTime::UNIX_EPOCH;
		let now = since + Duration::from_secs(HL); // 8 -> ~4 after decay
		let got = deposit(8.0, Some(since), now, HL, 1.5);
		assert!((got - 5.5).abs() < 1e-4, "decayed (~4) + deposit (1.5) = ~5.5, got {got}");
	}

	#[test]
	fn config_default_is_a_one_week_half_life() {
		let c = HeatConfig::default();
		assert_eq!(c.half_life_secs, 7 * 24 * 60 * 60);
		assert_eq!(c.deposit_access, 1.0);
		assert_eq!(c.deposit_traversal, 0.5);
	}
}
