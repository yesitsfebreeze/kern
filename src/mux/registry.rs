//! Registry of active PTY panes.
//!
//! `PaneRegistry` owns the `Vec<PtySession>` and the focus index.
//! Wrap in `SharedRegistry` (`Arc<Mutex<PaneRegistry>>`) to share between
//! the TUI render loop and the MCP server task.

use std::sync::{Arc, Mutex};

use crate::mux::pty::{new_session_id, PtySession};

pub type SharedRegistry = Arc<Mutex<PaneRegistry>>;

pub struct PaneRegistry {
    pub panes: Vec<PtySession>,
    /// Index of the currently focused pane. 0 = main.
    pub focus: usize,
    /// Terminal dimensions at the time of last resize.
    pub cols: u16,
    pub rows: u16,
}

impl PaneRegistry {
    /// Create a new registry and immediately spawn the main pane.
    pub fn new(main_cmd: String, cols: u16, rows: u16) -> anyhow::Result<Self> {
        let id   = new_session_id();
        let pane = PtySession::spawn(id.clone(), "main".to_string(), main_cmd, cols, rows)?;
        // Emit ForkOpen so SessionMirror can index this pane when a kern daemon is running.
        // Payload carries fork_id redundantly because history.db's kind_from_tag reads from
        // the payload column; the inline enum field alone is lost after JSONL→SQLite rollover.
        journal::emit(journal::Entry::new(
            journal::Kind::ForkOpen { fork_id: id.clone(), parent: None },
            "mux",
            serde_json::json!({ "fork_id": id }),
        ));
        Ok(Self { panes: vec![pane], focus: 0, cols, rows })
    }

    /// Spawn a new sub-pane and return its session id.
    pub fn spawn_pane(&mut self, label: String, cmd: String, cols: u16, rows: u16) -> anyhow::Result<String> {
        let id   = new_session_id();
        let pane = PtySession::spawn(id.clone(), label, cmd, cols, rows)?;
        self.panes.push(pane);
        journal::emit(journal::Entry::new(
            journal::Kind::ForkOpen { fork_id: id.clone(), parent: None },
            "mux",
            serde_json::json!({ "fork_id": &id }),
        ));
        Ok(id)
    }

    /// Look up a pane by session id.
    pub fn find(&self, id: &str) -> Option<&PtySession> {
        self.panes.iter().find(|p| p.id == id)
    }

    /// Mutable look up a pane by session id.
    pub fn find_mut(&mut self, id: &str) -> Option<&mut PtySession> {
        self.panes.iter_mut().find(|p| p.id == id)
    }

    /// Return the currently focused pane.
    pub fn focused(&self) -> Option<&PtySession> {
        self.panes.get(self.focus)
    }

    /// Return a mutable reference to the currently focused pane.
    pub fn focused_mut(&mut self) -> Option<&mut PtySession> {
        self.panes.get_mut(self.focus)
    }

    /// Advance focus to the next pane, wrapping around.
    pub fn cycle_focus(&mut self) {
        if self.panes.len() > 1 {
            self.focus = (self.focus + 1) % self.panes.len();
        }
    }

    /// Drain PTY output into each pane's vt100 parser. Call each frame.
    pub fn drain_all(&mut self) {
        for pane in &mut self.panes {
            pane.drain();
        }
    }

    /// Remove panes whose child process has exited.
    /// Clamps `focus` to stay in bounds.
    pub fn reap_exited(&mut self) {
        let mut i = 0;
        while i < self.panes.len() {
            if self.panes[i].poll_exited() {
                let id = self.panes[i].id.clone();
                let close_payload = serde_json::json!({ "fork_id": &id });
                journal::emit(journal::Entry::new(
                    journal::Kind::ForkClose { fork_id: id },
                    "mux",
                    close_payload,
                ));
                self.panes.remove(i);
                // If a pane below the focus was removed, shift focus down to keep pointing at the same pane.
                if i < self.focus && self.focus > 0 {
                    self.focus -= 1;
                }
                // Clamp if the focused pane itself was removed and it was the last one.
                if self.focus >= self.panes.len() && !self.panes.is_empty() {
                    self.focus = self.panes.len() - 1;
                }
            } else {
                i += 1;
            }
        }
        if self.panes.is_empty() { self.focus = 0; }
    }

    /// Resize every pane to `(cols, rows)`.
    pub fn resize_all(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        for pane in &mut self.panes {
            pane.resize(cols, rows);
        }
    }

    /// Write `text` bytes to the pane identified by `session_id`.
    /// Returns `false` if no pane with that id exists.
    pub fn send_to(&mut self, session_id: &str, text: &str) -> bool {
        let Some(pane) = self.find_mut(session_id) else { return false };
        pane.write_input(text.as_bytes());
        journal::emit(journal::Entry::new(
            journal::Kind::Log,
            "mux.send",
            serde_json::json!({ "session_id": session_id, "len": text.len() }),
        ));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_registry() -> PaneRegistry {
        #[cfg(windows)]
        let cmd = "cmd";
        #[cfg(not(windows))]
        let cmd = "sh";
        PaneRegistry::new(cmd.to_string(), 80, 24).expect("spawn main pane")
    }

    #[test]
    fn new_creates_main_pane() {
        let reg = make_registry();
        assert_eq!(reg.panes.len(), 1);
        assert_eq!(reg.panes[0].label, "main");
        assert_eq!(reg.focus, 0);
    }

    #[test]
    fn spawn_pane_returns_id_and_appends() {
        let mut reg = make_registry();
        #[cfg(windows)]
        let cmd = "cmd";
        #[cfg(not(windows))]
        let cmd = "sh";
        let id = reg.spawn_pane("sub-1".to_string(), cmd.to_string(), 80, 24).expect("spawn");
        assert_eq!(id.len(), 8);
        assert_eq!(reg.panes.len(), 2);
        assert_eq!(reg.panes[1].label, "sub-1");
    }

    #[test]
    fn cycle_focus_wraps() {
        let mut reg = make_registry();
        #[cfg(windows)]
        let cmd = "cmd";
        #[cfg(not(windows))]
        let cmd = "sh";
        reg.spawn_pane("sub-1".to_string(), cmd.to_string(), 80, 24).unwrap();
        reg.spawn_pane("sub-2".to_string(), cmd.to_string(), 80, 24).unwrap();
        assert_eq!(reg.focus, 0);
        reg.cycle_focus(); assert_eq!(reg.focus, 1);
        reg.cycle_focus(); assert_eq!(reg.focus, 2);
        reg.cycle_focus(); assert_eq!(reg.focus, 0);
    }

    #[test]
    fn find_by_id() {
        let mut reg = make_registry();
        #[cfg(windows)]
        let cmd = "cmd";
        #[cfg(not(windows))]
        let cmd = "sh";
        let id = reg.spawn_pane("sub-1".to_string(), cmd.to_string(), 80, 24).unwrap();
        assert!(reg.find(&id).is_some());
        assert!(reg.find("nonexistent").is_none());
    }
}
