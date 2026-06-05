use std::sync::{Arc, RwLock};

use crate::base::constants::*;
use crate::base::graph::GraphGnn;
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
	/// Persist hook. Federation mutations (remote scope inject, counter
	/// merges, question resolution) call this so federated knowledge survives
	/// a restart instead of living only in memory.
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

/// Periodically broadcast this node's root-kern scope (purpose vector +
/// radii) so peers become aware of the knowledge it holds and can route
/// questions / fetch content to it. The outbound counterpart to
/// `handle_sphere`: without it a node only ever receives. Runs until the
/// node's stop signal fires.
pub fn start_announce(node: Arc<Node>, graph: Arc<RwLock<GraphGnn>>) {
	let mut stop = node.stop_rx.clone();
	tokio::spawn(async move {
		let mut interval = tokio::time::interval(GOSSIP_HEARTBEAT_INTERVAL);
		loop {
			tokio::select! {
				_ = interval.tick() => {
					let payload = {
						let g = graph.read().unwrap();
						// Nothing worth announcing until the kern has a purpose.
						if g.root.purpose_vec.is_empty() {
							None
						} else {
							Some(SpherePayload {
								network_id: g.network_id.clone(),
								kern_id: g.root.id.clone(),
								purpose_text: g.root.purpose_text.clone(),
								purpose_vec: g.root.purpose_vec.clone(),
								entity_id: String::new(),
								inner_radius: g.root.inner_radius,
								outer_radius: g.root.outer_radius,
							})
						}
					};
					if let Some(payload) = payload {
						let stamp = std::time::SystemTime::now()
							.duration_since(std::time::UNIX_EPOCH)
							.map(|d| d.as_nanos())
							.unwrap_or(0);
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

/// Periodically broadcast this node's hottest LOCAL entities so peers can
/// merge the actual thought content (not just scope) into a per-network
/// phantom kern via `base::merge::merge_remote_entity`. The outbound
/// counterpart to `handle_entity_sync`. Never re-broadcasts entities living
/// in `remote-*` kerns (received data). Runs until the node's stop signal.
pub fn start_entity_sync(node: Arc<Node>, graph: Arc<RwLock<GraphGnn>>) {
	let mut stop = node.stop_rx.clone();
	tokio::spawn(async move {
		let mut interval = tokio::time::interval(GOSSIP_HEARTBEAT_INTERVAL);
		loop {
			tokio::select! {
				_ = interval.tick() => {
					let payload = {
						let g = graph.read().unwrap();
						let mut entities: Vec<crate::base::types::Entity> = g
							.kerns
							.iter()
							.filter(|(kid, _)| !kid.starts_with("remote-"))
							.flat_map(|(_, k)| k.entities.values().cloned())
							.collect();
						if entities.is_empty() {
							None
						} else {
							entities.sort_by(|a, b| {
								b.heat.partial_cmp(&a.heat).unwrap_or(std::cmp::Ordering::Equal)
							});
							entities.truncate(32);
							Some(EntitySyncPayload {
								network_id: g.network_id.clone(),
								kern_id: g.root.id.clone(),
								entities,
							})
						}
					};
					if let Some(payload) = payload {
						let stamp = std::time::SystemTime::now()
							.duration_since(std::time::UNIX_EPOCH)
							.map(|d| d.as_nanos())
							.unwrap_or(0);
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

fn handle_sphere(d: &Deps, msg: GossipMessage) {
	let sphere = match &msg.payload {
		GossipPayload::Sphere(s) => s,
		_ => return,
	};

	if !sphere.network_id.is_empty() {
		let mut g = d.graph.write().unwrap();
		if sphere.network_id != g.network_id {
			inject_remote_scope(&mut g, sphere, &msg.origin);
		}
		drop(g);
		d.node.ledger.put_routing(&sphere.network_id, &msg.origin);
		d.persist();
	}

	if let Some(q) = &d.queue {
		let mut g = d.graph.write().unwrap();
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
		let mut g = d.graph.write().unwrap();
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

	let g = d.graph.read().unwrap();
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
			purpose_text: g.root.purpose_text.clone(),
			purpose_vec: g.root.purpose_vec.clone(),
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

	let mut g = d.graph.write().unwrap();
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
		if d.node.peer_list().len() >= GOSSIP_MAX_PEERS {
			break;
		}
		d.node.add_peer(peer);
	}
}

/// Validate an inbound CRDT delta, returning the slot value to merge or `None`
/// to drop it.
///
/// `value` is the sender's ABSOLUTE total for its `replica` slot (not an
/// increment): merging it via the GCounter per-slot `max` is therefore
/// commutative, idempotent and convergent regardless of delivery order or
/// duplication. Empty ids and zero are dropped (no-ops), and values above
/// [`GOSSIP_CRDT_DELTA_MAX`] are rejected to bound a peer pinning a slot.
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

	let value = match validated_delta_value(&delta.replica, &delta.object_id, delta.value) {
		Some(v) => v,
		None => return,
	};
	let mut incoming = GCounter::new();
	incoming.increment(&delta.replica, value);

	let mut g = d.graph.write().unwrap();
	match delta.target {
		CrdtTarget::ThoughtAccessCount => {
			for kern_id in g.all_ids() {
				if let Some(kern) = g.get_mut(&kern_id) {
					if let Some(t) = kern.entities.get_mut(&delta.object_id) {
						t.access_count.merge(&incoming);
						break;
					}
				}
			}
		}
		CrdtTarget::ReasonTraversalCount => {
			for kern_id in g.all_ids() {
				if let Some(kern) = g.get_mut(&kern_id) {
					if let Some(r) = kern.reasons.get_mut(&delta.object_id) {
						r.traversal_count.merge(&incoming);
						break;
					}
				}
			}
		}
	}
}

/// Merge entity bodies a peer shared into a per-network phantom kern via the
/// content-addressed CRDT join. Ignores our own data echoed back and empty
/// network ids. Persists only when the merge actually changed the graph.
fn handle_entity_sync(d: &Deps, msg: GossipMessage) {
	let payload = match &msg.payload {
		GossipPayload::EntitySync(p) => p,
		_ => return,
	};
	if payload.network_id.is_empty() {
		return;
	}
	let mut g = d.graph.write().unwrap();
	// Ignore our own data echoed back.
	if payload.network_id == g.network_id {
		return;
	}
	let phantom = format!("remote-{}-{}", payload.network_id, payload.kern_id);
	if !g.kerns.contains_key(&phantom) {
		let mut k = Kern::new(phantom.as_str(), g.root.id.as_str());
		k.root_id = g.root.root_id.clone();
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
		kern.purpose_text = sphere.purpose_text.clone();
		kern.purpose_vec = sphere.purpose_vec.clone();
		kern.inner_radius = sphere.inner_radius;
		kern.outer_radius = sphere.outer_radius;
	} else {
		let mut k = Kern::new(&phantom_id, &g.root.id);
		k.root_id = g.root.root_id.clone();
		k.purpose_text = sphere.purpose_text.clone();
		k.purpose_vec = sphere.purpose_vec.clone();
		k.inner_radius = sphere.inner_radius;
		k.outer_radius = sphere.outer_radius;
		g.register(k);
	}
}

fn resolve_question_from_peer(d: &Deps, reason_id: &str, sphere: &SpherePayload, origin: &str) {
	if sphere.entity_id.is_empty() || sphere.network_id.is_empty() {
		return;
	}

	let (reason, kern_id) = match crate::base::search::find_reason(&d.graph.read().unwrap(), reason_id) {
		Some(pair) => pair,
		None => return,
	};
	if reason.kind != ReasonKind::Question || !reason.to.is_empty() {
		return;
	}

	let is_local = sphere.network_id == d.graph.read().unwrap().network_id;

	let mut g = d.graph.write().unwrap();
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
	use crate::base::types::{
		Acl, ChunkPart, ChunkPartKind, Entity, EntityKind, EntityStatus, Source,
	};
	use crate::crdt::GCounter;

	fn mk_entity(id: &str, text: &str, heat: f64) -> Entity {
		let mut e = Entity {
			id: id.to_string(),
			root_id: String::new(),
			external_id: String::new(),
			superseded_by: String::new(),
			kind: EntityKind::Fact,
			status: EntityStatus::Active,
			statements: vec![text.to_string()],
			chunks: vec![ChunkPart {
				kind: ChunkPartKind::StatementRef,
				text: String::new(),
				index: 0,
			}],
			vector: vec![0.0; 8],
			gnn_vector: Vec::new(),
			score: 0.0,
			conf_alpha: 2.0,
			conf_beta: 1.0,
			source: Source::Inline {
				hash: id.into(),
				section: String::new(),
			},
			created_at: None,
			acl: Acl::default(),
			access_count: GCounter::new(),
			accessed_at: None,
			heat: heat as f32,
			heat_updated_at: None,
			updated_at: None,
			valid_until: None,
			producer_id: String::new(),
			unlinked_count: 0,
		};
		e.refresh_score();
		e
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

		let msg = esync_msg("othernet", "rootK", vec![mk_entity("eR", "remote thought", 3.0)]);
		handle_entity_sync(&d, msg);

		let guard = g.read().unwrap();
		let phantom = "remote-othernet-rootK";
		let kern = guard.kerns.get(phantom).expect("phantom kern created");
		assert!(kern.entities.contains_key("eR"), "remote entity merged into phantom");
		assert_eq!(guard.kern_of_entity("eR"), Some(phantom));
	}

	#[test]
	fn entity_sync_ignores_same_network() {
		let g = Arc::new(RwLock::new(GraphGnn::new()));
		let own_net = g.read().unwrap().network_id.clone();
		let d = mk_deps(g.clone());

		let msg = esync_msg(&own_net, "rootK", vec![mk_entity("eR", "echo", 3.0)]);
		handle_entity_sync(&d, msg);

		let guard = g.read().unwrap();
		assert!(guard.kern_of_entity("eR").is_none(), "own-network echo ignored");
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
		// A peer trying to pin a slot toward u64::MAX is dropped.
		assert_eq!(
			validated_delta_value("r1", "obj", GOSSIP_CRDT_DELTA_MAX + 1),
			None
		);
		assert_eq!(validated_delta_value("r1", "obj", u64::MAX), None);
	}
}
