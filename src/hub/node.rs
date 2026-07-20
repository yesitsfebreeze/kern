use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use trnsprt::kern_rpc::KernRpcClient;
use trnsprt::typed::{Endpoint, JsonEnvelopeCodec};

// Bootstrap loads the whole graph before binding kern.sock, so a big store
// needs a generous ready window.
const READY_RETRIES: u32 = 40;
const READY_DELAY_MS: u64 = 250;

pub struct NodeHandle {
	pub root: PathBuf,
	pub endpoint: Endpoint,
	// None = adopted: a daemon someone else started owns the socket.
	pub child: Option<Child>,
}

impl NodeHandle {
	pub fn pid(&self) -> u32 {
		self.child.as_ref().map(|c| c.id()).unwrap_or(0)
	}

	pub fn alive(&mut self) -> bool {
		match self.child.as_mut() {
			Some(child) => matches!(child.try_wait(), Ok(None)),
			None => true,
		}
	}
}

// None = unreachable; Some(0) also means "treat as active" — daemons predating
// the field report 0 and must never be idle-unloaded on a lie.
pub async fn idle_ms(endpoint: &Endpoint) -> Option<u64> {
	let client = KernRpcClient::<JsonEnvelopeCodec>::connect_endpoint_with_retry(
		endpoint,
		1,
		Duration::from_millis(0),
	)
	.await
	.ok()?;
	let res = client.health().await.ok()?;
	Some(res.idle_ms)
}

pub async fn probe(endpoint: &Endpoint) -> bool {
	KernRpcClient::<JsonEnvelopeCodec>::connect_endpoint_with_retry(
		endpoint,
		1,
		Duration::from_millis(0),
	)
	.await
	.is_ok()
}

// A rebuild unlinks the running binary; /proc/self/exe then reads
// "<path> (deleted)" and spawning it ENOENTs. The fresh binary sits at the
// original path — strip the marker so a long-lived hub keeps spawning nodes.
fn strip_deleted_marker(s: &str) -> &str {
	s.strip_suffix(" (deleted)").unwrap_or(s)
}

fn self_exe() -> Result<PathBuf, String> {
	let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
	let s = exe.to_string_lossy();
	let stripped = strip_deleted_marker(&s);
	if stripped.len() != s.len() {
		return Ok(PathBuf::from(stripped));
	}
	Ok(exe)
}

pub async fn spawn(root: &Path) -> Result<NodeHandle, String> {
	let endpoint = Endpoint::kern_for(root);
	if probe(&endpoint).await {
		return Ok(NodeHandle {
			root: root.to_path_buf(),
			endpoint,
			child: None,
		});
	}
	let exe = self_exe()?;
	let child = Command::new(exe)
		.arg("--daemon")
		.current_dir(root)
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.spawn()
		.map_err(|e| format!("spawn node for {}: {e}", root.display()))?;
	for _ in 0..READY_RETRIES {
		if probe(&endpoint).await {
			return Ok(NodeHandle {
				root: root.to_path_buf(),
				endpoint,
				child: Some(child),
			});
		}
		tokio::time::sleep(Duration::from_millis(READY_DELAY_MS)).await;
	}
	Err(format!(
		"node for {} never bound {}",
		root.display(),
		endpoint.display()
	))
}

pub async fn shutdown(handle: &mut NodeHandle) -> Result<(), String> {
	if let Ok(client) = KernRpcClient::<JsonEnvelopeCodec>::connect_endpoint_with_retry(
		&handle.endpoint,
		1,
		Duration::from_millis(0),
	)
	.await
	{
		let _ = client.shutdown().await;
	}
	let Some(child) = handle.child.as_mut() else {
		return Ok(());
	};
	for _ in 0..READY_RETRIES {
		if let Ok(Some(_)) = child.try_wait() {
			return Ok(());
		}
		tokio::time::sleep(Duration::from_millis(READY_DELAY_MS)).await;
	}
	// Graceful path stalled — the node's flush already had ~10s; kill beats a
	// zombie holding the socket.
	child.kill().map_err(|e| format!("kill node: {e}"))?;
	let _ = child.wait();
	Ok(())
}

#[cfg(test)]
mod self_exe_tests {
	use super::*;

	#[test]
	fn deleted_marker_is_stripped_only_as_suffix() {
		assert_eq!(
			strip_deleted_marker("/x/kern (deleted)"),
			"/x/kern",
			"rebuilt binary path recovers"
		);
		assert_eq!(strip_deleted_marker("/x/kern"), "/x/kern");
		assert_eq!(
			strip_deleted_marker("/x/kern (deleted)/sub"),
			"/x/kern (deleted)/sub",
			"marker inside the path is a real directory name, not a marker"
		);
	}

	#[test]
	fn self_exe_resolves_to_an_existing_binary() {
		let exe = self_exe().unwrap();
		assert!(exe.exists(), "test runner binary must exist: {exe:?}");
	}
}
