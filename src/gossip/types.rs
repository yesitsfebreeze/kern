use serde::{Deserialize, Serialize};

// repr(u8) values are on-wire: append variants, never renumber.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum GossipKind {
	Sphere = 0,
	Question = 1,
	Pulse = 2,
	PeerExchange = 3,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpherePayload {
	pub network_id: String,
	pub kern_id: String,
	// Serialized as Vec<f64> to keep the wire format stable.
	#[serde(with = "crate::base::util::vec_f64_compat")]
	pub anchor_vec: Vec<f32>,
	pub anchor_text: String,
	pub entity_id: String,
	// Cosine distance (1 - cos), smaller = closer.
	pub inner_radius: f64,
	// Invariant: inner_radius <= outer_radius.
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
	ReasonScore = 2,
	ValidUntil = 3,
	Statements = 4,
}

impl CrdtTarget {
	pub fn from_u8(v: u8) -> Option<Self> {
		match v {
			0 => Some(Self::ThoughtAccessCount),
			1 => Some(Self::ReasonTraversalCount),
			2 => Some(Self::ReasonScore),
			3 => Some(Self::ValidUntil),
			4 => Some(Self::Statements),
			_ => None,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySyncPayload {
	pub network_id: String,
	pub kern_id: String,
	pub entities: Vec<crate::base::types::Entity>,
}

// value is the sender's ABSOLUTE replica-slot total, not an increment — a
// delta-since-last would be lost under the receiver's max-merge.
// lamport + producer carry the LWW-Register tiebreak for ReasonScore / ValidUntil.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrdtDeltaPayload {
	pub kern_id: String,
	pub object_id: String,
	pub target: CrdtTarget,
	pub replica: String,
	pub value: u64,
	#[serde(default)]
	pub lamport: u64,
	#[serde(default)]
	pub producer: String,
	// Encoded LWW value for ReasonScore / ValidUntil (bincode of the f64 / Option<SystemTime>).
	#[serde(default)]
	pub lww_value: Vec<u8>,
	// Encoded OR-Set delta for Statements (bincode of Vec<String> adds).
	#[serde(default)]
	pub orset_delta: Vec<u8>,
}
