use crate::base::heat::{self, HeatConfig};
use crate::base::util::cmp_partial;
use crate::base::types::{Entity, EntityKind, EntityStatus};
use crate::config::RetrievalConfig;
use crate::retrieval::expand::ScoredEntity;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortField {
	#[default]
	Score,
	Date,
	Access,
	Confidence,
}

impl SortField {
	pub fn parse(s: &str) -> Self {
		match s.to_lowercase().as_str() {
			"date" => Self::Date,
			"access" => Self::Access,
			"confidence" => Self::Confidence,
			_ => Self::Score,
		}
	}
}

#[derive(Debug, Clone, Default)]
pub struct QueryOptions {
	pub sort: SortField,
	pub ascending: bool,
	/// Legacy free-form source-system filter. Compared against
	/// `Source::system()`.
	pub source: String,
	/// Typed entity-kind filter; `None` disables the filter.
	pub kind: Option<EntityKind>,
	/// URI scheme filter (`"file"` / `"ticket"` / etc); `None` disables.
	pub scheme: Option<String>,
	pub since: Option<SystemTime>,
	pub before: Option<SystemTime>,
	pub min_conf: f64,
	pub valid_at: Option<SystemTime>,
}

pub fn qbst(cfg: &RetrievalConfig, access_count: i32, accessed_at: Option<SystemTime>) -> f64 {
	let access = (access_count as f64 + 1.0).ln() * cfg.qbst_access_weight;
	let recency = match accessed_at {
		Some(at) => {
			let age = SystemTime::now()
				.duration_since(at)
				.unwrap_or_default()
				.as_secs_f64();
			let half_life = Duration::from_secs(cfg.qbst_recency_half_life_secs)
				.as_secs_f64()
				.max(1.0);
			cfg.qbst_recency_weight * (-age / half_life).exp()
		}
		None => 0.0,
	};
	(access + recency).min(cfg.qbst_cap)
}

pub fn apply_boosts(cfg: &RetrievalConfig, results: &mut [ScoredEntity]) {
	for r in results.iter_mut() {
		let confidence = r.entity.score;
		let boost = qbst(
			cfg,
			r.entity.access_count.value_i32(),
			r.entity.accessed_at,
		);
		let fact_bonus = if r.entity.kind == EntityKind::Fact {
			cfg.fact_score_boost
		} else {
			0.0
		};
		r.score = r.score * confidence + boost + fact_bonus;
	}
}

pub fn filter_delivery(cfg: &RetrievalConfig, results: &mut Vec<ScoredEntity>) {
	results.retain(|r| r.entity.status != EntityStatus::Superseded);
	let floor = cfg.min_deliver_score;
	if results.iter().any(|r| r.score >= floor) {
		results.retain(|r| r.score >= floor);
	}
	// When MMR is enabled it diversifies this pool and performs the final cut
	// to `max_deliver_results` itself, so keep a larger candidate pool here —
	// otherwise we would truncate to the delivery cap first and MMR's
	// `len() <= max_deliver_results` guard would make it a no-op (dead code).
	let cap = if cfg.mmr_enabled {
		cfg.mmr_pool_size.max(cfg.max_deliver_results)
	} else {
		cfg.max_deliver_results
	};
	results.truncate(cap);
}

/// Whether `entity` passes the metadata filters in `opts` (source/kind/scheme/
/// confidence/time validity). Sort and `ascending` are presentation, not
/// filters, so they are not considered here. Single source of truth shared by
/// post-filtering ([`apply_query_options`], which trims an already-retrieved
/// result set) and pre-filtered ANN search (a `keep` predicate handed to
/// `search_all_filtered`, which filters during the index traversal).
pub fn matches_filter(entity: &Entity, opts: &QueryOptions) -> bool {
	if !opts.source.is_empty() && entity.source.system() != opts.source {
		return false;
	}
	if let Some(want) = opts.kind {
		if entity.kind != want {
			return false;
		}
	}
	if let Some(ref want) = opts.scheme {
		if entity.source.scheme() != want.as_str() {
			return false;
		}
	}
	if opts.min_conf > 0.0 && entity.score < opts.min_conf {
		return false;
	}
	// `since`/`before` gate on `created_at`; an entity with no timestamp is not
	// excluded by either bound (matches the previous `is_none_or` semantics).
	if let Some(since) = opts.since {
		if entity.created_at.is_some_and(|t| t < since) {
			return false;
		}
	}
	if let Some(before) = opts.before {
		if entity.created_at.is_some_and(|t| t > before) {
			return false;
		}
	}
	// `valid_at`: an entity whose validity has expired before the query instant
	// is filtered out; no expiry means always valid.
	if let Some(valid_at) = opts.valid_at {
		if entity.valid_until.is_some_and(|exp| exp < valid_at) {
			return false;
		}
	}
	true
}

pub fn apply_query_options(results: &mut Vec<ScoredEntity>, opts: &QueryOptions) {
	results.retain(|r| matches_filter(&r.entity, opts));

	// Sort each field ascending, then flip for descending. `dir` keeps the
	// asc/desc branch in one place instead of per-field if/else.
	let asc = opts.ascending;
	let dir = |ord: std::cmp::Ordering| if asc { ord } else { ord.reverse() };
	match opts.sort {
		SortField::Score => {
			results.sort_by(|a, b| dir(cmp_partial(&a.score, &b.score)));
		}
		SortField::Date => {
			results.sort_by(|a, b| dir(a.entity.created_at.cmp(&b.entity.created_at)));
		}
		SortField::Access => {
			results.sort_by(|a, b| {
				dir(a.entity.access_count.value().cmp(&b.entity.access_count.value()))
			});
		}
		SortField::Confidence => {
			results.sort_by(|a, b| dir(cmp_partial(&a.entity.score, &b.entity.score)));
		}
	}
}

pub fn commit_access(results: &mut [ScoredEntity]) {
	commit_access_with_half_life(results, HeatConfig::default().half_life_secs);
}

pub fn commit_access_with_half_life(results: &mut [ScoredEntity], half_life_secs: u64) {
	let now = SystemTime::now();
	for r in results.iter_mut() {
		let replica = if r.entity.producer_id.is_empty() {
			"local"
		} else {
			r.entity.producer_id.as_str()
		};
		r.entity.access_count.increment(replica, 1);
		r.entity.accessed_at = Some(now);
		r.entity.heat = heat::deposit(
			r.entity.heat,
			r.entity.heat_updated_at,
			now,
			half_life_secs,
			HeatConfig::default().deposit_access,
		);
		r.entity.heat_updated_at = Some(now);
	}
}

pub fn softmax(values: &[f64], temperature: f64) -> Vec<f64> {
	let n = values.len();
	if n == 0 {
		return Vec::new();
	}
	if !temperature.is_finite() || temperature <= 0.0 {
		return vec![1.0 / n as f64; n];
	}
	let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
	if !max.is_finite() {
		return vec![1.0 / n as f64; n];
	}
	let mut out: Vec<f64> = values
		.iter()
		.map(|v| ((v - max) / temperature).exp())
		.collect();
	let sum: f64 = out.iter().sum();
	if sum <= 0.0 || !sum.is_finite() {
		return vec![1.0 / n as f64; n];
	}
	for x in out.iter_mut() {
		*x /= sum;
	}
	out
}

#[cfg(test)]
mod query_filter_tests {
	use super::*;
	use crate::base::types::{Entity, Source};

	fn ent(id: &str, kind: EntityKind, src: Source) -> ScoredEntity {
		ScoredEntity {
			entity: Entity {
				id: id.into(),
				kind,
				source: src,
				score: 0.5,
				..Default::default()
			},
			score: 1.0,
		}
	}

	fn file_src(path: &str) -> Source {
		Source::File {
			path: path.into(),
			section: String::new(),
			title: String::new(),
			author: String::new(),
			url: String::new(),
		}
	}

	fn ticket_src(id: &str) -> Source {
		Source::Ticket {
			system: "github".into(),
			object_id: id.into(),
			section: String::new(),
			title: String::new(),
			author: String::new(),
			url: String::new(),
		}
	}

	#[test]
	fn query_filter_by_kind_retains_only_matching() {
		let mut results = vec![
			ent("a", EntityKind::Fact, file_src("/a")),
			ent("b", EntityKind::Claim, file_src("/b")),
			ent("c", EntityKind::Question, ticket_src("123")),
		];
		let opts = QueryOptions {
			kind: Some(EntityKind::Fact),
			..QueryOptions::default()
		};
		apply_query_options(&mut results, &opts);
		assert_eq!(results.len(), 1);
		assert_eq!(results[0].entity.id, "a");
	}

	#[test]
	fn query_filter_by_scheme_retains_only_matching() {
		let mut results = vec![
			ent("a", EntityKind::Fact, file_src("/a")),
			ent("b", EntityKind::Claim, ticket_src("42")),
			ent("c", EntityKind::Document, file_src("/c")),
		];
		let opts = QueryOptions {
			scheme: Some("file".into()),
			..QueryOptions::default()
		};
		apply_query_options(&mut results, &opts);
		assert_eq!(results.len(), 2);
		assert!(results.iter().all(|r| r.entity.source.scheme() == "file"));
	}

	#[test]
	fn matches_filter_is_the_per_entity_predicate() {
		let fact_file = ent("a", EntityKind::Fact, file_src("/a")).entity;
		// Default (no filter) matches anything.
		assert!(matches_filter(&fact_file, &QueryOptions::default()));
		// Kind filter.
		assert!(matches_filter(&fact_file, &QueryOptions { kind: Some(EntityKind::Fact), ..Default::default() }));
		assert!(!matches_filter(&fact_file, &QueryOptions { kind: Some(EntityKind::Claim), ..Default::default() }));
		// Scheme filter.
		assert!(matches_filter(&fact_file, &QueryOptions { scheme: Some("file".into()), ..Default::default() }));
		assert!(!matches_filter(&fact_file, &QueryOptions { scheme: Some("ticket".into()), ..Default::default() }));
		// Confidence floor (entity.score is 0.5 from the `ent` helper).
		assert!(matches_filter(&fact_file, &QueryOptions { min_conf: 0.4, ..Default::default() }));
		assert!(!matches_filter(&fact_file, &QueryOptions { min_conf: 0.6, ..Default::default() }));
		// Combined filters must all pass.
		assert!(matches_filter(&fact_file, &QueryOptions {
			kind: Some(EntityKind::Fact),
			scheme: Some("file".into()),
			min_conf: 0.5,
			..Default::default()
		}));
	}

	#[test]
	fn filter_delivery_keeps_mmr_pool_when_mmr_enabled() {
		// Regression: previously truncated straight to max_deliver_results,
		// which made MMR's len-guard a no-op. With MMR on, keep the pool so
		// MMR has candidates to diversify and does the final cut itself.
		let cfg = RetrievalConfig::default(); // mmr on, pool 50, cap 25
		let mut results: Vec<ScoredEntity> = (0..60)
			.map(|i| ent(&format!("e{i}"), EntityKind::Fact, file_src("/x")))
			.collect();
		filter_delivery(&cfg, &mut results);
		assert_eq!(results.len(), cfg.mmr_pool_size);
	}

	#[test]
	fn filter_delivery_cuts_to_cap_when_mmr_disabled() {
		let cfg = RetrievalConfig {
			mmr_enabled: false,
			..Default::default()
		};
		let mut results: Vec<ScoredEntity> = (0..60)
			.map(|i| ent(&format!("e{i}"), EntityKind::Fact, file_src("/x")))
			.collect();
		filter_delivery(&cfg, &mut results);
		assert_eq!(results.len(), cfg.max_deliver_results);
	}

	// ---- qbst / apply_boosts -----------------------------------------------

	#[test]
	fn qbst_zero_access_and_no_recency_is_zero() {
		let cfg = RetrievalConfig::default();
		// ln(0+1)=0 access component; accessed_at None -> 0 recency.
		assert_eq!(qbst(&cfg, 0, None), 0.0);
	}

	#[test]
	fn qbst_access_component_follows_log_count_times_weight() {
		let cfg = RetrievalConfig {
			qbst_access_weight: 1.5,
			qbst_recency_weight: 0.0, // isolate the access term
			qbst_cap: 1e9,            // don't clamp
			..Default::default()
		};
		let got = qbst(&cfg, 9, None);
		let expected = (9.0_f64 + 1.0).ln() * 1.5;
		assert!((got - expected).abs() < 1e-9, "got {got}, want {expected}");
	}

	#[test]
	fn qbst_recency_is_near_full_weight_at_zero_age() {
		let cfg = RetrievalConfig {
			qbst_access_weight: 0.0, // isolate the recency term
			qbst_recency_weight: 3.0,
			qbst_cap: 1e9,
			..Default::default()
		};
		// age ~0 -> exp(0) ~ 1 -> ~ full recency weight.
		let got = qbst(&cfg, 0, Some(SystemTime::now()));
		assert!((got - 3.0).abs() < 0.05, "near-zero age -> ~full weight, got {got}");
	}

	#[test]
	fn qbst_clamps_to_cap() {
		let cfg = RetrievalConfig {
			qbst_access_weight: 100.0,
			qbst_recency_weight: 100.0,
			qbst_cap: 2.0,
			..Default::default()
		};
		assert_eq!(qbst(&cfg, 1000, Some(SystemTime::now())), 2.0, "clamped to qbst_cap");
	}

	#[test]
	fn apply_boosts_scales_by_confidence_and_adds_fact_bonus_only_for_facts() {
		let cfg = RetrievalConfig {
			qbst_access_weight: 0.0, // boost = 0 so the arithmetic is exact
			qbst_recency_weight: 0.0,
			fact_score_boost: 0.5,
			..Default::default()
		};
		let mut fact = ent("f", EntityKind::Fact, file_src("/f"));
		fact.score = 2.0; // raw retrieval score
		fact.entity.score = 0.5; // confidence
		let mut claim = ent("c", EntityKind::Claim, file_src("/c"));
		claim.score = 2.0;
		claim.entity.score = 0.5;
		let mut results = vec![fact, claim];
		apply_boosts(&cfg, &mut results);
		// fact: 2.0*0.5 + 0(boost) + 0.5(fact bonus) = 1.5
		assert!((results[0].score - 1.5).abs() < 1e-9, "fact got {}", results[0].score);
		// claim: 2.0*0.5 + 0 + 0 = 1.0 (no fact bonus)
		assert!((results[1].score - 1.0).abs() < 1e-9, "claim got {}", results[1].score);
	}

	// ---- softmax edge cases ------------------------------------------------

	#[test]
	fn softmax_zero_or_invalid_temperature_is_uniform() {
		assert_eq!(softmax(&[1.0, 2.0, 3.0], 0.0), vec![1.0 / 3.0; 3]);
		assert_eq!(softmax(&[1.0, 2.0], -1.0), vec![0.5, 0.5]);
		assert_eq!(softmax(&[1.0, 2.0], f64::NAN), vec![0.5, 0.5]);
	}

	#[test]
	fn softmax_non_finite_values_fall_back_to_uniform() {
		// +inf makes `max` non-finite -> uniform.
		assert_eq!(softmax(&[1.0, f64::INFINITY, 2.0], 1.0), vec![1.0 / 3.0; 3]);
		// A NaN value leaves max finite (f64::max skips NaN) but the exp/sum goes
		// NaN -> non-finite sum -> uniform.
		assert_eq!(softmax(&[1.0, f64::NAN, 2.0], 1.0), vec![1.0 / 3.0; 3]);
	}

	#[test]
	fn softmax_empty_input_is_empty() {
		assert!(softmax(&[], 1.0).is_empty());
	}

	#[test]
	fn softmax_normal_case_sums_to_one_and_is_monotonic() {
		let out = softmax(&[1.0, 2.0, 3.0], 1.0);
		let sum: f64 = out.iter().sum();
		assert!((sum - 1.0).abs() < 1e-9, "probabilities sum to 1");
		assert!(out[2] > out[1] && out[1] > out[0], "higher logit -> higher prob");
	}
}

