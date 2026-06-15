//! TCP wire transport for gossip: length-prefixed bincode framing plus the
//! dial-and-send / dial-and-roundtrip helpers. Kept separate from `node.rs`'s
//! networking policy (heartbeat / broadcast / forward) so the framing layer can
//! evolve — and be tested — independently of peer-selection logic.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::base::constants::{GOSSIP_DIAL_TIMEOUT, GOSSIP_FETCH_TIMEOUT, GOSSIP_MAX_FRAME_BYTES};

use super::types::*;

/// Write `msg` as a big-endian u32 length prefix followed by its bincode bytes,
/// then flush.
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

/// Read one length-prefixed frame and bincode-decode it. Returns `None` on any
/// I/O error, a decode failure, or a length prefix over `GOSSIP_MAX_FRAME_BYTES`
/// (rejected before allocating or reading the body).
pub(super) async fn decode_msg(stream: &mut TcpStream) -> Option<GossipMessage> {
	let mut len_buf = [0u8; 4];
	stream.read_exact(&mut len_buf).await.ok()?;
	let len = u32::from_be_bytes(len_buf) as usize;
	if len > GOSSIP_MAX_FRAME_BYTES {
		return None;
	}
	let mut buf = vec![0u8; len];
	stream.read_exact(&mut buf).await.ok()?;
	bincode::serde::decode_from_slice(&buf, bincode::config::standard())
		.ok()
		.map(|(v, _)| v)
}

/// Dial `addr` (with `GOSSIP_DIAL_TIMEOUT`) and send one framed message.
pub(super) async fn send_msg(addr: &str, msg: &GossipMessage) -> Result<(), std::io::Error> {
	let mut stream = tokio::time::timeout(GOSSIP_DIAL_TIMEOUT, TcpStream::connect(addr))
		.await
		.map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "dial timeout"))??;
	encode_msg(&mut stream, msg).await
}

/// Dial `addr`, send `msg`, then await a single framed reply (bounded by
/// `GOSSIP_FETCH_TIMEOUT`). `None` on any dial / send / read failure.
pub(super) async fn send_and_receive(addr: &str, msg: &GossipMessage) -> Option<GossipMessage> {
	let mut stream = tokio::time::timeout(GOSSIP_DIAL_TIMEOUT, TcpStream::connect(addr))
		.await
		.ok()?
		.ok()?;
	encode_msg(&mut stream, msg).await.ok()?;
	tokio::time::timeout(GOSSIP_FETCH_TIMEOUT, decode_msg(&mut stream))
		.await
		.ok()?
}

#[cfg(test)]
mod tests {
	use super::*;
	use tokio::net::TcpListener;

	fn sample_msg() -> GossipMessage {
		GossipMessage {
			kind: GossipKind::PeerExchange,
			id: "msg-1".into(),
			origin: "127.0.0.1:9999".into(),
			payload: GossipPayload::PeerExchange(PeerExchangePayload {
				peers: vec!["a".into(), "b".into()],
			}),
		}
	}

	#[tokio::test]
	async fn encode_decode_round_trips_over_loopback() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap().to_string();
		let msg = sample_msg();

		let server = tokio::spawn(async move {
			let (mut stream, _) = listener.accept().await.unwrap();
			decode_msg(&mut stream).await
		});

		let mut client = TcpStream::connect(&addr).await.unwrap();
		encode_msg(&mut client, &msg).await.unwrap();

		let got = server
			.await
			.unwrap()
			.expect("a message decodes on the server side");
		assert_eq!(got.kind, msg.kind);
		assert_eq!(got.id, msg.id);
		assert_eq!(got.origin, msg.origin);
		match got.payload {
			GossipPayload::PeerExchange(p) => {
				assert_eq!(p.peers, vec!["a".to_string(), "b".to_string()]);
			}
			other => panic!("round-trip changed the payload variant: {other:?}"),
		}
	}

	#[tokio::test]
	async fn decode_rejects_frame_over_max_size() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap().to_string();

		let server = tokio::spawn(async move {
			let (mut stream, _) = listener.accept().await.unwrap();
			decode_msg(&mut stream).await
		});

		let mut client = TcpStream::connect(&addr).await.unwrap();
		// Declare a length one byte over the cap; decode must bail on the length
		// prefix alone, before allocating or reading any body bytes.
		let oversized = (GOSSIP_MAX_FRAME_BYTES as u32 + 1).to_be_bytes();
		client.write_all(&oversized).await.unwrap();
		client.flush().await.unwrap();

		assert!(
			server.await.unwrap().is_none(),
			"an oversized frame is rejected"
		);
	}
}
