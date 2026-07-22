use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::typed::{AdapterError, Channel, JsonEnvelopeCodec};

/// The one frame a caller sends before any `KernRpc` method is reachable.
///
/// `token` is the per-graph secret the daemon minted (`resolve_mcp_token`) —
/// the same `mcp-token` the HTTP surface demands, never a second one.
///
/// `principal` is *declared*, not proven. The socket is `0600` and the CLI, the
/// `kern mcp` proxy and the hub all run as the same uid, so nothing on this
/// connection can distinguish them: the secret proves "this uid", the principal
/// says which of that uid's programs is talking. It is recorded, not enforced —
/// items 9 and 18 are what will decide a principal's rights.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuthReq {
	pub token: String,
	#[serde(default)]
	pub principal: String,
}

/// A human at the terminal drove this.
pub const PRINCIPAL_CLI: &str = "cli";
/// An agent drove this, through `kern mcp`'s stdio proxy.
pub const PRINCIPAL_MCP: &str = "mcp";
/// The machine hub, on its own behalf (probe, idle poll, unload).
pub const PRINCIPAL_HUB: &str = "hub";

// One message for every refusal. A missing frame, a malformed frame and a wrong
// token must read identically, or the reply becomes an oracle that tells a
// caller how far it got.
const REFUSED: &str = "kern.sock: unauthenticated";

impl AuthReq {
	pub fn new(token: impl Into<String>, principal: &str) -> Self {
		Self {
			token: token.into(),
			principal: principal.to_string(),
		}
	}
}

// Constant time over the compared bytes: a compare that returns at the first
// mismatch reports how long a shared prefix was.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
	if a.len() != b.len() {
		return false;
	}
	let mut diff = 0u8;
	for (x, y) in a.iter().zip(b) {
		diff |= x ^ y;
	}
	diff == 0
}

/// Client half: present the token, then wait for the daemon's verdict.
/// Anything but an explicit `ok: true` is a refusal.
pub async fn present_auth(
	channel: &mut Channel<JsonEnvelopeCodec>,
	auth: &AuthReq,
) -> Result<(), AdapterError> {
	let frame = serde_json::json!({ "auth": auth });
	channel.send(frame).await?;
	match channel.recv().await {
		Ok(Some(reply)) if reply.pointer("/auth/ok").and_then(Value::as_bool) == Some(true) => Ok(()),
		_ => Err(AdapterError::Unauthenticated(REFUSED.to_string())),
	}
}

/// Server half: read the caller's auth frame and verify it. Returns the
/// declared principal.
///
/// Every other outcome is a refusal — EOF, a codec error, a frame that is not
/// an auth frame, a token that does not match, and an `expected` that is itself
/// empty. There is no branch here that returns `Ok` without having compared a
/// non-empty secret, which is the whole point: a gate that fails open reads as
/// protection while being none.
pub async fn verify_auth(
	channel: &mut Channel<JsonEnvelopeCodec>,
	expected: &str,
) -> Result<String, AdapterError> {
	let req = match channel.recv().await {
		Ok(Some(frame)) => frame
			.get("auth")
			.cloned()
			.and_then(|v| serde_json::from_value::<AuthReq>(v).ok()),
		_ => None,
	};
	match req {
		Some(req) if !expected.is_empty() && ct_eq(req.token.as_bytes(), expected.as_bytes()) => {
			channel
				.send(serde_json::json!({ "auth": { "ok": true } }))
				.await?;
			Ok(req.principal)
		}
		_ => {
			// Best-effort: say no out loud so a misconfigured client reports a
			// refusal instead of a bare EOF. The refusal stands either way.
			let _ = channel
				.send(serde_json::json!({ "auth": { "ok": false, "error": REFUSED } }))
				.await;
			Err(AdapterError::Unauthenticated(REFUSED.to_string()))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::typed::InprocAdapter;

	fn pair() -> (Channel<JsonEnvelopeCodec>, Channel<JsonEnvelopeCodec>) {
		let (a, b) = InprocAdapter::pair();
		(
			Channel::new(a, JsonEnvelopeCodec::new()),
			Channel::new(b, JsonEnvelopeCodec::new()),
		)
	}

	#[test]
	fn ct_eq_agrees_with_plain_equality_including_the_prefix_case() {
		assert!(ct_eq(b"abc", b"abc"));
		assert!(!ct_eq(b"abc", b"abd"));
		assert!(!ct_eq(b"abc", b"ab"), "a shared prefix is not a match");
		assert!(!ct_eq(b"", b"a"));
		assert!(ct_eq(b"", b""), "equal lengths, no differing bytes");
	}

	#[tokio::test]
	async fn the_right_token_verifies_and_hands_back_the_declared_principal() {
		let (mut server, mut client) = pair();
		let task = tokio::spawn(async move { verify_auth(&mut server, "s3cret").await });
		present_auth(&mut client, &AuthReq::new("s3cret", PRINCIPAL_CLI))
			.await
			.expect("the right token is accepted");
		assert_eq!(task.await.unwrap().unwrap(), PRINCIPAL_CLI);
	}

	// `s3crey` is the load-bearing case, not `guess`. A wrong token of a
	// *different* length is refused by `ct_eq`'s length check alone, so a suite
	// that only ever offers one never runs the byte compare at all — delete the
	// compare's body and every such test still passes. `s3crey` is the same
	// length as `s3cret` and differs in the last byte, so it can only be refused
	// by the compare, and only by one that reads to the end.
	#[tokio::test]
	async fn a_wrong_token_is_refused_on_both_halves() {
		for offered in ["guess", "s3crey"] {
			let (mut server, mut client) = pair();
			let task = tokio::spawn(async move { verify_auth(&mut server, "s3cret").await });
			let out = present_auth(&mut client, &AuthReq::new(offered, PRINCIPAL_CLI)).await;
			assert!(
				matches!(out, Err(AdapterError::Unauthenticated(_))),
				"the client must learn it was refused, not that nothing was there (offered {offered:?})"
			);
			assert!(task.await.unwrap().is_err(), "offered {offered:?}");
		}
	}

	#[tokio::test]
	async fn a_frame_that_is_not_an_auth_frame_is_refused() {
		let (mut server, mut client) = pair();
		let task = tokio::spawn(async move { verify_auth(&mut server, "s3cret").await });
		client
			.send(serde_json::json!({"id": 1, "method": "call_tool", "params": {}}))
			.await
			.unwrap();
		assert!(
			task.await.unwrap().is_err(),
			"a caller that skips the handshake is a caller with no identity"
		);
	}

	#[tokio::test]
	async fn an_auth_frame_with_no_token_field_is_refused() {
		let (mut server, mut client) = pair();
		let task = tokio::spawn(async move { verify_auth(&mut server, "s3cret").await });
		client
			.send(serde_json::json!({"auth": {"principal": "cli"}}))
			.await
			.unwrap();
		assert!(task.await.unwrap().is_err(), "no token is not a token");
	}

	#[tokio::test]
	async fn a_hung_up_caller_is_refused_rather_than_admitted() {
		let (mut server, client) = pair();
		drop(client);
		assert!(
			verify_auth(&mut server, "s3cret").await.is_err(),
			"EOF before the handshake must fail closed"
		);
	}

	// The daemon-side degenerate case. If the secret could not be read, the
	// expected token is empty — and an empty expectation must reject everyone,
	// including a caller that helpfully sends an empty token.
	#[tokio::test]
	async fn an_empty_expected_token_authenticates_nobody() {
		for offered in ["", "anything"] {
			let (mut server, mut client) = pair();
			let task = tokio::spawn(async move { verify_auth(&mut server, "").await });
			let _ = present_auth(&mut client, &AuthReq::new(offered, PRINCIPAL_CLI)).await;
			assert!(
				task.await.unwrap().is_err(),
				"a daemon with no secret must serve nobody, not everybody (offered {offered:?})"
			);
		}
	}
}
