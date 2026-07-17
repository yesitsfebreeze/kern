use serde::{Deserialize, Serialize};

/// Coarse dispatch tag — the AUTHORITATIVE shape is the [`GossipPayload`] variant
/// (not 1:1). `repr(u8)` values are on-wire: append variants, never renumber.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum GossipKind {
	Sphere = 0,
	Question = 1,
	Pulse = 2,
	PeerExchange = 3,
	/// Both directions of a fetch — `FetchRequest` and `FetchResult` payloads.
	Fetch = 4,
	Delta = 5,
	EntitySync = 6,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GossipMessage {
	pub kind: GossipKind,
	pub id: String,
	pub origin: String,
	pub payload: GossipPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipPayload {
	Sphere(SpherePayload),
	Question(QuestionPayload),
	Pulse(PulsePayload),
	PeerExchange(PeerExchangePayload),
	FetchRequest(FetchPayload),
	FetchResult(FetchResultPayload),
	CrdtDelta(CrdtDeltaPayload),
	EntitySync(EntitySyncPayload),
}

/// Advertises a kern ("sphere") to peers so they can route thoughts toward it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpherePayload {
	/// Federation network this sphere belongs to (gossip is scoped per network).
	pub network_id: String,
	pub kern_id: String,
	/// Anchor centroid thoughts are routed against; empty = un-centred, matches
	/// nothing. Serialized as `Vec<f64>` so the wire format stays stable.
	#[serde(with = "crate::base::util::vec_f64_compat")]
	pub anchor_vec: Vec<f32>,
	pub anchor_text: String,
	/// A representative entity id for the sphere (provenance / dedup handle).
	pub entity_id: String,
	/// Inner routing radius as a COSINE DISTANCE (`1 - cos`, smaller = closer);
	/// within it a thought is firmly inside this kern.
	pub inner_radius: f64,
	/// Beyond it firmly outside; `inner..outer` is the fuzzy "consider" zone.
	/// Invariant: `inner_radius <= outer_radius`.
	pub outer_radius: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionPayload {
	pub reason_id: String,
	pub from_id: String,
	#[serde(with = "crate::base::util::vec_f64_compat")]
	pub reason_vec: Vec<f32>,
	pub question_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PulsePayload {
	pub kern_id: String,
	pub strength: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerExchangePayload {
	pub peers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchPayload {
	pub resource: String,
	pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchResultPayload {
	pub found: bool,
	pub body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum CrdtTarget {
	ThoughtAccessCount = 0,
	ReasonTraversalCount = 1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySyncPayload {
	pub network_id: String,
	pub kern_id: String,
	pub entities: Vec<crate::base::types::Entity>,
}

/// CRDT counter update. `value` is the sender's ABSOLUTE `replica`-slot total —
/// an increment-since-last would be lost under the receiver's max-merge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrdtDeltaPayload {
	pub kern_id: String,
	pub object_id: String,
	pub target: CrdtTarget,
	pub replica: String,
	pub value: u64,
}
