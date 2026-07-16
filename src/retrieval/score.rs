use crate::base::graph::GraphGnn;
use crate::base::heat::{self, HeatConfig};
use crate::base::types::{Entity, EntityKind, EntityStatus};
use crate::base::util::cmp_partial;
use crate::config::RetrievalConfig;
use crate::retrieval::expand::{Scored, ScoredEntity};
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
	/// Bi-temporal WORLD-TIME point query: keep only entities whose
	/// `[valid_from, valid_to)` window covers this instant. Distinct from
	/// `valid_at` (which gates the TTL-expiry `valid_until`): `as_of` asks "what
	/// was true at time T", surfacing the revision that held then.
	pub as_of: Option<SystemTime>,
	/// Include Superseded entities reachable by walking `Supersedes` chains back
	/// from active hits. Not a per-entity filter (the ANN never holds superseded
	/// entities); the retrieval/tool layer does the chain walk and flags them.
	pub include_history: bool,
}

impl QueryOptions {
	/// Whether any metadata filter is set. `sort`/`ascending` are presentation,
	/// not filters, so they are excluded. When this is false, [`matches_filter`]
	/// accepts every entity, so callers can take the cheaper unfiltered ANN path
	/// instead of filtering during traversal.
	pub fn is_active(&self) -> bool {
		!self.source.is_empty()
			|| self.kind.is_some()
			|| self.scheme.is_some()
			|| self.min_conf > 0.0
			|| self.since.is_some()
			|| self.before.is_some()
			|| self.valid_at.is_some()
			|| self.as_of.is_some()
	}
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

pub fn apply_boosts<T: Scored>(cfg: &RetrievalConfig, results: &mut [T]) {
	for r in results.iter_mut() {
		let e = r.entity();
		let confidence = e.score;
		let boost = qbst(cfg, e.access_count.value_i32(), e.accessed_at);
		let fact_bonus = if e.kind == EntityKind::Fact {
			cfg.fact_score_boost
		} else {
			0.0
		};
		r.set_score(r.score() * confidence + boost + fact_bonus);
	}
}

pub fn filter_delivery<T: Scored>(cfg: &RetrievalConfig, results: &mut Vec<T>) {
	results.retain(|r| r.entity().status != EntityStatus::Superseded);
	let floor = cfg.min_deliver_score;
	if results.iter().any(|r| r.score() >= floor) {
		results.retain(|r| r.score() >= floor);
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
	// `as_of`: bi-temporal point query — keep only entities whose world-time
	// window `[valid_from, valid_to)` covers the instant.
	if let Some(as_of) = opts.as_of {
		if !entity.is_valid_at(as_of) {
			return false;
		}
	}
	true
}

pub fn apply_query_options<T: Scored>(results: &mut Vec<T>, opts: &QueryOptions) {
	results.retain(|r| matches_filter(r.entity(), opts));

	// Sort each field ascending, then flip for descending. `dir` keeps the
	// asc/desc branch in one place instead of per-field if/else.
	let asc = opts.ascending;
	let dir = |ord: std::cmp::Ordering| if asc { ord } else { ord.reverse() };
	match opts.sort {
		SortField::Score => {
			results.sort_by(|a, b| dir(cmp_partial(&a.score(), &b.score())));
		}
		SortField::Date => {
			results.sort_by(|a, b| dir(a.entity().created_at.cmp(&b.entity().created_at)));
		}
		SortField::Access => {
			results.sort_by(|a, b| {
				dir(
					a.entity()
						.access_count
						.value()
						.cmp(&b.entity().access_count.value()),
				)
			});
		}
		SortField::Confidence => {
			results.sort_by(|a, b| dir(cmp_partial(&a.entity().score, &b.entity().score)));
		}
	}
}

pub fn commit_access(results: &mut [ScoredEntity]) {
	commit_access_with_half_life(results, HeatConfig::default().half_life_secs);
}

pub fn commit_access_with_half_life(results: &mut [ScoredEntity], half_life_secs: u64) {
	let now = SystemTime::now();
	for r in results.iter_mut() {
		stamp_access(&mut r.entity, now, half_life_secs);
	}
}

/// The single access-stamp: bump the CRDT access counter, set `accessed_at`,
/// and deposit query heat. Shared by [`commit_access`] (which stamps the
/// ephemeral result copies handed back to the caller) and
/// [`commit_access_ids`] (which stamps the live graph entities so the stamp
/// actually persists).
fn stamp_access(e: &mut Entity, now: SystemTime, half_life_secs: u64) {
	let replica = if e.producer_id.is_empty() {
		"local"
	} else {
		e.producer_id.as_str()
	};
	e.access_count.increment(replica, 1);
	e.accessed_at = Some(now);
	e.heat = heat::deposit(
		e.heat,
		e.heat_updated_at,
		now,
		half_life_secs,
		HeatConfig::default().deposit_access,
	);
	e.heat_updated_at = Some(now);
}

pub fn commit_access_ids(g: &mut GraphGnn, ids: &[String]) {
	commit_access_ids_with_half_life(g, ids, HeatConfig::default().half_life_secs);
}

/// Stamp the LIVE graph entities named by `ids` with an access. `commit_access`
/// only mutates the cloned entities inside a query's result set, so without this
/// the hot graph never records an access and the stigmergy GC's `accessed_at`
/// staleness clock never advances (making self-compaction inert on a standalone
/// node). The query path used to do this inline under a brief write lock; it now
/// defers it to a `CommitAccess` tick task so queries take ONLY a read lock. Goes
/// through `kerns` directly — NOT `get_mut` — because an access stamp must not bump
/// the mutation epoch: it would invalidate the whole query cache on every query.
/// Same convention as the heat deposits in `tick::pulse`.
pub fn commit_access_ids_with_half_life(g: &mut GraphGnn, ids: &[String], half_life_secs: u64) {
	let now = SystemTime::now();
	for id in ids {
		let Some(kern_id) = g.kern_of_entity(id).map(str::to_string) else {
			continue;
		};
		if let Some(e) = g.kerns.get_mut(&kern_id).and_then(|k| k.entities.get_mut(id)) {
			stamp_access(e, now, half_life_secs);
		}
	}
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
		assert!(matches_filter(
			&fact_file,
			&QueryOptions {
				kind: Some(EntityKind::Fact),
				..Default::default()
			}
		));
		assert!(!matches_filter(
			&fact_file,
			&QueryOptions {
				kind: Some(EntityKind::Claim),
				..Default::default()
			}
		));
		// Scheme filter.
		assert!(matches_filter(
			&fact_file,
			&QueryOptions {
				scheme: Some("file".into()),
				..Default::default()
			}
		));
		assert!(!matches_filter(
			&fact_file,
			&QueryOptions {
				scheme: Some("ticket".into()),
				..Default::default()
			}
		));
		// Confidence floor (entity.score is 0.5 from the `ent` helper).
		assert!(matches_filter(
			&fact_file,
			&QueryOptions {
				min_conf: 0.4,
				..Default::default()
			}
		));
		assert!(!matches_filter(
			&fact_file,
			&QueryOptions {
				min_conf: 0.6,
				..Default::default()
			}
		));
		// Combined filters must all pass.
		assert!(matches_filter(
			&fact_file,
			&QueryOptions {
				kind: Some(EntityKind::Fact),
				scheme: Some("file".into()),
				min_conf: 0.5,
				..Default::default()
			}
		));
	}

	#[test]
	fn as_of_filters_across_open_and_closed_windows() {
		use std::time::{Duration, UNIX_EPOCH};
		let t = |s| UNIX_EPOCH + Duration::from_secs(s);

		let mut e = ent("a", EntityKind::Fact, file_src("/a")).entity;
		// Open window: valid_from = 100 (via created_at), valid_to = None.
		e.created_at = Some(t(100));

		// Before the window opens -> excluded.
		assert!(!matches_filter(
			&e,
			&QueryOptions {
				as_of: Some(t(50)),
				..Default::default()
			}
		));
		// At/after valid_from with an open upper bound -> included.
		assert!(matches_filter(
			&e,
			&QueryOptions {
				as_of: Some(t(100)),
				..Default::default()
			}
		));
		assert!(matches_filter(
			&e,
			&QueryOptions {
				as_of: Some(t(10_000)),
				..Default::default()
			}
		));

		// Closed window [100, 200): the instant AT valid_to is excluded (half-open).
		e.valid_to = Some(t(200));
		assert!(matches_filter(
			&e,
			&QueryOptions {
				as_of: Some(t(150)),
				..Default::default()
			}
		));
		assert!(
			!matches_filter(
				&e,
				&QueryOptions {
					as_of: Some(t(200)),
					..Default::default()
				}
			),
			"valid_to is exclusive"
		);
		assert!(!matches_filter(
			&e,
			&QueryOptions {
				as_of: Some(t(500)),
				..Default::default()
			}
		));
		// An explicit valid_from hint overrides created_at as the lower bound.
		e.valid_from = Some(t(120));
		assert!(!matches_filter(
			&e,
			&QueryOptions {
				as_of: Some(t(110)),
				..Default::default()
			}
		));
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

	// ---- commit_access_ids ---------------------------------------------------

	#[test]
	fn commit_access_ids_stamps_the_live_entity_without_bumping_the_epoch() {
		use crate::base::types::Kern;
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		k.entities
			.insert("a".into(), ent("a", EntityKind::Claim, file_src("/a")).entity);
		g.kerns.insert("k".into(), k);
		g.index_entity("a", "k");
		let epoch_before = g.mutation_epoch();

		commit_access_ids(&mut g, &["a".to_string()]);

		let live = g.kerns.get("k").unwrap().entities.get("a").unwrap();
		assert!(
			live.accessed_at.is_some(),
			"the LIVE entity gets a persisted accessed_at, not just the result copy"
		);
		assert_eq!(live.access_count.value(), 1, "live access counter bumped");
		assert!(live.heat > 0.0, "query heat deposited on the live entity");
		assert_eq!(
			g.mutation_epoch(),
			epoch_before,
			"access stamps must not invalidate the query cache"
		);
	}

	#[test]
	fn commit_access_ids_skips_ids_unknown_to_the_graph() {
		let mut g = GraphGnn::new();
		commit_access_ids(&mut g, &["ghost".to_string()]);
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
		assert!(
			(got - 3.0).abs() < 0.05,
			"near-zero age -> ~full weight, got {got}"
		);
	}

	#[test]
	fn qbst_clamps_to_cap() {
		let cfg = RetrievalConfig {
			qbst_access_weight: 100.0,
			qbst_recency_weight: 100.0,
			qbst_cap: 2.0,
			..Default::default()
		};
		assert_eq!(
			qbst(&cfg, 1000, Some(SystemTime::now())),
			2.0,
			"clamped to qbst_cap"
		);
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
		assert!(
			(results[0].score - 1.5).abs() < 1e-9,
			"fact got {}",
			results[0].score
		);
		// claim: 2.0*0.5 + 0 + 0 = 1.0 (no fact bonus)
		assert!(
			(results[1].score - 1.0).abs() < 1e-9,
			"claim got {}",
			results[1].score
		);
	}
}
