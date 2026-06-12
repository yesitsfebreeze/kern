//! Mux PTY-multiplexer TUI — default `kern` launch mode.
//!
//! `run_mux` starts the TUI and a background MCP server task that exposes
//! the four `mux_*` tools on a TCP loopback socket.

pub mod delegate;
pub mod pty;
mod questions;
pub mod registry;
mod research;
pub mod tui;

pub use delegate::{boot_message, result_key, task_key};
pub use questions::{QuestionsAction, QuestionsOverlay};
pub use research::ResearchPanel;
pub use pty::{new_session_id, PtySession};
pub use registry::{PaneRegistry, SharedRegistry};
pub use tui::run_tui;

use std::sync::{Arc, Mutex};

use crate::config::{Config, KeyMap};

/// Launch the mux TUI as the cwd's singleton kern daemon.
///
/// Binds `kern.sock` and serves the engine **in-process** (via
/// [`crate::commands::bootstrap`]), so every spawned pane's `kern mcp` bridge
/// attaches to THIS process in proxy mode and the comms tools dispatch against
/// the live pane registry. If a daemon already owns the cwd, the TUI runs
/// attached to it (no second engine — single-writer lock). Blocks on the TUI
/// render loop until quit, then persists the graph.
pub async fn run_mux(cli: &crate::commands::Cli, cfg: &Config) {
    // Register kern MCP in this project's .mcp.json so `mcp__kern__*` tools
    // appear in Claude Code automatically and panes attach to this process.
    {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        crate::commands::ensure_mcp_registered(&cwd);
    }

    // Determine terminal size (fall back to 80×24 if detection fails).
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let layout       = crate::mux::tui::pane_layout(cols, rows);

    let registry = match PaneRegistry::new(
        cfg.mux.agent_cmd.clone(),
        layout.main_cols,
        layout.sub_cols,
        layout.pane_rows,
    ) {
        Ok(r)  => Arc::new(Mutex::new(r)),
        Err(e) => {
            eprintln!("kern mux: failed to spawn main pane: {e}");
            return;
        }
    };

    // ── Become the cwd singleton: own kern.sock + serve the engine in-process ──
    let endpoint = trnsprt::typed::Endpoint::kern();
    match trnsprt::typed::bind_kern_listener(&endpoint).await {
        Ok(trnsprt::typed::BindOutcome::Bound(listener)) => {
            // We own the cwd. Build the engine with the pane registry threaded in
            // so the comms tools (delegate/collect/panes/…) dispatch against it,
            // then serve kern.sock so every pane's `kern mcp` bridge attaches here.
            let h = crate::commands::bootstrap(cli, cfg, Some(registry.clone())).await;
            let mem = Arc::new(std::sync::Mutex::new(crate::memory_service::MemoryService::new()));
            let handler = crate::rpc::KernRpcHandler::new(h.server.clone(), mem);
            tokio::spawn(crate::rpc::serve_kern_rpc_loop(listener, handler));
            tracing::info!(target: "kern.mux", "mux owns kern.sock; engine in-process");

            run_tui_blocking(registry.clone(), KeyMap::from_config(&cfg.mux)).await;

            // Persist on exit, same as the headless daemon's shutdown path.
            let g = crate::base::locks::read_recovered(&h.graph);
            crate::commands::save_graph(&g);
        }
        Ok(trnsprt::typed::BindOutcome::AlreadyRunning) => {
            // A headless daemon already owns this cwd; do NOT open a second engine
            // (single-writer lock). The TUI runs attached: panes + research chat
            // reach the existing daemon over kern.sock. Comms tools that need this
            // process's registry are unavailable in this degraded mode.
            tracing::info!(target: "kern.mux", "daemon already owns kern.sock; TUI attaches to it");
            run_tui_blocking(registry.clone(), KeyMap::from_config(&cfg.mux)).await;
        }
        Err(e) => {
            eprintln!("kern mux: kern.sock bind failed: {e}");
        }
    }
}

/// Run the blocking TUI render loop on a dedicated blocking thread so its
/// `event::poll`/`event::read` syscalls never park a tokio async worker (the
/// in-process engine + kern_rpc accept loop need those workers responsive).
async fn run_tui_blocking(registry: SharedRegistry, keymap: KeyMap) {
    match tokio::task::spawn_blocking(move || run_tui(&registry, &keymap)).await {
        Ok(Ok(()))  => {}
        Ok(Err(e))  => eprintln!("kern mux: TUI error: {e}"),
        Err(e)      => eprintln!("kern mux: TUI panicked: {e}"),
    }
}
