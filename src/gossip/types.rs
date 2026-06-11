use serde::{Deserialize, Serialize};

/// Coarse wire/routing category for a [`GossipMessage`] — a lightweight tag used
/// for dispatch and metrics. The AUTHORITATIVE message shape is the
/// [`GossipPayload`] variant; `GossipKind` is intentionally coarser and is NOT
/// 1:1 with it: `Fetch` covers BOTH the request (`GossipPayload::FetchRequest`)
/// and the response (`GossipPayload::FetchResult`), because a fetch is a single
/// logical request/response exchange. When you need the exact shape, match on the
/// payload, not the kind. The `repr(u8)` discriminants are the on-wire values and
/// must stay stable (append new variants at the end; never renumber).
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
	/// Id of the kern being advertised.
	pub kern_id: String,
	/// The kern's anchor embedding — the centroid incoming thoughts are routed
	/// against. Same dimensionality as entity vectors; empty for an un-centred
	/// (unnamed) kern, which then matches nothing by similarity.
	pub anchor_vec: Vec<f64>,
	/// Human-readable anchor label (the kern's name).
	pub anchor_text: String,
	/// A representative entity id for the sphere (provenance / dedup handle).
	pub entity_id: String,
	/// Inner routing radius as a COSINE DISTANCE (`1 - cosine_similarity`, so
	/// smaller = closer). A thought within `inner_radius` of `anchor_vec` is firmly
	/// inside this kern.
	pub inner_radius: f64,
	/// Outer routing radius (cosine distance). A thought beyond `outer_radius` is
	/// firmly outside; the band `inner_radius..outer_radius` is the fuzzy
	/// "consider" zone. Invariant: `inner_radius <= outer_radius`.
	pub outer_radius: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionPayload {
	pub reason_id: String,
	pub from_id: String,
	pub reason_vec: Vec<f64>,
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

/// A CRDT counter update shared over gossip.
///
/// `value` is the sender's **absolute total** for its `replica` slot, not an
/// increment-since-last. The receiver max-merges it into the local GCounter, so
/// delivery in any order and with duplicates converges to the same state
/// (commutative + idempotent). Senders must therefore transmit the full slot
/// value; an increment-based value would be lost under the max-merge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrdtDeltaPayload {
	pub kern_id: String,
	pub object_id: String,
	pub target: CrdtTarget,
	pub replica: String,
	pub value: u64,
}
