use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::RwLock;

use crate::base::constants::*;
use crate::base::graph::GraphGnn;
use crate::base::locks::{read_recovered, write_recovered};
use crate::base::search::search_all_unlocked;
use crate::base::types::{Kern, ReasonKind};
use crate::crdt::GCounter;
use crate::tick;

use super::node::{Handler, Node};
use super::types::*;

pub struct Deps {
	pub graph: Arc<RwLock<GraphGnn>>,
	pub node: Arc<Node>,
	pub queue: Option<Arc<tick::queue::Queue>>,
	// Every federation mutation must call this or federated knowledge is lost on restart.
	pub save: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
}

impl Deps {
	fn persist(&self) {
		if let Some(s) = &self.save {
			s();
		}
	}
}

pub fn new_handler(d: Arc<Deps>) -> Handler {
	Arc::new(move |msg: GossipMessage| match msg.kind {
		GossipKind::Sphere => {
			if msg.id.starts_with("answer-") {
				handle_answer(&d, msg);
			} else {
				handle_sphere(&d, msg);
			}
		}
		GossipKind::Question => handle_question(&d, msg),
		GossipKind::Pulse => handle_pulse(&d, msg),
		GossipKind::PeerExchange => handle_peer_exchange(&d, msg),
		GossipKind::Fetch => {}
		GossipKind::Delta => handle_crdt_delta(&d, msg),
		GossipKind::EntitySync => handle_entity_sync(&d, msg),
	})
}

pub fn start_announce(node: Arc<Node>, graph: Arc<RwLock<GraphGnn>>) {
	let mut stop = node.stop_rx.clone();
	tokio::spawn(async move {
		let mut interval = tokio::time::interval(GOSSIP_HEARTBEAT_INTERVAL);
		loop {
			tokio::select! {
				_ = interval.tick() => {
					let payload = {
						let g = read_recovered(&graph);
						if g.root.anchor_vec.is_empty() {
							None
						} else {
							Some(SpherePayload {
								network_id: g.network_id.clone(),
								kern_id: g.root.id.clone(),
								anchor_text: g.root.anchor_text.clone(),
								anchor_vec: g.root.anchor_vec.clone(),
								entity_id: String::new(),
								inner_radius: g.root.inner_radius,
								outer_radius: g.root.outer_radius,
							})
						}
					};
					if let Some(payload) = payload {
						let stamp = crate::base::util::now_nanos();
						let msg = GossipMessage {
							kind: GossipKind::Sphere,
							id: format!("sphere-{}-{}", node.addr(), stamp),
							origin: node.addr(),
							payload: GossipPayload::Sphere(payload),
						};
						node.broadcast(msg);
					}
				}
				_ = stop.changed() => break,
			}
		}
	});
}

pub fn start_entity_sync(node: Arc<Node>, graph: Arc<RwLock<GraphGnn>>) {
	let mut stop = node.stop_rx.clone();
	tokio::spawn(async move {
		let mut interval = tokio::time::interval(GOSSIP_HEARTBEAT_INTERVAL);
		loop {
			tokio::select! {
				_ = interval.tick() => {
					let payload = {
						let g = read_recovered(&graph);
						let mut entities: Vec<crate::base::types::Entity> = g
							.kerns
							.iter()
							.filter(|(kid, _)| !kid.starts_with("remote-"))
							.flat_map(|(_, k)| k.entities.values().cloned())
							.collect();
						if entities.is_empty() {
							None
						} else {
							entities.sort_by(|a, b| crate::base::util::cmp_rank(a.heat as f64, &a.id, b.heat as f64, &b.id));
							entities.truncate(32);
							Some(EntitySyncPayload {
								network_id: g.network_id.clone(),
								kern_id: g.root.id.clone(),
								entities,
							})
						}
					};
					if let Some(payload) = payload {
						let stamp = crate::base::util::now_nanos();
						let msg = GossipMessage {
							kind: GossipKind::EntitySync,
							id: format!("esync-{}-{}", node.addr(), stamp),
							origin: node.addr(),
							payload: GossipPayload::EntitySync(payload),
						};
						node.broadcast(msg);
					}
				}
				_ = stop.changed() => break,
			}
		}
	});
}

pub fn start_delta_flush(node: Arc<Node>, graph: Arc<RwLock<GraphGnn>>) {
	let mut stop = node.stop_rx.clone();
	tokio::spawn(async move {
		let mut interval = tokio::time::interval(GOSSIP_HEARTBEAT_INTERVAL);
		loop {
			tokio::select! {
				_ = interval.tick() => {
					let (deltas, kern_id) = {
						let g = read_recovered(&graph);
						(g.drain_pending_deltas(), g.root.id.clone())
					};
					for delta in deltas {
						let target = match CrdtTarget::from_u8(delta.target) {
							Some(t) => t,
							None => continue,
						};
						let payload = CrdtDeltaPayload {
							kern_id: kern_id.clone(),
							object_id: delta.object_id,
							target,
							replica: delta.replica,
							value: delta.value,
							lamport: delta.lamport,
							producer: delta.producer,
							lww_value: delta.lww_value,
							orset_delta: Vec::new(),
						};
						let stamp = crate::base::util::now_nanos();
						let msg = GossipMessage {
							kind: GossipKind::Delta,
							id: format!("delta-{}-{}", node.addr(), stamp),
							origin: node.addr(),
							payload: GossipPayload::CrdtDelta(payload),
						};
						node.broadcast(msg);
					}
				}
				_ = stop.changed() => break,
			}
		}
	});
}

fn handle_sphere(d: &Deps, msg: GossipMessage) {
	let sphere = match &msg.payload {
		GossipPayload::Sphere(s) => s,
		_ => return,
	};

	if !sphere.network_id.is_empty() {
		let mut g = write_recovered(&d.graph);
		if sphere.network_id != g.network_id {
			inject_remote_scope(&mut g, sphere, &msg.origin);
		}
		drop(g);
		d.node.ledger.put_routing(&sphere.network_id, &msg.origin);
		d.persist();
	}

	if let Some(q) = &d.queue {
		let mut g = write_recovered(&d.graph);
		let root_id = g.root.id.clone();
		tick::pulse::pulse(q, &mut g, &root_id, PULSE_THRESHOLD * 2.0);
	}
}

fn handle_answer(d: &Deps, msg: GossipMessage) {
	let sphere = match &msg.payload {
		GossipPayload::Sphere(s) => s,
		_ => return,
	};

	let reason_id = msg.id.strip_prefix("answer-").unwrap_or(&msg.id);
	resolve_question_from_peer(d, reason_id, sphere, &msg.origin);

	if let Some(q) = &d.queue {
		let mut g = write_recovered(&d.graph);
		let root_id = g.root.id.clone();
		tick::pulse::pulse(q, &mut g, &root_id, PULSE_THRESHOLD * 2.0);
	}
}

fn handle_question(d: &Deps, msg: GossipMessage) {
	let question = match &msg.payload {
		GossipPayload::Question(q) => q,
		_ => return,
	};

	if question.reason_vec.is_empty() {
		return;
	}

	let g = read_recovered(&d.graph);
	let hits = search_all_unlocked(&g, &question.reason_vec, 1);
	if hits.is_empty() || hits[0].score < QUESTION_RESOLVE_THRESHOLD {
		return;
	}

	let reply = GossipMessage {
		kind: GossipKind::Sphere,
		id: format!("answer-{}", question.reason_id),
		origin: d.node.addr(),
		payload: GossipPayload::Sphere(SpherePayload {
			network_id: g.network_id.clone(),
			kern_id: g.root.id.clone(),
			anchor_text: g.root.anchor_text.clone(),
			anchor_vec: g.root.anchor_vec.clone(),
			entity_id: hits[0].entity_id.clone(),
			inner_radius: g.root.inner_radius,
			outer_radius: g.root.outer_radius,
		}),
	};
	drop(g);

	d.node.broadcast(reply);
}

fn handle_pulse(d: &Deps, msg: GossipMessage) {
	let pulse = match &msg.payload {
		GossipPayload::Pulse(p) => p,
		_ => return,
	};
	let q = match &d.queue {
		Some(q) => q,
		None => return,
	};

	let mut g = write_recovered(&d.graph);
	let kern_id = if g.kerns.contains_key(&pulse.kern_id) {
		pulse.kern_id.clone()
	} else {
		g.root.id.clone()
	};
	tick::pulse::pulse(q, &mut g, &kern_id, pulse.strength);
}

fn handle_peer_exchange(d: &Deps, msg: GossipMessage) {
	let pe = match &msg.payload {
		GossipPayload::PeerExchange(p) => p,
		_ => return,
	};

	if !msg.origin.is_empty() {
		d.node.add_peer(&msg.origin);
	}

	let self_addr = d.node.addr();
	for peer in &pe.peers {
		if peer == &self_addr {
			continue;
		}
		if d.node.peer_count() >= GOSSIP_MAX_PEERS {
			break;
		}
		d.node.add_peer(peer);
	}
}

fn validated_delta_value(replica: &str, object_id: &str, value: u64) -> Option<u64> {
	if replica.is_empty() || object_id.is_empty() {
		return None;
	}
	if value == 0 || value > GOSSIP_CRDT_DELTA_MAX {
		return None;
	}
	Some(value)
}

fn handle_crdt_delta(d: &Deps, msg: GossipMessage) {
	let delta = match &msg.payload {
		GossipPayload::CrdtDelta(c) => c.clone(),
		_ => return,
	};

	let mut changed = false;
	{
		let mut g = write_recovered(&d.graph);
		g.observe_lamport(delta.lamport);

		match delta.target {
			CrdtTarget::ThoughtAccessCount => {
				let value = match validated_delta_value(&delta.replica, &delta.object_id, delta.value) {
					Some(v) => v,
					None => return,
				};
				let mut incoming = GCounter::new();
				incoming.increment(&delta.replica, value);
				for kern_id in g.all_ids() {
					if let Some(kern) = g.get_mut(&kern_id) {
						if let Some(t) = kern.entities.get_mut(&delta.object_id) {
							changed = t.access_count.merge(&incoming);
							break;
						}
					}
				}
			}
			CrdtTarget::ReasonTraversalCount => {
				let value = match validated_delta_value(&delta.replica, &delta.object_id, delta.value) {
					Some(v) => v,
					None => return,
				};
				let mut incoming = GCounter::new();
				incoming.increment(&delta.replica, value);
				for kern_id in g.all_ids() {
					if let Some(kern) = g.get_mut(&kern_id) {
						if let Some(r) = kern.reasons.get_mut(&delta.object_id) {
							changed = r.traversal_count.merge(&incoming);
							break;
						}
					}
				}
			}
			CrdtTarget::ReasonScore => {
				let Some(score) = decode_lww::<f64>(&delta.lww_value) else {
					return;
				};
				for kern_id in g.all_ids() {
					if let Some(kern) = g.get_mut(&kern_id) {
						if let Some(r) = kern.reasons.get_mut(&delta.object_id) {
							if (delta.lamport, &delta.producer) > (r.score_lamport, &r.score_producer) {
								r.score = score;
								r.score_lamport = delta.lamport;
								r.score_producer = delta.producer.clone();
								changed = true;
							}
							break;
						}
					}
				}
			}
			CrdtTarget::ValidUntil => {
				let Some(valid) = decode_lww::<Option<SystemTime>>(&delta.lww_value) else {
					return;
				};
				for kern_id in g.all_ids() {
					if let Some(kern) = g.get_mut(&kern_id) {
						if let Some(t) = kern.entities.get_mut(&delta.object_id) {
							if (delta.lamport, &delta.producer) > (t.valid_until_lamport, &t.valid_until_producer)
							{
								t.valid_until = valid;
								t.valid_until_lamport = delta.lamport;
								t.valid_until_producer = delta.producer.clone();
								changed = true;
							}
							break;
						}
					}
				}
			}
			CrdtTarget::Statements => {
				let Some(adds) = decode_lww::<Vec<String>>(&delta.orset_delta) else {
					return;
				};
				for kern_id in g.all_ids() {
					if let Some(kern) = g.get_mut(&kern_id) {
						if let Some(t) = kern.entities.get_mut(&delta.object_id) {
							for s in &adds {
								if !t.statements.iter().any(|existing| existing == s) {
									t.statements.push(s.clone());
									changed = true;
								}
							}
							break;
						}
					}
				}
			}
		}
	}
	// Persist only after dropping the write guard — the save closure read-locks the graph (deadlock otherwise).
	if changed {
		d.persist();
	}
}

fn decode_lww<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Option<T> {
	bincode::serde::decode_from_slice(bytes, bincode::config::standard())
		.ok()
		.map(|(v, _)| v)
}

// Content↔id binding is NOT verified — network scoping + GOSSIP_REMOTE_KERN_ENTITY_CAP bound it.
fn handle_entity_sync(d: &Deps, msg: GossipMessage) {
	let payload = match &msg.payload {
		GossipPayload::EntitySync(p) => p,
		_ => return,
	};
	if payload.network_id.is_empty() {
		return;
	}
	let mut g = write_recovered(&d.graph);
	if payload.network_id == g.network_id {
		return;
	}
	let phantom = format!("remote-{}-{}", payload.network_id, payload.kern_id);
	if !g.kerns.contains_key(&phantom) {
		let k = new_phantom_kern(&g, &phantom);
		g.register(k);
	}
	let mut changed = false;
	for e in &payload.entities {
		changed |= crate::base::merge::merge_remote_entity(&mut g, &phantom, e.clone());
	}
	drop(g);
	if changed {
		d.persist();
	}
}

fn inject_remote_scope(g: &mut GraphGnn, sphere: &SpherePayload, _origin: &str) {
	let phantom_id = format!("remote-{}-{}", sphere.network_id, sphere.kern_id);

	if let Some(kern) = g.kerns.get_mut(&phantom_id) {
		kern.anchor_text = sphere.anchor_text.clone();
		kern.anchor_vec = sphere.anchor_vec.clone();
		kern.inner_radius = sphere.inner_radius;
		kern.outer_radius = sphere.outer_radius;
	} else {
		let mut k = new_phantom_kern(g, &phantom_id);
		k.anchor_text = sphere.anchor_text.clone();
		k.anchor_vec = sphere.anchor_vec.clone();
		k.inner_radius = sphere.inner_radius;
		k.outer_radius = sphere.outer_radius;
		g.register(k);
	}
}

fn new_phantom_kern(g: &GraphGnn, phantom_id: &str) -> Kern {
	let mut k = Kern::new(phantom_id, &g.root.id);
	k.root_id = g.root.root_id.clone();
	k
}

fn resolve_question_from_peer(d: &Deps, reason_id: &str, sphere: &SpherePayload, origin: &str) {
	if sphere.entity_id.is_empty() || sphere.network_id.is_empty() {
		return;
	}

	let (reason, kern_id, local_net) = {
		let g = read_recovered(&d.graph);
		match crate::base::search::find_reason(&g, reason_id) {
			Some((reason, kern_id)) => (reason, kern_id, g.network_id.clone()),
			None => return,
		}
	};
	if reason.kind != ReasonKind::Question || !reason.to.is_empty() {
		return;
	}

	let is_local = sphere.network_id == local_net;

	let mut g = write_recovered(&d.graph);
	if let Some(kern) = g.kerns.get_mut(&kern_id) {
		if let Some(r) = kern.reasons.get_mut(reason_id) {
			r.to = sphere.entity_id.clone();
			r.kind = ReasonKind::Similarity;
			if !is_local {
				r.to_net_id = sphere.network_id.clone();
				d.node.ledger.put_routing(&sphere.network_id, origin);
			}
			r.id = crate::base::math::reason_id(&r.from, &r.to, r.kind, &r.text, &r.to_net_id);
		}
	}
	drop(g);

	if let Some(q) = &d.queue {
		q.enqueue(tick::queue::task(tick::queue::TaskKind::Persist, &kern_id));
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::reason::add_reason;
	use crate::base::types::{mk_entity as mk_entity_kind, Entity, EntityKind, Reason};

	fn mk_entity(id: &str, text: &str, heat: f64) -> Entity {
		mk_entity_kind(id, text, heat, EntityKind::Fact)
	}

	fn mk_deps(graph: Arc<RwLock<GraphGnn>>) -> Deps {
		Deps {
			graph,
			node: Node::new("127.0.0.1:0", "testnet", vec![]),
			queue: None,
			save: None,
		}
	}

	fn esync_msg(network_id: &str, kern_id: &str, entities: Vec<Entity>) -> GossipMessage {
		GossipMessage {
			kind: GossipKind::EntitySync,
			id: "esync-test".to_string(),
			origin: "127.0.0.1:1".to_string(),
			payload: GossipPayload::EntitySync(EntitySyncPayload {
				network_id: network_id.to_string(),
				kern_id: kern_id.to_string(),
				entities,
			}),
		}
	}

	#[test]
	fn entity_sync_merges_remote_body_into_phantom() {
		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let d = mk_deps(g.clone());

		let msg = esync_msg(
			"othernet",
			"rootK",
			vec![mk_entity("eR", "remote thought", 3.0)],
		);
		handle_entity_sync(&d, msg);

		let guard = g.read();
		let phantom = "remote-othernet-rootK";
		let kern = guard.kerns.get(phantom).expect("phantom kern created");
		assert!(
			kern.entities.contains_key("eR"),
			"remote entity merged into phantom"
		);
		assert_eq!(guard.kern_of_entity("eR"), Some(phantom));
	}

	#[test]
	fn entity_sync_ignores_same_network() {
		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let own_net = g.read().network_id.clone();
		let d = mk_deps(g.clone());

		let msg = esync_msg(&own_net, "rootK", vec![mk_entity("eR", "echo", 3.0)]);
		handle_entity_sync(&d, msg);

		let guard = g.read();
		assert!(
			guard.kern_of_entity("eR").is_none(),
			"own-network echo ignored"
		);
		assert!(
			!guard.kerns.keys().any(|k| k.starts_with("remote-")),
			"no phantom kern created for own data"
		);
	}

	#[test]
	fn delta_validation_accepts_sane_value() {
		assert_eq!(validated_delta_value("r1", "obj", 5), Some(5));
		assert_eq!(
			validated_delta_value("r1", "obj", GOSSIP_CRDT_DELTA_MAX),
			Some(GOSSIP_CRDT_DELTA_MAX)
		);
	}

	#[test]
	fn delta_validation_drops_empty_ids_and_zero() {
		assert_eq!(validated_delta_value("", "obj", 5), None);
		assert_eq!(validated_delta_value("r1", "", 5), None);
		assert_eq!(validated_delta_value("r1", "obj", 0), None);
	}

	#[test]
	fn delta_validation_rejects_oversized_value() {
		assert_eq!(
			validated_delta_value("r1", "obj", GOSSIP_CRDT_DELTA_MAX + 1),
			None
		);
		assert_eq!(validated_delta_value("r1", "obj", u64::MAX), None);
	}

	#[test]
	fn peer_exchange_caps_at_max_peers() {
		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let d = mk_deps(g);
		let peers: Vec<String> = (0..100).map(|i| format!("10.0.0.{i}:7400")).collect();
		let msg = GossipMessage {
			kind: GossipKind::PeerExchange,
			id: "pe-test".to_string(),
			origin: "127.0.0.1:1".to_string(),
			payload: GossipPayload::PeerExchange(PeerExchangePayload { peers }),
		};
		handle_peer_exchange(&d, msg);
		assert_eq!(
			d.node.peer_count(),
			GOSSIP_MAX_PEERS,
			"peer table is capped at GOSSIP_MAX_PEERS"
		);
	}

	fn mk_deps_with_save(
		graph: Arc<RwLock<GraphGnn>>,
		calls: Arc<std::sync::atomic::AtomicUsize>,
	) -> Deps {
		Deps {
			graph,
			node: Node::new("127.0.0.1:0", "testnet", vec![]),
			queue: None,
			save: Some(Arc::new(move || {
				calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
			})),
		}
	}

	fn delta_msg(target: CrdtTarget, object_id: &str, replica: &str, value: u64) -> GossipMessage {
		GossipMessage {
			kind: GossipKind::Delta,
			id: "delta-test".to_string(),
			origin: "127.0.0.1:1".to_string(),
			payload: GossipPayload::CrdtDelta(CrdtDeltaPayload {
				kern_id: "k".to_string(),
				object_id: object_id.to_string(),
				target,
				replica: replica.to_string(),
				value,
				lamport: 0,
				producer: String::new(),
				lww_value: Vec::new(),
				orset_delta: Vec::new(),
			}),
		}
	}

	fn graph_with_one_entity(id: &str) -> Arc<RwLock<GraphGnn>> {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		k.entities.insert(id.to_string(), mk_entity(id, "t", 1.0));
		g.kerns.insert("k".to_string(), k);
		Arc::new(RwLock::new(g))
	}

	#[test]
	fn crdt_delta_merges_counter_and_persists() {
		use std::sync::atomic::{AtomicUsize, Ordering};
		let g = graph_with_one_entity("e");
		let calls = Arc::new(AtomicUsize::new(0));
		let d = mk_deps_with_save(g.clone(), calls.clone());

		handle_crdt_delta(
			&d,
			delta_msg(CrdtTarget::ThoughtAccessCount, "e", "peerR", 7),
		);

		let merged = g.read().kerns["k"].entities["e"].access_count.value();
		assert_eq!(merged, 7, "the remote replica's count is merged in");
		assert_eq!(
			calls.load(Ordering::SeqCst),
			1,
			"a counter merge persists per the Deps contract (was silently dropped)"
		);
	}

	#[test]
	fn crdt_delta_idempotent_redelta_does_not_persist_again() {
		use std::sync::atomic::{AtomicUsize, Ordering};
		let g = graph_with_one_entity("e");
		let calls = Arc::new(AtomicUsize::new(0));
		let d = mk_deps_with_save(g.clone(), calls.clone());

		handle_crdt_delta(
			&d,
			delta_msg(CrdtTarget::ThoughtAccessCount, "e", "peerR", 5),
		);
		handle_crdt_delta(
			&d,
			delta_msg(CrdtTarget::ThoughtAccessCount, "e", "peerR", 5),
		);
		assert_eq!(
			calls.load(Ordering::SeqCst),
			1,
			"only the changing merge persists"
		);
	}

	fn graph_with_open_question() -> (Arc<RwLock<GraphGnn>>, String) {
		let mut g = GraphGnn::new();
		let net = g.network_id.clone();
		let mut k = Kern::new("kq", "");
		add_reason(
			&mut k,
			Reason {
				from: "a".into(),
				to: String::new(),
				id: "r1".into(),
				kind: ReasonKind::Question,
				..Default::default()
			},
		);
		g.kerns.insert("kq".into(), k);
		(Arc::new(RwLock::new(g)), net)
	}

	fn answer_sphere(net: &str, entity_id: &str) -> SpherePayload {
		SpherePayload {
			network_id: net.to_string(),
			kern_id: "rk".into(),
			anchor_vec: vec![],
			anchor_text: String::new(),
			entity_id: entity_id.to_string(),
			inner_radius: 0.0,
			outer_radius: 0.0,
		}
	}

	#[test]
	fn resolve_question_from_peer_fills_answer_and_promotes_to_similarity() {
		let (g, net) = graph_with_open_question();
		let d = mk_deps(g.clone());
		resolve_question_from_peer(&d, "r1", &answer_sphere(&net, "ans"), "127.0.0.1:9");
		let guard = g.read();
		let r = guard.kerns["kq"].reasons.get("r1").expect("reason present");
		assert_eq!(r.to, "ans", "answer endpoint filled in");
		assert!(
			matches!(r.kind, ReasonKind::Similarity),
			"open question promoted to similarity"
		);
	}

	#[test]
	fn resolve_question_from_peer_ignores_an_empty_answer() {
		let (g, net) = graph_with_open_question();
		let d = mk_deps(g.clone());
		resolve_question_from_peer(&d, "r1", &answer_sphere(&net, ""), "o");
		let guard = g.read();
		let r = guard.kerns["kq"].reasons.get("r1").unwrap();
		assert!(
			r.to.is_empty() && matches!(r.kind, ReasonKind::Question),
			"an empty answer leaves the question untouched",
		);
	}

	#[test]
	fn handle_question_with_empty_reason_vec_is_a_noop() {
		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let d = mk_deps(g.clone());
		let msg = GossipMessage {
			kind: GossipKind::Question,
			id: "q".into(),
			origin: "o".into(),
			payload: GossipPayload::Question(QuestionPayload {
				reason_id: "r".into(),
				from_id: "a".into(),
				reason_vec: vec![],
				question_text: String::new(),
			}),
		};
		handle_question(&d, msg);
	}

	#[test]
	fn handle_pulse_falls_back_to_root_for_an_unknown_kern() {
		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let q = Arc::new(tick::queue::Queue::new(64));
		let d = Deps {
			graph: g.clone(),
			node: Node::new("127.0.0.1:0", "testnet", vec![]),
			queue: Some(q),
			save: None,
		};
		let msg = GossipMessage {
			kind: GossipKind::Pulse,
			id: "p".into(),
			origin: "o".into(),
			payload: GossipPayload::Pulse(PulsePayload {
				kern_id: "does-not-exist".into(),
				strength: 1.0,
			}),
		};
		handle_pulse(&d, msg);
	}
}
