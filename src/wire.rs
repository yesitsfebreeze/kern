use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::base::constants::{AGENT_SOURCE, USER_SOURCE};
use crate::base::types::EntityKind;

pub const VERSION: &str = "1";

pub const WIRE_CONF_MIN: f64 = 0.0;
pub const WIRE_CONF_MAX: f64 = 1.0;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum WireError {
	#[error("conf {0} out of range [0.0..=1.0]")]
	ConfOutOfRange(f64),
	#[error("thought kind {0:?} is internal-only and not accepted on the wire")]
	InternalKindOnWire(EntityKind),
	#[error("fact-tier conf requires trusted source (got source={0:?})")]
	FactFromUntrustedSource(String),
}

pub fn validate_wire_conf(conf: f64) -> Result<f64, WireError> {
	if conf.is_nan() || !(WIRE_CONF_MIN..=WIRE_CONF_MAX).contains(&conf) {
		return Err(WireError::ConfOutOfRange(conf));
	}
	Ok(conf)
}

pub fn validate_wire_kind(kind: EntityKind) -> Result<EntityKind, WireError> {
	match kind {
		EntityKind::Claim | EntityKind::Fact => Ok(kind),
		EntityKind::Document | EntityKind::Question | EntityKind::Answer | EntityKind::Conclusion => {
			Err(WireError::InternalKindOnWire(kind))
		}
	}
}

pub fn validate_fact_source(source: &str) -> Result<(), WireError> {
	if source == USER_SOURCE || source == AGENT_SOURCE {
		Ok(())
	} else {
		Err(WireError::FactFromUntrustedSource(source.to_string()))
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
	pub version: String,
	pub id: String,
	pub op: String,
	pub body: Vec<u8>,
	#[serde(default)]
	pub error: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryRequest {
	pub text: String,
	#[serde(default)]
	pub mode: String,
	#[serde(default)]
	pub k: i32,
	#[serde(default)]
	pub answer: bool,
	#[serde(default)]
	pub sort: String,
	#[serde(default)]
	pub ascending: bool,
	#[serde(default)]
	pub source: String,
	#[serde(default)]
	pub kind: String,
	#[serde(default)]
	pub since: String,
	#[serde(default)]
	pub before: String,
	#[serde(default)]
	pub min_conf: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryEntity {
	pub id: String,
	pub score: f64,
	pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryChain {
	pub nodes: Vec<String>,
	pub score: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryResponse {
	pub entities: Vec<QueryEntity>,
	#[serde(default)]
	pub answer: String,
	#[serde(default)]
	pub chains: Vec<QueryChain>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestFailure {
	pub scope: String,
	#[serde(default, skip_serializing_if = "is_zero")]
	pub chunk_index: i32,
	pub class: String,
	pub error: String,
}

fn is_zero<T: Default + PartialEq>(v: &T) -> bool {
	*v == T::default()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngestResponse {
	pub doc_id: String,
	pub status: String,
	pub conf: f64,
	pub kind: i32,
	#[serde(default, skip_serializing_if = "is_zero")]
	pub total_chunks: i32,
	#[serde(default, skip_serializing_if = "is_zero")]
	pub embedded_chunks: i32,
	#[serde(default, skip_serializing_if = "is_zero")]
	pub failed_chunks: i32,
	#[serde(default, skip_serializing_if = "is_zero")]
	pub transient_failures: i32,
	#[serde(default, skip_serializing_if = "is_zero")]
	pub permanent_failures: i32,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub failures: Vec<IngestFailure>,
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HealthResponse {
	pub kerns: i32,
	pub entities: i32,
	pub reasons: i32,
	pub unnamed: i32,
	pub descriptors: i32,
	pub queue_depth: i32,
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub data_dir: String,
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub core_addr: String,
	#[serde(default, skip_serializing_if = "is_zero")]
	pub query_count: i64,
	#[serde(default, skip_serializing_if = "is_zero")]
	pub query_latency_ms_avg: i64,
	#[serde(default, skip_serializing_if = "is_zero")]
	pub query_path_depth_avg: i64,
	#[serde(default, skip_serializing_if = "is_zero")]
	pub query_path_depth_max: i64,
	#[serde(default, skip_serializing_if = "is_zero")]
	pub ingest_committed: i64,
	#[serde(default, skip_serializing_if = "is_zero")]
	pub ingest_partial: i64,
	#[serde(default, skip_serializing_if = "is_zero")]
	pub ingest_failed: i64,
	#[serde(default, skip_serializing_if = "is_zero")]
	pub ingest_chunk_failures: i64,
	#[serde(default, skip_serializing_if = "is_zero")]
	pub task_count: i64,
	#[serde(default, skip_serializing_if = "is_zero")]
	pub task_latency_ms_avg: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetRequest {
	pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasonEdge {
	pub id: String,
	pub from: String,
	pub to: String,
	pub kind: i32,
	pub text: String,
	pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityDetail {
	pub id: String,
	pub kind: i32,
	pub text: String,
	pub score: f64,
	pub access_count: i32,
	pub kern: String,
	pub edges: Vec<ReasonEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetResponse {
	pub entity: EntityDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkRequest {
	pub from: String,
	pub to: String,
	#[serde(default)]
	pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkResponse {
	pub edge: ReasonEdge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgetRequest {
	pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgetResponse {
	pub removed: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DegradeRequest {
	pub query_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DegradeResponse {
	pub ok: bool,
	pub decayed_edges: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PulseRequest {
	pub kern_id: String,
	pub strength: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PulseResponse {
	pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescriptorRequest {
	pub action: String,
	pub name: String,
	#[serde(default)]
	pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescriptorResponse {
	pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GravitonRequest {
	pub action: String,
	pub name: String,
	pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GravitonResponse {
	pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetReasonRequest {
	pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasonDetail {
	pub id: String,
	pub from: String,
	pub to: String,
	pub kind: i32,
	pub text: String,
	pub score: f64,
	pub traversal_count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetReasonResponse {
	pub reason: ReasonDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEntity {
	pub id: String,
	pub kind: i32,
	pub kern_id: String,
	pub text: String,
	pub vector: Vec<f64>,
	pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotReason {
	pub id: String,
	pub from: String,
	pub to: String,
	pub kind: i32,
	pub text: String,
	pub vector: Vec<f64>,
	pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotKern {
	pub id: String,
	pub graviton_text: String,
	pub graviton_vec: Vec<f64>,
	pub inner_radius: f64,
	pub outer_radius: f64,
	pub parent: String,
	pub children: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnapshotResponse {
	pub entities: Vec<SnapshotEntity>,
	pub reasons: Vec<SnapshotReason>,
	pub kerns: Vec<SnapshotKern>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityListItem {
	pub id: String,
	pub score: f64,
	pub text: String,
	pub kern: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListEntitiesResponse {
	pub entities: Vec<EntityListItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernListItem {
	pub id: String,
	pub entities: i32,
	pub reasons: i32,
	pub children: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListKernsResponse {
	pub kerns: Vec<KernListItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListDescriptorsResponse {
	pub descriptors: HashMap<String, String>,
}

#[cfg(test)]
mod wire_validation_tests {
	use super::*;

	#[test]
	fn conf_out_of_range_rejected_high() {
		assert!(matches!(
			validate_wire_conf(1.5),
			Err(WireError::ConfOutOfRange(_))
		));
	}

	#[test]
	fn conf_out_of_range_rejected_low() {
		assert!(matches!(
			validate_wire_conf(-0.01),
			Err(WireError::ConfOutOfRange(_))
		));
	}

	#[test]
	fn conf_out_of_range_rejected_nan() {
		assert!(matches!(
			validate_wire_conf(f64::NAN),
			Err(WireError::ConfOutOfRange(_))
		));
	}

	#[test]
	fn document_on_wire_rejected() {
		assert_eq!(
			validate_wire_kind(EntityKind::Document),
			Err(WireError::InternalKindOnWire(EntityKind::Document))
		);
	}

	#[test]
	fn question_on_wire_rejected() {
		assert_eq!(
			validate_wire_kind(EntityKind::Question),
			Err(WireError::InternalKindOnWire(EntityKind::Question))
		);
	}

	#[test]
	fn conf_inclusive_bounds_accepted() {
		assert_eq!(validate_wire_conf(0.0), Ok(0.0));
		assert_eq!(validate_wire_conf(1.0), Ok(1.0));
		assert_eq!(validate_wire_conf(0.5), Ok(0.5));
	}

	#[test]
	fn normal_on_wire_allowed() {
		assert_eq!(validate_wire_kind(EntityKind::Claim), Ok(EntityKind::Claim));
		assert_eq!(validate_wire_kind(EntityKind::Fact), Ok(EntityKind::Fact));
	}

	#[test]
	fn answer_and_conclusion_on_wire_rejected() {
		assert_eq!(
			validate_wire_kind(EntityKind::Answer),
			Err(WireError::InternalKindOnWire(EntityKind::Answer))
		);
		assert_eq!(
			validate_wire_kind(EntityKind::Conclusion),
			Err(WireError::InternalKindOnWire(EntityKind::Conclusion))
		);
	}

	#[test]
	fn is_zero_generic_matches_defaults_for_both_int_widths() {
		assert!(is_zero(&0_i32));
		assert!(is_zero(&0_i64));
		assert!(!is_zero(&1_i32));
		assert!(!is_zero(&-1_i64));
	}

	#[test]
	fn fact_source_rejects_untrusted() {
		assert!(matches!(
			validate_fact_source("stranger"),
			Err(WireError::FactFromUntrustedSource(_))
		));
	}

	#[test]
	fn fact_source_allows_trusted() {
		assert!(validate_fact_source(AGENT_SOURCE).is_ok());
		assert!(validate_fact_source(USER_SOURCE).is_ok());
	}
}
