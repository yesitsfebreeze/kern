use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;

use rand::seq::SliceRandom;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use crate::base::constants::*;
use crate::base::locks::{read_recovered, write_recovered};

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
		*write_recovered(&self.handler) = Some(h);
	}

	pub fn set_fetch_handler(&self, h: FetchHandler) {
		*write_recovered(&self.fetch_handler) = Some(h);
	}

	pub fn addr(&self) -> String {
		read_recovered(&self.addr).clone()
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
		let mut peers = write_recovered(&self.peers);
		if peers.len() >= GOSSIP_MAX_PEERS {
			return;
		}
		if !peers.iter().any(|p| p == addr) {
			peers.push(addr.to_string());
		}
	}

	pub fn peer_list(&self) -> Vec<String> {
		read_recovered(&self.peers).clone()
	}

	pub fn peer_count(&self) -> usize {
		read_recovered(&self.peers).len()
	}

	pub async fn listen(self: &Arc<Self>) -> Result<String, std::io::Error> {
		let addr = self.addr();
		let listener = TcpListener::bind(&addr).await?;
		let actual = listener.local_addr()?.to_string();
		*write_recovered(&self.addr) = actual.clone();

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
		let peer_addr = self.ledger.lookup_routing(network_id)?;
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

		if let Some(h) = read_recovered(&self.handler).as_ref() {
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

		let (body, found) = if let Some(fh) = read_recovered(&self.fetch_handler).as_ref() {
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

fn now_nanos() -> u64 {
	crate::base::util::now_nanos() as u64
}
