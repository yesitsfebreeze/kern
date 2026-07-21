use serde::{Deserialize, Serialize};

// serde/bincode encode the declaration index, so variant ORDER is on-wire;
// reordering is a breaking wire change (alpha: peers upgrade together).
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
	pub graviton_vec: Vec<f32>,
	pub graviton_text: String,
	pub entity_id: String,
	// Cosine distance (1 - cos), smaller = closer.
	pub inner_radius: f64,
	// Invariant: inner_radius <= outer_radius.
	pub outer_radius: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionPayload {
	pub reason_id: String,
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
	pub lamport: u64,
	pub producer: String,
	// Encoded LWW value for ReasonScore / ValidUntil (bincode of the f64 / Option<SystemTime>).
	pub lww_value: Vec<u8>,
	// Encoded OR-Set delta for Statements (bincode of Vec<String> adds).
	pub orset_delta: Vec<u8>,
}
