use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;

use rand::seq::SliceRandom;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use crate::base::constants::*;

use super::ledger::Ledger;
use super::seen::SeenSet;
use super::transport::{decode_msg, encode_msg, send_and_receive, send_msg};
use super::types::*;

pub type Handler = Arc<dyn Fn(GossipMessage) + Send + Sync>;

pub type FetchHandler = Arc<dyn Fn(&str, &str) -> (Vec<u8>, bool) + Send + Sync>;

pub struct Node {
	pub addr: RwLock<String>,
	pub network_id: String,
	peers: RwLock<Vec<String>>,
	seen: SeenSet,
	pub ledger: Ledger,
	lamport: AtomicU64,
	handler: RwLock<Option<Handler>>,
	fetch_handler: RwLock<Option<FetchHandler>>,
	stop_tx: watch::Sender<bool>,
	pub stop_rx: watch::Receiver<bool>,
}

impl Node {
	pub fn new(addr: &str, network_id: &str, peers: Vec<String>) -> Arc<Self> {
		let (stop_tx, stop_rx) = watch::channel(false);
		let mut peers = peers;
		peers.truncate(GOSSIP_MAX_PEERS);
		Arc::new(Self {
			addr: RwLock::new(addr.to_string()),
			network_id: network_id.to_string(),
			peers: RwLock::new(peers),
			seen: SeenSet::new(),
			ledger: Ledger::new(),
			lamport: AtomicU64::new(0),
			handler: RwLock::new(None),
			fetch_handler: RwLock::new(None),
			stop_tx,
			stop_rx,
		})
	}

	pub fn set_handler(&self, h: Handler) {
		*self.handler.write() = Some(h);
	}

	pub fn set_fetch_handler(&self, h: FetchHandler) {
		*self.fetch_handler.write() = Some(h);
	}

	pub fn addr(&self) -> String {
		self.addr.read().clone()
	}

	pub fn bump_lamport(&self) -> u64 {
		self.lamport.fetch_add(1, Ordering::SeqCst) + 1
	}

	pub fn observe_lamport(&self, remote: u64) {
		let mut current = self.lamport.load(Ordering::SeqCst);
		while remote > current {
			match self
				.lamport
				.compare_exchange(current, remote + 1, Ordering::SeqCst, Ordering::SeqCst)
			{
				Ok(_) => break,
				Err(actual) => current = actual,
			}
		}
	}

	pub fn add_peer(&self, addr: &str) {
		let mut peers = self.peers.write();
		if peers.len() >= GOSSIP_MAX_PEERS {
			return;
		}
		if !peers.iter().any(|p| p == addr) {
			peers.push(addr.to_string());
		}
	}

	pub fn peer_list(&self) -> Vec<String> {
		self.peers.read().clone()
	}

	pub fn peer_count(&self) -> usize {
		self.peers.read().len()
	}

	pub async fn listen(self: &Arc<Self>) -> Result<String, std::io::Error> {
		let addr = self.addr();
		let listener = TcpListener::bind(&addr).await?;
		let actual = listener.local_addr()?.to_string();
		*self.addr.write() = actual.clone();

		let node = self.clone();
		let mut stop = self.stop_rx.clone();
		tokio::spawn(async move {
			loop {
				tokio::select! {
					result = listener.accept() => {
						match result {
							Ok((stream, _)) => {
								let n = node.clone();
								tokio::spawn(async move { n.handle_conn(stream).await });
							}
							Err(_) => break,
						}
					}
					_ = stop.changed() => break,
				}
			}
		});

		Ok(actual)
	}

	pub fn close(&self) {
		let _ = self.stop_tx.send(true);
	}

	pub fn broadcast(self: &Arc<Self>, msg: GossipMessage) {
		if self.seen.add_and_check(&msg.id) {
			return;
		}
		self.forward(msg);
	}

	pub async fn fetch_thought(&self, network_id: &str, entity_id: &str) -> Option<Vec<u8>> {
		let peer_addr = self
			.ledger
			.lookup_thought(entity_id)
			.or_else(|| self.ledger.lookup_routing(network_id))?;
		let msg = GossipMessage {
			kind: GossipKind::Fetch,
			id: format!("fetch-{entity_id}-{}", now_nanos()),
			origin: self.addr(),
			payload: GossipPayload::FetchRequest(FetchPayload {
				resource: "thought".into(),
				id: entity_id.into(),
			}),
		};
		match send_and_receive(&peer_addr, &msg).await {
			Some(reply) => {
				if let GossipPayload::FetchResult(r) = reply.payload {
					if r.found {
						Some(r.body)
					} else {
						None
					}
				} else {
					None
				}
			}
			None => None,
		}
	}

	pub fn start_heartbeat(self: &Arc<Self>) {
		let node = self.clone();
		let mut stop = self.stop_rx.clone();
		tokio::spawn(async move {
			let mut interval = tokio::time::interval(GOSSIP_HEARTBEAT_INTERVAL);
			loop {
				tokio::select! {
					_ = interval.tick() => {
						let msg = GossipMessage {
							kind: GossipKind::PeerExchange,
							id: format!("pe-{}-{}", node.addr(), now_nanos()),
							origin: node.addr(),
							payload: GossipPayload::PeerExchange(PeerExchangePayload {
								peers: node.peer_list(),
							}),
						};
						node.broadcast(msg);
					}
					_ = stop.changed() => break,
				}
			}
		});
	}

	async fn handle_conn(self: Arc<Self>, mut stream: TcpStream) {
		let msg = match decode_msg(&mut stream).await {
			Some(m) => m,
			None => return,
		};

		if msg.kind == GossipKind::Fetch {
			self.handle_fetch(stream, msg).await;
			return;
		}

		if self.seen.add_and_check(&msg.id) {
			return;
		}

		if !msg.origin.is_empty() && msg.origin != self.addr() {
			self.add_peer(&msg.origin);
		}

		if let GossipPayload::Sphere(ref s) = msg.payload {
			self.ledger.put_routing(&s.kern_id, &msg.origin);
		}

		if let Some(h) = self.handler.read().as_ref() {
			h(msg.clone());
		}

		self.forward(msg);
	}

	async fn handle_fetch(&self, mut stream: TcpStream, msg: GossipMessage) {
		let (resource, id) = if let GossipPayload::FetchRequest(ref f) = msg.payload {
			(f.resource.as_str(), f.id.as_str())
		} else {
			return;
		};

		let (body, found) = if let Some(fh) = self.fetch_handler.read().as_ref() {
			fh(resource, id)
		} else {
			(Vec::new(), false)
		};

		let reply = GossipMessage {
			kind: GossipKind::Fetch,
			id: String::new(),
			origin: self.addr(),
			payload: GossipPayload::FetchResult(FetchResultPayload { found, body }),
		};
		let _ = encode_msg(&mut stream, &reply).await;
	}

	fn forward(self: &Arc<Self>, msg: GossipMessage) {
		let peers = self.peer_list();
		let self_addr = self.addr();
		let mut candidates: Vec<&String> = peers
			.iter()
			.filter(|p| *p != &msg.origin && *p != &self_addr)
			.collect();

		let mut rng = rand::rng();
		candidates.shuffle(&mut rng);
		candidates.truncate(GOSSIP_FANOUT);

		for peer in candidates {
			let peer = peer.clone();
			let msg = msg.clone();
			tokio::spawn(async move {
				let _ = send_msg(&peer, &msg).await;
			});
		}
	}
}

use crate::base::util::now_nanos;

#[cfg(test)]
mod tests {
	use super::*;

	// Port 1 on loopback: refused instantly, no DNS and no off-host traffic.
	const DEAD_SEED: &str = "127.0.0.1:1";

	#[tokio::test]
	async fn an_unreachable_bootstrap_seed_never_blocks_or_panics_startup() {
		let node = Node::new("127.0.0.1:0", "net", vec![DEAD_SEED.into()]);
		let started = std::time::Instant::now();
		node.listen().await.expect("listener binds");
		node.start_heartbeat();
		node.broadcast(GossipMessage {
			kind: GossipKind::PeerExchange,
			id: "pe-1".into(),
			origin: node.addr(),
			payload: GossipPayload::PeerExchange(PeerExchangePayload {
				peers: node.peer_list(),
			}),
		});
		assert!(
			started.elapsed() < GOSSIP_DIAL_TIMEOUT,
			"a dead seed degrades in the background instead of stalling startup"
		);
		assert_eq!(node.peer_list(), vec![DEAD_SEED.to_string()]);
		node.close();
	}

	#[test]
	fn a_bootstrap_list_cannot_exceed_the_peer_cap() {
		let peers: Vec<String> = (0..GOSSIP_MAX_PEERS + 10)
			.map(|i| format!("10.0.0.{i}:7400"))
			.collect();
		let node = Node::new("127.0.0.1:0", "net", peers);
		assert_eq!(node.peer_count(), GOSSIP_MAX_PEERS);
		node.add_peer("10.9.9.9:7400");
		assert_eq!(node.peer_count(), GOSSIP_MAX_PEERS, "add_peer still capped");
	}
}
