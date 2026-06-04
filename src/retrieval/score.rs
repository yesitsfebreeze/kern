use crate::base::heat::{self, HeatConfig};
use crate::base::types::{EntityKind, EntityStatus};
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
	/// [`Source::system()`].
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
	results.truncate(cfg.max_deliver_results);
}

pub fn apply_query_options(results: &mut Vec<ScoredEntity>, opts: &QueryOptions) {
	if !opts.source.is_empty() {
		results.retain(|r| r.entity.source.system() == opts.source);
	}
	if let Some(want) = opts.kind {
		results.retain(|r| r.entity.kind == want);
	}
	if let Some(ref want) = opts.scheme {
		results.retain(|r| r.entity.source.scheme() == want.as_str());
	}
	if opts.min_conf > 0.0 {
		results.retain(|r| r.entity.score >= opts.min_conf);
	}
	if let Some(since) = opts.since {
		results.retain(|r| r.entity.created_at.is_none_or(|t| t >= since));
	}
	if let Some(before) = opts.before {
		results.retain(|r| r.entity.created_at.is_none_or(|t| t <= before));
	}
	if let Some(valid_at) = opts.valid_at {
		results.retain(|r| r.entity.valid_until.is_none_or(|exp| exp >= valid_at));
	}

	let asc = opts.ascending;
	match opts.sort {
		SortField::Score => {
			results.sort_by(|a, b| {
				if asc {
					a.score.partial_cmp(&b.score)
				} else {
					b.score.partial_cmp(&a.score)
				}
				.unwrap_or(std::cmp::Ordering::Equal)
			});
		}
		SortField::Date => {
			results.sort_by(|a, b| {
				let ta = a.entity.created_at;
				let tb = b.entity.created_at;
				if asc {
					ta.cmp(&tb)
				} else {
					tb.cmp(&ta)
				}
			});
		}
		SortField::Access => {
			results.sort_by(|a, b| {
				let (av, bv) = (
					a.entity.access_count.value(),
					b.entity.access_count.value(),
				);
				if asc {
					av.cmp(&bv)
				} else {
					bv.cmp(&av)
				}
			});
		}
		SortField::Confidence => {
			results.sort_by(|a, b| {
				let ca = a.entity.score;
				let cb = b.entity.score;
				if asc {
					ca.partial_cmp(&cb)
				} else {
					cb.partial_cmp(&ca)
				}
				.unwrap_or(std::cmp::Ordering::Equal)
			});
		}
	}
}

pub fn commit_access(results: &mut [ScoredEntity]) {
	commit_access_with_half_life(results, HeatConfig::defaults().half_life_secs);
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
			HeatConfig::defaults().deposit_access,
		);
		r.entity.heat_updated_at = Some(now);
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

