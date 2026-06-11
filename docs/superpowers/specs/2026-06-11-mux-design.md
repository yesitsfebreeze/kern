# Mux — PTY Multiplexer Mode for kern

**Date:** 2026-06-11  
**Status:** Approved  
**Scope:** `src/mux/`, `src/config/mux.rs`, `src/commands.rs`, `src/main.rs`, `src/lib.rs`, `Cargo.toml`

---

## Overview

`kern` with no flags launches a fullscreen PTY-multiplexer TUI (`mux` mode) instead of the daemon.
`kern --daemon` (or `kern daemon`) continues to run the knowledge substrate as-is.

The mux is a split-pane terminal UI that manages multiple independent agent processes — each
running in its own OS PTY. The default agent command is `claude` (Claude Code), configurable
via `kern.toml`. The left pane is always the main/orchestrator agent. The right half shows
whichever sub-pane is currently focused. A tab strip at the bottom lists all active panes.

An MCP server bound to `mux.sock` exposes four mux tools so agents in panes can spawn,
interrogate, and communicate with sibling panes programmatically.

---

## Entry Point Changes

### `src/commands.rs`

`Cli` gains a `daemon` subcommand (alias for `--daemon`). Both routes call `run_server()`.

```
kern              →  run_mux()   (new default — mux TUI)
kern --daemon     →  run_server()       (existing daemon path)
kern daemon       →  run_server()       (new subcommand alias)
kern query ...    →  dispatch()         (existing, unchanged)
```

### `src/main.rs`

```rust
match cli.command {
    Some(Commands::Daemon) => run_server(&cli, &cfg).await,
    Some(cmd)              => dispatch(cmd, &cfg).await,
    None if cli.daemon     => run_server(&cli, &cfg).await,
    None                   => kern::mux::run_mux(&cfg).await,
}
```

### `src/lib.rs`

Add `pub mod mux;`

---

## Module Layout

```
src/mux/
  mod.rs      — pub async fn run_mux(cfg); wires pty + registry + tui + mcp
  pty.rs      — PtySession: portable-pty + vt100, spawn/drain/write/resize
  registry.rs — PaneRegistry: Vec<PtySession>, focus index, Arc<Mutex<>>
  tui.rs      — crossterm render loop, event pump, draw_frame()
  mcp.rs      — MuxMcpServer: four mux tools
```

---

## Section 1: PTY & Data Model (`src/mux/pty.rs`)

### `PtySession`

```rust
pub struct PtySession {
    pub id:      String,                    // 8-char hex, generated via rand (workspace dep)
    pub label:   String,                    // "main", "sub-1", etc.
    pub cmd:     String,                    // command that was spawned
    master:      Box<dyn MasterPty + Send>,
    writer:      Box<dyn Write + Send>,
    pub parser:  vt100::Parser,             // in-memory screen grid
    rx:          mpsc::Receiver<Vec<u8>>,   // PTY output from reader thread
    child:       Box<dyn portable_pty::Child + Send>,
    pub exited:  bool,
}
```

**`PtySession::spawn(id, label, cmd, cols, rows) -> anyhow::Result<Self>`**  
Opens the PTY pair via `portable_pty::native_pty_system()`, spawns the child process,
launches a reader thread that drains output into an `mpsc::channel`. Mirrors
`relay/src/editor_process.rs::EditorProcess::spawn`.

**`drain(&mut self)`** — drains `rx` via `try_recv` loop, feeds bytes into `vt100::Parser::process()`. Called each frame.

**`write_input(&mut self, bytes: &[u8])`** — writes to PTY stdin. Used for keystrokes and `mux_send`.

**`resize(&mut self, cols: u16, rows: u16)`** — resizes PTY and rebuilds parser.

**`screen_text(&self) -> String`** — iterates `vt100::Screen` rows, collects cell contents as plain text (newline-separated, trailing whitespace trimmed). Used by `mux_status`.

---

## Section 2: Pane Registry (`src/mux/registry.rs`)

```rust
pub struct PaneRegistry {
    panes: Vec<PtySession>,
    pub focus: usize,   // index into panes; 0 = main
}
pub type SharedRegistry = Arc<Mutex<PaneRegistry>>;
```

**Methods:**

| Method | Behaviour |
|--------|-----------|
| `new(main_cmd, cols, rows)` | Spawns `panes[0]` (label `"main"`) immediately |
| `spawn_pane(label, cmd, cols, rows) -> String` | Appends new pane, returns `session_id`. Emits `journal::Kind::ForkOpen` |
| `find(&self, id) -> Option<&PtySession>` | Lookup by session_id |
| `find_mut(&mut self, id) -> Option<&mut PtySession>` | Mutable lookup |
| `focused_mut()` | `panes.get_mut(focus)` |
| `cycle_focus()` | `focus = (focus + 1) % panes.len()` |
| `drain_all()` | Calls `drain()` on every pane |
| `reap_exited()` | Removes exited panes, emits `ForkClose`, clamps focus |
| `resize_all(cols, rows)` | Calls `resize()` on every pane |
| `send_to(id, text)` | `find_mut` → `write_input(text.as_bytes())`. Emits `journal::Kind::Log` with key `"mux.send"` |

**Journal events:**

| Action | `journal::Kind` |
|--------|----------------|
| Pane spawned | `ForkOpen { fork_id: id, parent: None }` |
| Pane exited | `ForkClose { fork_id: id }` |
| `mux_send` called | `Log` with key `"mux.send"` |

---

## Section 3: TUI Render Loop (`src/mux/tui.rs`)

### Terminal setup / teardown

Enter raw mode, switch to alt-screen, hide cursor. A `Guard` struct restores terminal on
`Drop`. A panic hook calls the same restore path so crashes don't leave the terminal broken.

### Frame loop (~60 Hz)

```
loop {
    registry.drain_all()
    poll crossterm events (timeout 16ms)
    route keys via KeyMap
    registry.reap_exited()
    draw_frame(&registry, &mut stdout, cols, rows)
}
```

### Layout

```
┌─────────────────────┬──────────────────────┐
│  Main pane (PTY)    │  Focused sub-pane    │  rows - 1
├─────────────────────┴──────────────────────┤
│  [●main] [sub-1] [sub-2]       kern  12:34 │  1 row
└────────────────────────────────────────────┘
```

- Left pane: always `panes[0]`. Width = `cols / 2`.
- Right pane: `panes[focus]` when `focus > 0`; blank when only main exists. Width = `cols - cols/2`.
- Rendered cell-by-cell from `vt100::Screen` using crossterm `MoveTo` + colour attributes.
- Tab strip: one bottom row. Active pane prefixed with `●`. Right side shows `kern  HH:MM`.
- Resize: on `Event::Resize(w, h)` call `registry.resize_all(w, h-1)` and redraw.

### Key routing

All non-intercepted keystrokes are forwarded as raw bytes to the focused pane's PTY stdin.
Intercepted keys are defined by `KeyMap` (loaded from `cfg.mux`):

| Default binding | Action |
|-----------------|--------|
| `Tab` | Cycle focus through panes |
| `Alt+N` | Spawn new sub-pane |
| `Ctrl+W` | Close focused sub-pane (no-op on main) |
| `Ctrl+Q` | Kill all panes and exit |

---

## Section 4: Configuration (`src/config/mux.rs`)

```toml
[mux]
agent_cmd      = "claude"
key_new_pane   = "alt+n"
key_close_pane = "ctrl+w"
key_cycle      = "tab"
key_quit       = "ctrl+q"
```

`Config` gains `pub mux: MuxConfig`. Missing `[mux]` section uses `MuxConfig::default()`.

`KeyMap` is built once at startup from `MuxConfig` and threaded into the TUI event pump.
Each field parses a `&str` like `"alt+n"` or `"ctrl+w"` into a `crossterm::event::KeyEvent`.

---

## Section 5: MCP Server (`src/mux/mcp.rs`)

### Transport

Binds `<cwd>/.kern/mux.sock` (Unix domain socket / Windows named pipe via `trnsprt`).
Launched as a background tokio task before the TUI loop. Cancelled via `CancellationToken`
on TUI exit. Socket file removed on shutdown.

Only active in mux mode. Never registered when running `--daemon`.

### `MuxMcpServer`

Implements `trnsprt::McpServer`. Holds `Arc<Mutex<PaneRegistry>>`.

### Tools (all prefixed `mux_`)

**`mux_spawn`**
- Args: `label: string`, `cmd?: string`
- Returns: `{ session_id: string }`
- Spawns a new sub-pane. Uses `cfg.mux.agent_cmd` when `cmd` is omitted.

**`mux_send`**
- Args: `session_id: string`, `text: string`
- Returns: `{}`
- Writes `text` to the target pane's PTY stdin. Emits journal `Log`.

**`mux_list`**
- Args: _(none)_
- Returns: `[{ session_id, label, exited }]`
- Lists all current panes.

**`mux_status`**
- Args: `session_id: string`
- Returns: `{ session_id, label, exited, cols, rows, cursor_row, cursor_col, screen_text }`
- `screen_text` is the full visible pane content as plain text (rows joined with `\n`, trailing
  whitespace trimmed). Gives the calling agent enough context to send a `/btw` and understand
  the current pane state without further prompting.

---

## Dependencies Added to `kern/Cargo.toml`

```toml
portable-pty = "0.8"
vt100 = "0.15"
```

No changes to workspace `Cargo.toml` — these are kern-specific.

---

## What Does Not Change

- `run_server()` and all daemon code — untouched.
- Existing MCP tools (`query`, `ingest`, `link`, etc.) — untouched.
- `shared/` crates — no new crates needed; journal and protocol are reused as-is.
- Version stays `1.0.0`.
