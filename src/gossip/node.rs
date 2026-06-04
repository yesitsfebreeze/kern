use std::sync::{Arc, RwLock};
use std::time::Duration;

use rand::seq::SliceRandom;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use crate::base::constants::*;

use super::ledger::Ledger;
use super::seen::SeenSet;
use super::sybil::RateClipper;
use super::types::*;

pub type Handler = Arc<dyn Fn(GossipMessage) + Send + Sync>;

pub type FetchHandler = Arc<dyn Fn(&str, &str) -> (Vec<u8>, bool) + Send + Sync>;

pub struct Node {
	pub addr: RwLock<String>,
	pub network_id: String,
	peers: RwLock<Vec<String>>,
	seen: SeenSet,
	pub ledger: Ledger,
	handler: RwLock<Option<Handler>>,
	fetch_handler: RwLock<Option<FetchHandler>>,
	clipper: RwLock<Option<Arc<RateClipper>>>,
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
			handler: RwLock::new(None),
			fetch_handler: RwLock::new(None),
			clipper: RwLock::new(None),
			stop_tx,
			stop_rx,
		})
	}

	pub fn set_handler(&self, h: Handler) {
		*self.handler.write().unwrap() = Some(h);
	}

	pub fn set_fetch_handler(&self, h: FetchHandler) {
		*self.fetch_handler.write().unwrap() = Some(h);
	}

	pub fn set_clipper(&self, c: Option<Arc<RateClipper>>) {
		*self.clipper.write().unwrap() = c;
	}

	pub fn clipper(&self) -> Option<Arc<RateClipper>> {
		self.clipper.read().unwrap().clone()
	}

	pub fn addr(&self) -> String {
		self.addr.read().unwrap().clone()
	}

	pub fn add_peer(&self, addr: &str) {
		let mut peers = self.peers.write().unwrap();
		if peers.len() >= GOSSIP_MAX_PEERS {
			return;
		}
		if !peers.iter().any(|p| p == addr) {
			peers.push(addr.to_string());
		}
	}

	pub fn peer_list(&self) -> Vec<String> {
		self.peers.read().unwrap().clone()
	}

	pub async fn listen(self: &Arc<Self>) -> Result<String, std::io::Error> {
		let addr = self.addr();
		let listener = TcpListener::bind(&addr).await?;
		let actual = listener.local_addr()?.to_string();
		*self.addr.write().unwrap() = actual.clone();

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

		if !msg.origin.is_empty() {
			if let Some(c) = self.clipper.read().unwrap().as_ref() {
				if !c.admit(&msg.origin) {
					return;
				}
			}
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

		if let Some(h) = self.handler.read().unwrap().as_ref() {
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

		let (body, found) = if let Some(fh) = self.fetch_handler.read().unwrap().as_ref() {
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

pub(super) async fn encode_msg(
	stream: &mut TcpStream,
	msg: &GossipMessage,
) -> Result<(), std::io::Error> {
	let bytes = bincode::serde::encode_to_vec(msg, bincode::config::standard())
		.map_err(std::io::Error::other)?;
	let len = (bytes.len() as u32).to_be_bytes();
	stream.write_all(&len).await?;
	stream.write_all(&bytes).await?;
	stream.flush().await?;
	Ok(())
}

pub(super) async fn decode_msg(stream: &mut TcpStream) -> Option<GossipMessage> {
	let mut len_buf = [0u8; 4];
	stream.read_exact(&mut len_buf).await.ok()?;
	let len = u32::from_be_bytes(len_buf) as usize;
	if len > 4 * 1024 * 1024 {
		return None;
	}
	let mut buf = vec![0u8; len];
	stream.read_exact(&mut buf).await.ok()?;
	bincode::serde::decode_from_slice(&buf, bincode::config::standard()).ok().map(|(v, _)| v)
}

pub(super) async fn send_msg(addr: &str, msg: &GossipMessage) -> Result<(), std::io::Error> {
	let mut stream = tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(addr))
		.await
		.map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "dial timeout"))??;
	encode_msg(&mut stream, msg).await
}

async fn send_and_receive(addr: &str, msg: &GossipMessage) -> Option<GossipMessage> {
	let mut stream = tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(addr))
		.await
		.ok()?
		.ok()?;
	encode_msg(&mut stream, msg).await.ok()?;
	tokio::time::timeout(Duration::from_secs(5), decode_msg(&mut stream))
		.await
		.ok()?
}

fn now_nanos() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_nanos() as u64
}
