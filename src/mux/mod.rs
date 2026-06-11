//! Mux PTY-multiplexer TUI — default `kern` launch mode.
//!
//! `run_mux` starts the TUI and a background MCP server task that exposes
//! the four `mux_*` tools on a TCP loopback socket.

pub mod delegate;
pub mod mcp;
pub mod pty;
pub mod registry;
pub mod tui;

pub use delegate::{boot_message, kern_ingest_text, result_key, task_key, DelegateSpec};
pub use mcp::MuxMcpServer;
pub use pty::{new_session_id, PtySession};
pub use registry::{PaneRegistry, SharedRegistry};
pub use tui::run_tui;

use std::sync::{Arc, Mutex};

use crate::config::{Config, KeyMap};

/// Launch the mux TUI.
///
/// 1. Starts the MCP server on `cfg.mux.mcp_addr` in a background task.
/// 2. Runs the TUI render loop (blocks until quit).
/// 3. Cancels the MCP task on return.
pub async fn run_mux(cfg: &Config) {
    // Determine terminal size (fall back to 80×24 if detection fails).
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let pane_rows    = rows.saturating_sub(1);

    let registry = match PaneRegistry::new(cfg.mux.agent_cmd.clone(), cols / 2, pane_rows) {
        Ok(r)  => Arc::new(Mutex::new(r)),
        Err(e) => {
            eprintln!("kern mux: failed to spawn main pane: {e}");
            return;
        }
    };

    // ── MCP server ────────────────────────────────────────────────────────
    let mcp_addr   = cfg.mux.mcp_addr.clone();
    let agent_cmd  = cfg.mux.agent_cmd.clone();
    let reg_mcp    = registry.clone();
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(&mcp_addr).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(target: "kern.mux", addr = %mcp_addr, error = %e, "mux MCP bind failed");
                return;
            }
        };
        tracing::info!(target: "kern.mux", addr = %mcp_addr, "mux MCP listening");

        let mut cancel_rx = cancel_rx;
        loop {
            tokio::select! {
                _ = &mut cancel_rx => break,
                result = listener.accept() => {
                    let Ok((stream, _)) = result else { continue };
                    let Ok(std_stream) = stream.into_std() else { continue };
                    let reg  = reg_mcp.clone();
                    let cmd  = agent_cmd.clone();
                    std::thread::spawn(move || {
                        let server = MuxMcpServer { registry: reg, agent_cmd: cmd };
                        let reader_stream = match std_stream.try_clone() {
                            Ok(s) => s,
                            Err(_) => return,
                        };
                        let mut reader = std::io::BufReader::new(reader_stream);
                        let mut writer = std_stream;
                        let _ = trnsprt::serve_rw(&mut reader, &mut writer, &server);
                    });
                }
            }
        }
    });

    // ── TUI loop (blocking) ───────────────────────────────────────────────
    let keymap = KeyMap::from_config(&cfg.mux);
    {
        let mut reg = registry.lock().expect("registry lock");
        if let Err(e) = run_tui(&mut reg, &keymap) {
            eprintln!("kern mux: TUI error: {e}");
        }
    }

    // Signal the MCP task to stop.
    let _ = cancel_tx.send(());
}
