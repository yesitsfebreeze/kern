//! Mux PTY-multiplexer TUI — default `kern` launch mode.
//!
//! `run_mux` starts the TUI and a background MCP server task that exposes
//! the four `mux_*` tools on a TCP loopback socket.

pub mod delegate;
mod kern_client;
pub mod mcp;
pub mod pty;
pub mod registry;
mod research;
pub mod tui;

pub use delegate::{boot_message, result_key, task_key};
pub use kern_client::KernClient;
pub use research::ResearchPanel;
pub use mcp::MuxMcpServer;
pub use pty::{new_session_id, PtySession};
pub use registry::{PaneRegistry, SharedRegistry};
pub use tui::run_tui;

use std::sync::{Arc, Mutex};

use crate::config::{Config, KeyMap};

/// Launch the mux TUI.
///
/// 1. Ensures the per-cwd kern daemon is running (spawns a detached one if not)
///    so every pane shares one warm daemon.
/// 2. Starts the MCP server on `cfg.mux.mcp_addr` in a background task.
/// 3. Runs the TUI render loop (blocks until quit).
/// 4. Cancels the MCP task on return.
pub async fn run_mux(cfg: &Config) {
    // Bring up the per-cwd kern daemon BEFORE spawning any pane, so each pane's
    // `kern mcp` bridge attaches to one warm shared daemon (proxy mode) instead
    // of cold-spawning its own and timing out. Detached + null stdio, so it
    // never touches the TUI. No-op (instant) when a daemon is already running.
    crate::commands::ensure_daemon().await;

    // Register kern MCP in this project's .claude/settings.json so
    // `mcp__kern__*` tools appear in Claude Code automatically.
    {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        crate::commands::ensure_mcp_registered(&cwd);
    }

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
    let mcp_addr       = cfg.mux.mcp_addr.clone();
    let agent_cmd      = cfg.mux.agent_cmd.clone();
    let kern_mcp_addr  = cfg.mux.kern_mcp_addr.clone();
    let reg_mcp        = registry.clone();
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
                    // Tokio sets the fd non-blocking; restore blocking mode so the OS thread's
                    // BufReader::read_line in serve_rw blocks correctly instead of getting WouldBlock.
                    if let Err(e) = std_stream.set_nonblocking(false) {
                        tracing::warn!(target: "kern.mux", error = %e, "set_nonblocking(false) failed; skipping connection");
                        continue;
                    }
                    let reg  = reg_mcp.clone();
                    let cmd  = agent_cmd.clone();
                    let kern = kern_mcp_addr.clone();
                    std::thread::spawn(move || {
                        let server = MuxMcpServer { registry: reg, agent_cmd: cmd, kern_mcp_addr: kern };
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
    // `run_tui` calls `event::poll`/`event::read` in a hot loop — blocking
    // syscalls that must not occupy a tokio async-worker thread. We hand off
    // to `spawn_blocking` so the tokio runtime remains responsive for the
    // MCP server tasks.
    let keymap  = KeyMap::from_config(&cfg.mux);
    let reg_tui = registry.clone();
    match tokio::task::spawn_blocking(move || run_tui(&reg_tui, &keymap)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => eprintln!("kern mux: TUI error: {e}"),
        Err(e)     => eprintln!("kern mux: TUI panicked: {e}"),
    }

    // Signal the MCP task to stop.
    let _ = cancel_tx.send(());
}
