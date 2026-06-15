use crate::base::constants::{
	INGEST_DEDUP_THRESHOLD, INGEST_HNSW_EF, INGEST_HNSW_K, INGEST_REPHRASE_LOWER,
	INGEST_REPHRASE_UPPER,
};

/// Runtime ingest knobs threaded through the [`Worker`](crate::ingest::Worker)
/// pipeline. Distinct from the serde-deserialized
/// [`IngestConfig`](crate::config::IngestConfig): this runtime form carries
/// `ttl_secs` and drops the serde-only `session_mirror_max_seen`. Both share the
/// same default values via the `INGEST_*` constants in `base::constants`.
#[derive(Debug, Clone)]
pub struct Config {
	/// Cosine-similarity floor in `[0.0, 1.0]`: a new vector whose nearest
	/// neighbour scores at or above this is treated as a duplicate and merged
	/// instead of inserted. Higher → fewer merges (stricter "same thought").
	pub dedup_threshold: f64,
	/// Optional time-to-live, in seconds, applied to ingested entities. `None`
	/// means no expiry (the default).
	pub ttl_secs: Option<u64>,
	/// Nearest-neighbour count (`k`) for the synthesis/rephrase HNSW probe — how
	/// many existing entities are considered as rephrase candidates per ingest.
	pub hnsw_k: usize,
	/// HNSW search beam width (`ef`) for that probe. Wider → better recall of the
	/// true nearest neighbours at higher search cost. Must be `>= hnsw_k`.
	pub hnsw_ef: usize,
	/// Lower edge of the rephrase similarity band in `[0.0, 1.0]`: a candidate at
	/// or below this is too dissimilar to merge and stays a separate entity.
	pub rephrase_lower: f64,
	/// Upper edge of the rephrase band: a candidate at or above this is a
	/// near-duplicate (handled by dedup). Only candidates STRICTLY between
	/// `rephrase_lower` and `rephrase_upper` are rephrased/merged.
	pub rephrase_upper: f64,
}

impl Default for Config {
	fn default() -> Self {
		Self {
			dedup_threshold: INGEST_DEDUP_THRESHOLD,
			ttl_secs: None,
			hnsw_k: INGEST_HNSW_K,
			hnsw_ef: INGEST_HNSW_EF,
			rephrase_lower: INGEST_REPHRASE_LOWER,
			rephrase_upper: INGEST_REPHRASE_UPPER,
		}
	}
}

impl Config {
	/// Reject an out-of-range configuration at construction time rather than
	/// letting a bad knob surface as silently-wrong behaviour deep in ingest.
	/// Similarity thresholds must lie in `[0.0, 1.0]`, the rephrase band must be a
	/// real interval (`lower < upper`), and the HNSW probe must request at least
	/// one neighbour with a beam (`ef`) at least as wide as `k`.
	pub fn validate(&self) -> Result<(), String> {
		for (name, v) in [
			("dedup_threshold", self.dedup_threshold),
			("rephrase_lower", self.rephrase_lower),
			("rephrase_upper", self.rephrase_upper),
		] {
			if !(0.0..=1.0).contains(&v) {
				return Err(format!("{name} must be in [0.0, 1.0], got {v}"));
			}
		}
		if self.rephrase_lower >= self.rephrase_upper {
			return Err(format!(
				"rephrase_lower ({}) must be < rephrase_upper ({})",
				self.rephrase_lower, self.rephrase_upper
			));
		}
		if self.hnsw_k == 0 {
			return Err("hnsw_k must be >= 1".into());
		}
		if self.hnsw_ef < self.hnsw_k {
			return Err(format!(
				"hnsw_ef ({}) must be >= hnsw_k ({})",
				self.hnsw_ef, self.hnsw_k
			));
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The runtime `Config` and the serde `IngestConfig` describe the same knobs
	/// at two layers; their defaults must agree. Both now source the shared
	/// `INGEST_*` constants, so this guards against a future edit re-introducing a
	/// divergent literal in one layer.
	#[test]
	fn runtime_and_serde_ingest_defaults_agree() {
		let rt = Config::default();
		let serde = crate::config::IngestConfig::default();
		assert_eq!(rt.dedup_threshold, serde.dedup_threshold);
		assert_eq!(rt.hnsw_k, serde.hnsw_k);
		assert_eq!(rt.hnsw_ef, serde.hnsw_ef);
		assert_eq!(rt.rephrase_lower, serde.rephrase_lower);
		assert_eq!(rt.rephrase_upper, serde.rephrase_upper);
		// And both trace back to the shared constants.
		assert_eq!(rt.dedup_threshold, INGEST_DEDUP_THRESHOLD);
		assert_eq!(rt.rephrase_upper, INGEST_REPHRASE_UPPER);
	}

	/// The rephrase band must be a real interval and the dedup floor must sit at
	/// or above the band's upper edge, or the "dedup vs rephrase" handoff inverts.
	#[test]
	fn rephrase_band_is_ordered_and_dedup_caps_it() {
		let c = Config::default();
		assert!(c.rephrase_lower < c.rephrase_upper, "band lower < upper");
		assert!(
			c.dedup_threshold >= c.rephrase_upper,
			"dedup floor caps the rephrase band"
		);
		assert!(c.hnsw_ef >= c.hnsw_k, "ef beam must be at least k");
	}

	#[test]
	fn validate_accepts_the_default_and_rejects_bad_knobs() {
		assert!(
			Config::default().validate().is_ok(),
			"default config is valid"
		);

		let out_of_range = Config {
			dedup_threshold: 1.5,
			..Default::default()
		};
		assert!(out_of_range
			.validate()
			.unwrap_err()
			.contains("dedup_threshold"));

		let inverted = Config {
			rephrase_lower: 0.9,
			rephrase_upper: 0.8,
			..Default::default()
		};
		assert!(inverted.validate().unwrap_err().contains("rephrase_lower"));

		let zero_k = Config {
			hnsw_k: 0,
			..Default::default()
		};
		assert!(zero_k.validate().unwrap_err().contains("hnsw_k"));

		let narrow_beam = Config {
			hnsw_k: 16,
			hnsw_ef: 8,
			..Default::default()
		};
		assert!(narrow_beam.validate().unwrap_err().contains("hnsw_ef"));
	}
}
