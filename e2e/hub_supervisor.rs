// End-to-end hub lifecycle over a real socket, isolated from any user hub by a
// private XDG_RUNTIME_DIR. Unix-only: named pipes have no per-test namespace.
#![cfg(unix)]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn kern() -> Command {
	Command::new(env!("CARGO_BIN_EXE_kern"))
}

struct HubGuard(Child);

impl Drop for HubGuard {
	fn drop(&mut self) {
		let _ = self.0.kill();
		let _ = self.0.wait();
	}
}

fn start_hub_with(runtime_dir: &Path, cwd: &Path, extra: &[&str]) -> HubGuard {
	let child = kern()
		.arg("hub")
		.args(extra)
		.env("XDG_RUNTIME_DIR", runtime_dir)
		.current_dir(cwd)
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.spawn()
		.expect("spawn hub");
	let sock = runtime_dir.join("kern-hub.sock");
	let deadline = Instant::now() + Duration::from_secs(10);
	while !sock.exists() {
		assert!(Instant::now() < deadline, "hub never bound {sock:?}");
		std::thread::sleep(Duration::from_millis(50));
	}
	HubGuard(child)
}

fn start_hub(runtime_dir: &Path, cwd: &Path) -> HubGuard {
	start_hub_with(runtime_dir, cwd, &[])
}

fn run(runtime_dir: &Path, cwd: &Path, args: &[&str]) -> (String, String) {
	let out = kern()
		.args(args)
		.env("XDG_RUNTIME_DIR", runtime_dir)
		.current_dir(cwd)
		.output()
		.expect("run kern");
	(
		String::from_utf8_lossy(&out.stdout).to_string(),
		String::from_utf8_lossy(&out.stderr).to_string(),
	)
}

#[test]
fn hub_binds_serves_status_and_rejects_bogus_unload() {
	let dir = tempfile::tempdir().unwrap();
	let runtime = dir.path().join("run");
	let cwd = dir.path().join("proj");
	std::fs::create_dir_all(&runtime).unwrap();
	std::fs::create_dir_all(cwd.join(".kern")).unwrap();
	let _hub = start_hub(&runtime, &cwd);

	let (stdout, _) = run(&runtime, &cwd, &["hub", "status"]);
	assert!(
		stdout.contains("no nodes"),
		"fresh hub tracks nothing: {stdout}"
	);

	let (_, stderr) = run(
		&runtime,
		&cwd,
		&["hub", "unload", "/nonexistent/kern-hub-test"],
	);
	assert!(
		stderr.contains("/nonexistent/kern-hub-test"),
		"bogus root is rejected with the path named: {stderr}"
	);

	// Unload of a real-but-nodeless root is a clean no-op.
	let (stdout, stderr) = run(&runtime, &cwd, &["hub", "unload"]);
	assert!(
		stdout.contains("no node for"),
		"nodeless unload is a no-op: out={stdout} err={stderr}"
	);
}

#[test]
fn resolve_spawns_a_node_and_unload_reaps_it() {
	let dir = tempfile::tempdir().unwrap();
	let runtime = dir.path().join("run");
	let cwd = dir.path().join("proj");
	std::fs::create_dir_all(&runtime).unwrap();
	std::fs::create_dir_all(cwd.join(".kern")).unwrap();
	let _hub = start_hub(&runtime, &cwd);

	let (stdout, stderr) = run(&runtime, &cwd, &["hub", "resolve"]);
	assert!(
		stdout.contains("spawned"),
		"first resolve cold-boots a node: out={stdout} err={stderr}"
	);

	let (stdout, _) = run(&runtime, &cwd, &["hub", "resolve"]);
	assert!(
		stdout.contains("running"),
		"second resolve reuses the node: {stdout}"
	);

	let (stdout, _) = run(&runtime, &cwd, &["hub", "status"]);
	assert!(
		stdout.contains("up") && stdout.contains("kern-"),
		"status lists the live node: {stdout}"
	);

	let (stdout, stderr) = run(&runtime, &cwd, &["hub", "unload"]);
	assert!(
		stdout.contains("unloaded"),
		"unload succeeds: out={stdout} err={stderr}"
	);

	// The node's socket must be gone — graceful exit removes it.
	let deadline = Instant::now() + Duration::from_secs(10);
	loop {
		let node_socks = std::fs::read_dir(&runtime)
			.unwrap()
			.filter_map(|e| e.ok())
			.filter(|e| {
				let n = e.file_name().to_string_lossy().to_string();
				n.starts_with("kern-") && n != "kern-hub.sock"
			})
			.count();
		if node_socks == 0 {
			break;
		}
		assert!(
			Instant::now() < deadline,
			"node socket lingered after unload"
		);
		std::thread::sleep(Duration::from_millis(100));
	}
}

#[test]
fn an_idle_node_is_auto_unloaded() {
	let dir = tempfile::tempdir().unwrap();
	let runtime = dir.path().join("run");
	let cwd = dir.path().join("proj");
	std::fs::create_dir_all(&runtime).unwrap();
	std::fs::create_dir_all(cwd.join(".kern")).unwrap();
	let _hub = start_hub_with(&runtime, &cwd, &["--idle-unload-secs", "2"]);

	let (stdout, stderr) = run(&runtime, &cwd, &["hub", "resolve"]);
	assert!(
		stdout.contains("spawned"),
		"resolve boots a node: out={stdout} err={stderr}"
	);

	// Never queried after boot -> idle from startup; the 2s threshold with the
	// matched reaper cadence must clear it well inside the window.
	let deadline = Instant::now() + Duration::from_secs(30);
	loop {
		let (stdout, _) = run(&runtime, &cwd, &["hub", "status"]);
		if stdout.contains("no nodes") {
			break;
		}
		assert!(
			Instant::now() < deadline,
			"idle node was never unloaded; status: {stdout}"
		);
		std::thread::sleep(Duration::from_millis(500));
	}
}

#[test]
fn a_second_hub_refuses_the_taken_socket() {
	let dir = tempfile::tempdir().unwrap();
	let runtime = dir.path().join("run");
	let cwd = dir.path().join("proj");
	std::fs::create_dir_all(&runtime).unwrap();
	std::fs::create_dir_all(cwd.join(".kern")).unwrap();
	let _hub = start_hub(&runtime, &cwd);

	let out = kern()
		.args(["hub"])
		.env("XDG_RUNTIME_DIR", &runtime)
		.current_dir(&cwd)
		.output()
		.expect("second hub");
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		stderr.contains("already running"),
		"second hub must refuse and exit: {stderr}"
	);
}

#[test]
fn kern_mcp_auto_starts_the_hub_and_routes_through_it() {
	let dir = tempfile::tempdir().unwrap();
	let runtime = dir.path().join("run");
	let cwd = dir.path().join("proj");
	std::fs::create_dir_all(&runtime).unwrap();
	std::fs::create_dir_all(cwd.join(".kern")).unwrap();

	// One MCP handshake with NO hub running: the proxy must auto-start one,
	// resolve through it, and answer initialize.
	let mut child = kern()
		.arg("mcp")
		.env("XDG_RUNTIME_DIR", &runtime)
		.current_dir(&cwd)
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::null())
		.spawn()
		.expect("spawn kern mcp");
	{
		use std::io::Write;
		let stdin = child.stdin.as_mut().unwrap();
		writeln!(
			stdin,
			r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{}}}}"#
		)
		.unwrap();
	}
	// Reading the initialize response proves the whole attach path ran:
	// hub auto-started, node resolved (spawned), proxy connected.
	{
		use std::io::{BufRead, BufReader};
		let stdout = child.stdout.take().unwrap();
		let mut line = String::new();
		BufReader::new(stdout)
			.read_line(&mut line)
			.expect("read initialize response");
		assert!(
			line.contains(r#""protocolVersion""#),
			"proxy must answer initialize: {line}"
		);
	}
	assert!(
		runtime.join("kern-hub.sock").exists(),
		"kern mcp must leave a machine hub behind"
	);
	let _ = child.kill();
	let _ = child.wait();

	// The hub (not the proxy) owns the node: status from a fresh process sees it.
	let deadline = Instant::now() + Duration::from_secs(15);
	loop {
		let (stdout, _) = run(&runtime, &cwd, &["hub", "status"]);
		if stdout.contains("up") {
			break;
		}
		assert!(
			Instant::now() < deadline,
			"hub never tracked the node: {stdout}"
		);
		std::thread::sleep(Duration::from_millis(250));
	}

	// Cleanup: unload the node and stop the detached hub so nothing outlives the test.
	let (_, _) = run(&runtime, &cwd, &["hub", "unload"]);
	let (stdout, stderr) = run(&runtime, &cwd, &["hub", "stop"]);
	assert!(
		stdout.contains("hub stopped"),
		"detached hub must stop via RPC: out={stdout} err={stderr}"
	);
}

#[test]
fn status_without_a_hub_fails_softly() {
	let dir = tempfile::tempdir().unwrap();
	let runtime = dir.path().join("run");
	let cwd = dir.path().join("proj");
	std::fs::create_dir_all(&runtime).unwrap();
	std::fs::create_dir_all(cwd.join(".kern")).unwrap();

	let (_, stderr) = run(&runtime, &cwd, &["hub", "status"]);
	assert!(
		stderr.contains("not running"),
		"no hub -> soft failure, not a hang: {stderr}"
	);
}
