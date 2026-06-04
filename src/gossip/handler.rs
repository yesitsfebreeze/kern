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

fn handle_crdt_delta(d: &Deps, msg: GossipMessage) {
	let delta = match &msg.payload {
		GossipPayload::CrdtDelta(c) => c.clone(),
		_ => return,
	};

	let mut incoming = GCounter::new();
	if delta.value > 0 {
		incoming.increment(&delta.replica, delta.value);
	}

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
