//! `kern mcp` subcommand: serve stdio MCP.
//!
//! Two modes auto-selected:
//!
//! - **Proxy** (preferred): if a kern singleton is already running at
//!   `kern.sock`, attach as a `KernRpcClient` and forward every stdio
//!   MCP `tools/call` over kern.sock via the typed `call_tool` escape
//!   hatch. The proxy holds no graph, no tick worker, no ingest queue —
//!   every heavy bit lives in the daemon.
//!
//! - **Standalone** (fallback): no daemon is reachable, so load a full
//!   graph + worker + tick locally and serve stdio MCP directly from
//!   them. Matches the pre-singleton behavior so external MCP clients
//!   (Claude Desktop, etc.) keep working when no daemon is up.

use std::sync::Arc;
use std::sync::RwLock as StdRwLock;

use tokio::sync::Mutex as TokioMutex;
use trnsprt::kern_rpc::{CallToolReq, KernRpcClient};
use trnsprt::typed::{AdapterError, JsonEnvelopeCodec};
use trnsprt::{McpError, McpServer, ToolResult, ToolSchema};

use crate::base::locks::read_recovered;

use super::{load_graph, save_graph};

pub(super) async fn cmd_mcp(cfg: &crate::config::Config) {
	// First attach attempt — short retry catches the "daemon up but
	// slow to respond" race.
	match attach_with_retry(2, 150).await {
		Ok(client) => {
			run_proxy(client).await;
		}
		Err(e_first) => {
			tracing::info!(
				target: "kern.mcp",
				error = %e_first,
				"no daemon at kern.sock — auto-spawning detached daemon"
			);
			match spawn_daemon() {
				Ok(()) => match attach_with_retry(6, 150).await {
					Ok(client) => {
						tracing::info!(
							target: "kern.mcp_proxy",
							"attached to auto-spawned daemon — proxy mode"
						);
						run_proxy(client).await;
					}
					Err(e_retry) => {
						tracing::warn!(
							target: "kern.mcp",
							error = %e_retry,
							"auto-spawn failed, falling back to standalone"
						);
						run_standalone(cfg).await;
					}
				},
				Err(e_spawn) => {
					tracing::warn!(
						target: "kern.mcp",
						error = %e_spawn,
						"auto-spawn failed, falling back to standalone"
					);
					run_standalone(cfg).await;
				}
			}
		}
	}
}

/// Ensure a kern daemon is running for this cwd, spawning a detached one if not.
///
/// Probes `kern.sock`: if a daemon already answers, returns at once. Otherwise
/// spawns a detached `kern --daemon` (null stdio, outlives us — same mechanism
/// `kern mcp` uses) and waits a short, bounded window for its socket to come up.
///
/// Called by the mux at launch so every spawned pane's `kern mcp` bridge
/// attaches to one warm shared daemon (proxy mode) instead of each cold-spawning
/// its own and timing out — the "failed to reconnect" symptom.
///
/// Returns `true` if a daemon was already running, `false` if one was spawned
/// (or the spawn failed — see logs).
pub(crate) async fn ensure_daemon() -> bool {
	// Fast path: a daemon already serving kern.sock needs nothing more.
	if attach_with_retry(1, 0).await.is_ok() {
		return true;
	}
	// None reachable — spawn a detached daemon and give its socket a brief
	// head start so the first pane attaches in proxy mode rather than racing to
	// cold-spawn a second daemon. ~2s budget; if it's not up by then we proceed
	// anyway and the pane's own attach-retry covers the remainder.
	if let Err(e) = spawn_daemon() {
		tracing::warn!(target: "kern.mcp", error = %e, "ensure_daemon: spawn failed");
		return false;
	}
	match attach_with_retry(8, 250).await {
		Ok(_) => tracing::info!(target: "kern.mcp", "ensure_daemon: daemon up"),
		Err(e) => tracing::warn!(
			target: "kern.mcp",
			error = %e,
			"ensure_daemon: daemon not ready in budget; panes will retry"
		),
	}
	false
}

async fn run_proxy(client: KernRpcClient<JsonEnvelopeCodec>) {
	tracing::info!(
		target: "kern.mcp_proxy",
		"attached to running daemon — proxy mode"
	);
	let proxy = ProxyServer {
		client: Arc::new(TokioMutex::new(client)),
	};
	// `serve_stdio` is sync (BufRead/Write on stdin/stdout). Run
	// it on a blocking thread so it doesn't park a runtime worker.
	// Each `call_tool` invocation crosses back into async via
	// `block_in_place` + `Handle::current().block_on`, which is
	// supported on the multi-thread runtime kern uses.
	if let Err(e) = tokio::task::spawn_blocking(move || trnsprt::serve_stdio(&proxy)).await
	{
		tracing::warn!(target: "kern.mcp_proxy", error = %e, "stdio loop");
	}
}

async fn attach_with_retry(
	retries: u32,
	delay_ms: u64,
) -> Result<KernRpcClient<JsonEnvelopeCodec>, AdapterError> {
	let mut last_err: Option<AdapterError> = None;
	for i in 0..retries {
		match KernRpcClient::<JsonEnvelopeCodec>::connect_local().await {
			Ok(c) => return Ok(c),
			Err(e) => {
				last_err = Some(e);
				if i + 1 < retries {
					tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
				}
			}
		}
	}
	Err(last_err.unwrap_or_else(|| AdapterError::Other("no attempts".into())))
}

#[cfg(windows)]
fn spawn_daemon() -> std::io::Result<()> {
	use std::os::windows::process::CommandExt;
	use std::process::{Command, Stdio};
	// DETACHED_PROCESS = 0x00000008, CREATE_NEW_PROCESS_GROUP = 0x00000200
	const DETACHED_PROCESS: u32 = 0x0000_0008;
	const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
	let exe = std::env::current_exe()?;
	let _child = Command::new(exe)
		.arg("--daemon")
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
		.spawn()?;
	// Drop child handle — detach flags + null stdio keep it alive past
	// our exit.
	Ok(())
}

#[cfg(unix)]
fn spawn_daemon() -> std::io::Result<()> {
	use std::os::unix::process::CommandExt;
	use std::process::{Command, Stdio};
	let exe = std::env::current_exe()?;
	let _child = Command::new(exe)
		.arg("--daemon")
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.process_group(0)
		.spawn()?;
	Ok(())
}

// ---- Proxy ---------------------------------------------------------------

struct ProxyServer {
	client: Arc<TokioMutex<KernRpcClient<JsonEnvelopeCodec>>>,
}

impl McpServer for ProxyServer {
	fn server_name(&self) -> &str {
		"kern"
	}
	fn server_version(&self) -> &str {
		env!("CARGO_PKG_VERSION")
	}

	fn tools_list(&self) -> Vec<ToolSchema> {
		// Tool schema is static — no graph state needed. Serve directly
		// from the same source the standalone path uses so a proxy and
		// a standalone instance advertise byte-identical tool lists.
		crate::mcp::tools::tool_definitions()
			.into_iter()
			.filter_map(|v| serde_json::from_value(v).ok())
			.collect()
	}

	fn call_tool(
		&self,
		name: &str,
		args: &serde_json::Value,
	) -> Result<ToolResult, McpError> {
		let client = self.client.clone();
		let req = CallToolReq {
			name: name.to_string(),
			args: args.clone(),
		};
		let res = tokio::task::block_in_place(|| {
			tokio::runtime::Handle::current().block_on(async move {
				let c = client.lock().await;
				c.call_tool(req).await
			})
		})
		.map_err(|e| McpError::Rpc {
			code: -32000,
			message: format!("kern_rpc call_tool: {e}"),
		})?;

		Ok(tool_result_from_envelope(&res.envelope))
	}

	fn extra_capabilities(&self) -> serde_json::Value {
		// Match the standalone server so a client probing capabilities
		// can't tell the two apart. Resources/prompts handlers fall
		// through to method-not-found until they're proxied too
		// (follow-up: route resources/* via a future KernRpc method).
		serde_json::json!({"resources": {}, "prompts": {}})
	}
}

/// Map a kern_rpc `call_tool` reply envelope to an MCP [`ToolResult`]: take the
/// `content` array (empty when absent or not an array) and the `isError` flag
/// (false when absent). Pure, so the envelope decoding is testable without a
/// live kern.sock connection.
fn tool_result_from_envelope(envelope: &serde_json::Value) -> ToolResult {
	let content = envelope
		.get("content")
		.and_then(|v| v.as_array())
		.cloned()
		.unwrap_or_default();
	let is_error = envelope
		.get("isError")
		.and_then(|v| v.as_bool())
		.unwrap_or(false);
	ToolResult {
		content,
		is_error,
		structured_content: None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn envelope_extracts_content_array_and_error_flag() {
		let env = json!({ "content": [{ "type": "text", "text": "hi" }], "isError": true });
		let r = tool_result_from_envelope(&env);
		assert!(r.is_error);
		assert_eq!(r.content.len(), 1);
		assert_eq!(r.content[0]["text"], "hi");
		assert!(r.structured_content.is_none());
	}

	#[test]
	fn envelope_missing_fields_default_to_empty_and_ok() {
		let r = tool_result_from_envelope(&json!({}));
		assert!(!r.is_error, "missing isError defaults to false");
		assert!(r.content.is_empty(), "missing content defaults to empty");
	}

	#[test]
	fn envelope_non_array_content_falls_back_to_empty() {
		let r = tool_result_from_envelope(&json!({ "content": "oops", "isError": false }));
		assert!(r.content.is_empty(), "a non-array content is ignored, not panicked on");
	}
}

// ---- Standalone (legacy heavy path) --------------------------------------

async fn run_standalone(cfg: &crate::config::Config) {
	let g = Arc::new(StdRwLock::new(load_graph(cfg)));
	let llm_client = super::server_llm_client(cfg, cfg.reason_url(), &cfg.reason.model);
	let save_g = g.clone();
	let save_fn: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
		let g = read_recovered(&save_g);
		save_graph(&g);
	});
	let llm_fn: Option<crate::ingest::LlmFunc> = if !cfg.reason_url().is_empty() {
		Some(Arc::new(llm_client.complete_func()))
	} else {
		None
	};
	let worker = Arc::new(crate::ingest::Worker::new(
		g.clone(),
		llm_client.clone(),
		llm_fn,
		Some(save_fn.clone()),
	));

	let q = Arc::new(crate::tick::queue::Queue::new(512));
	let tick_llm: crate::tick::tasks::LlmFunc = Arc::new(llm_client.complete_func());
	let tick_embed: crate::tick::tasks::EmbedFunc = {
		let c = llm_client.clone();
		Arc::new(move |text: &str| -> Result<Vec<f64>, String> {
			let c = c.clone();
			let text = text.to_string();
			match tokio::runtime::Handle::try_current() {
				Ok(h) => {
					let result = std::thread::scope(|_| h.block_on(c.embed(&text)));
					result.map_err(|e: crate::llm::LlmError| e.to_string())
				}
				Err(_) => Err("no runtime".to_string()),
			}
		})
	};
	crate::tick::start(
		q.clone(),
		g.clone(),
		crate::tick::TickContext {
			llm: Some(tick_llm),
			embed: Some(tick_embed),
			broadcast_q: None,
			gnn_cfg: cfg.gnn.into(),
			tick_cfg: cfg.tick,
		},
	);

	let server = crate::mcp::Server {
		graph: g,
		worker,
		llm: Some(llm_client),
		save_fn,
		task_q: Some(q),
		cfg: Arc::new(cfg.clone()),
		cache: crate::retrieval::cache::QueryCache::shared(
			cfg.retrieval.query_cache_cap,
			cfg.retrieval.query_cache_theta,
		),
	};
	server.run_stdio();
}

// ── Claude Code MCP auto-registration ────────────────────────────────────────

/// Ensure the kern MCP server is registered in the project's `.mcp.json`.
///
/// `.mcp.json` is Claude Code's project-level MCP config file (sits at the
/// project root alongside `CLAUDE.md`). Called at daemon/mux startup so any
/// project directory that runs kern automatically gains `mcp__kern__*` tools
/// in the Claude Code session without manual setup. Idempotent — exits
/// immediately if the `mcpServers.kern` entry is already present. Does not
/// touch any other key.
pub(crate) fn ensure_mcp_registered(cwd: &std::path::Path) {
    let mcp_path = cwd.join(".mcp.json");

    // Read existing JSON or start from an empty object.
    let raw = std::fs::read_to_string(&mcp_path).unwrap_or_else(|_| "{}".to_string());
    let mut root: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::json!({}));

    // If kern is already registered, nothing to do.
    if root
        .get("mcpServers")
        .and_then(|s| s.get("kern"))
        .is_some()
    {
        return;
    }

    // Upsert the mcpServers.kern entry.
    root.as_object_mut()
        .map(|obj| {
            obj.entry("mcpServers")
                .or_insert_with(|| serde_json::json!({}))
                .as_object_mut()
                .map(|servers| {
                    servers.insert(
                        "kern".to_string(),
                        serde_json::json!({
                            "command": "kern",
                            "args":    ["mcp"]
                        }),
                    )
                })
        });

    match serde_json::to_string_pretty(&root) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&mcp_path, json) {
                tracing::warn!(target: "kern.mcp", error = %e, "ensure_mcp_registered: write failed");
            } else {
                tracing::info!(
                    target: "kern.mcp",
                    path = %mcp_path.display(),
                    "registered kern MCP server in .mcp.json"
                );
            }
        }
        Err(e) => tracing::warn!(target: "kern.mcp", error = %e, "ensure_mcp_registered: serialize failed"),
    }
}

#[cfg(test)]
mod ensure_mcp_tests {
    use super::*;

    #[test]
    fn writes_kern_entry_when_file_absent() {
        let dir = tempfile::tempdir().unwrap();
        ensure_mcp_registered(dir.path());
        let raw = std::fs::read_to_string(dir.path().join(".mcp.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["mcpServers"]["kern"]["command"], "kern");
        assert_eq!(v["mcpServers"]["kern"]["args"][0], "mcp");
    }

    #[test]
    fn preserves_existing_keys_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let mcp = dir.path().join(".mcp.json");
        // Seed with an unrelated mcpServer.
        std::fs::write(
            &mcp,
            r#"{"mcpServers":{"other":{"command":"other"}}}"#,
        )
        .unwrap();

        ensure_mcp_registered(dir.path());

        let raw = std::fs::read_to_string(&mcp).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        // Existing entry preserved.
        assert_eq!(v["mcpServers"]["other"]["command"], "other");
        // New kern entry added.
        assert_eq!(v["mcpServers"]["kern"]["command"], "kern");

        // Second call is idempotent — file unchanged.
        let before = std::fs::read_to_string(&mcp).unwrap();
        ensure_mcp_registered(dir.path());
        let after = std::fs::read_to_string(&mcp).unwrap();
        assert_eq!(before, after, "idempotent: file unchanged on second call");
    }

    #[test]
    fn does_not_overwrite_existing_kern_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mcp = dir.path().join(".mcp.json");
        std::fs::write(
            &mcp,
            r#"{"mcpServers":{"kern":{"command":"custom-kern","args":["custom"]}}}"#,
        )
        .unwrap();

        ensure_mcp_registered(dir.path());

        let raw = std::fs::read_to_string(&mcp).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        // Custom entry must not be overwritten.
        assert_eq!(v["mcpServers"]["kern"]["command"], "custom-kern");
        assert_eq!(v["mcpServers"]["kern"]["args"][0], "custom");
    }
}
