//! Registry of active PTY panes.
//!
//! `PaneRegistry` owns the `Vec<PtySession>` and the focus index.
//! Wrap in `SharedRegistry` (`Arc<Mutex<PaneRegistry>>`) to share between
//! the TUI render loop and the MCP server task.

use std::sync::{Arc, Mutex};

use crate::mux::pty::{new_session_id, PtySession};

pub type SharedRegistry = Arc<Mutex<PaneRegistry>>;

/// One in-flight human question raised by an agent via `raise_question`.
pub struct PendingQuestion {
    pub id:       String,
    pub label:    String,
    pub question: String,
    answer_tx:    std::sync::mpsc::Sender<String>,
}

/// In-memory roster of questions awaiting a human answer. Lives inside
/// [`PaneRegistry`]; the `raise_question` tool handler registers + blocks on
/// the receiver, the Ctrl+K overlay lists + answers. Never persisted.
#[derive(Default)]
pub struct QuestionRegistry {
    pending: Vec<PendingQuestion>,
}

impl QuestionRegistry {
    /// Register a question and return `(id, receiver)`. The caller blocks on
    /// the receiver AFTER releasing the registry lock.
    pub fn open(&mut self, label: String, question: String) -> (String, std::sync::mpsc::Receiver<String>) {
        let id = new_session_id();
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending.push(PendingQuestion { id: id.clone(), label, question, answer_tx: tx });
        (id, rx)
    }

    /// Roster view: `(id, label, question)` per pending entry, insertion order.
    pub fn list(&self) -> Vec<(String, String, String)> {
        self.pending.iter().map(|p| (p.id.clone(), p.label.clone(), p.question.clone())).collect()
    }

    /// Deliver `answer` to question `id`; remove it. False if `id` is unknown.
    pub fn answer(&mut self, id: &str, answer: String) -> bool {
        let Some(pos) = self.pending.iter().position(|p| p.id == id) else { return false };
        let p = self.pending.remove(pos);
        let _ = p.answer_tx.send(answer); // recv side may have hung up; ignore.
        true
    }

    /// Drop question `id` without answering (its sender drops → caller's recv errors).
    pub fn dismiss(&mut self, id: &str) -> bool {
        let Some(pos) = self.pending.iter().position(|p| p.id == id) else { return false };
        self.pending.remove(pos);
        true
    }

    pub fn len(&self) -> usize { self.pending.len() }
    pub fn is_empty(&self) -> bool { self.pending.is_empty() }
}

pub struct PaneRegistry {
    pub panes: Vec<PtySession>,
    /// Index of the currently focused pane. 0 = main.
    pub focus: usize,
    /// Dimensions for a newly spawned **sub-pane** (right-column width, pane
    /// height). Updated on every resize so panes created later via the MCP
    /// `spawn`/`delegate` tools match the rendered layout instead of being
    /// sized to the full terminal width.
    pub cols: u16,
    pub rows: u16,
    pub thoughts: u32,
    pub reasons:  u32,
    /// Questions raised by agents that are blocked awaiting a human answer.
    pub questions: QuestionRegistry,
}

impl PaneRegistry {
    /// Create a new registry and immediately spawn the main pane.
    ///
    /// `main_cols` sizes the main pane (left column); `sub_cols` is stored as the
    /// spawn width for future sub-panes (right column). Both come from
    /// [`pane_layout`](crate::mux::tui::pane_layout) so the PTY matches the
    /// region it is painted into.
    pub fn new(main_cmd: String, main_cols: u16, sub_cols: u16, rows: u16) -> anyhow::Result<Self> {
        let id   = new_session_id();
        let pane = PtySession::spawn(id.clone(), "main".to_string(), main_cmd, main_cols, rows)?;
        // Emit ForkOpen so SessionMirror can index this pane when a kern daemon is running.
        // Payload carries fork_id redundantly because history.db's kind_from_tag reads from
        // the payload column; the inline enum field alone is lost after JSONL→SQLite rollover.
        journal::emit(journal::Entry::new(
            journal::Kind::ForkOpen { fork_id: id.clone(), parent: None },
            "mux",
            serde_json::json!({ "fork_id": id }),
        ));
        Ok(Self { panes: vec![pane], focus: 0, cols: sub_cols, rows, thoughts: 0, reasons: 0, questions: QuestionRegistry::default() })
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

    /// Resize panes to match the two-column split: pane 0 (main) gets
    /// `main_cols`, every sub-pane gets `sub_cols`; all panes use `rows`. The
    /// renderer (`draw_frame`) derives the same widths from `pane_layout`, so
    /// each PTY's width matches the region it is painted into.
    ///
    /// `sub_cols`/`rows` are stored as the spawn dimensions for panes created
    /// later via the MCP `spawn`/`delegate` tools (always sub-panes).
    pub fn resize_split(&mut self, main_cols: u16, sub_cols: u16, rows: u16) {
        self.cols = sub_cols;
        self.rows = rows;
        for (i, pane) in self.panes.iter_mut().enumerate() {
            let w = if i == 0 { main_cols } else { sub_cols };
            pane.resize(w, rows);
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
        PaneRegistry::new(cmd.to_string(), 40, 39, 24).expect("spawn main pane")
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
    fn resize_split_sizes_main_and_subs_independently() {
        // Regression: panes must be sized to the column they are PAINTED into,
        // not the full terminal width. Main (index 0) = left half; subs = right
        // half minus the divider. The old `resize_all` gave every pane the full
        // width, so the right ~half of each agent's screen was clipped.
        let mut reg = make_registry();
        #[cfg(windows)]
        let cmd = "cmd";
        #[cfg(not(windows))]
        let cmd = "sh";
        reg.spawn_pane("sub-1".to_string(), cmd.to_string(), 10, 10).unwrap();

        reg.resize_split(40, 39, 23);

        assert_eq!(reg.panes[0].parser.screen().size(), (23, 40), "main pane = main_cols");
        assert_eq!(reg.panes[1].parser.screen().size(), (23, 39), "sub pane = sub_cols");
        assert_eq!(
            (reg.cols, reg.rows),
            (39, 23),
            "stored spawn dims track the sub-pane width so MCP spawn/delegate match the layout"
        );
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

    // ── QuestionRegistry ──────────────────────────────────────────────────

    #[test]
    fn question_open_then_answer_delivers_text() {
        let mut q = QuestionRegistry::default();
        let (id, rx) = q.open("worker-1".into(), "ship it?".into());
        assert_eq!(q.len(), 1);
        assert!(q.answer(&id, "yes".into()), "answer should find the id");
        assert_eq!(rx.recv().unwrap(), "yes", "blocked caller receives the answer");
        assert_eq!(q.len(), 0, "answered question is removed");
    }

    #[test]
    fn question_list_reports_label_and_text() {
        let mut q = QuestionRegistry::default();
        let _ = q.open("audit".into(), "delete shards?".into());
        let roster = q.list();
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].1, "audit");
        assert_eq!(roster[0].2, "delete shards?");
    }

    #[test]
    fn question_dismiss_drops_sender_so_recv_errors() {
        let mut q = QuestionRegistry::default();
        let (id, rx) = q.open(String::new(), "x?".into());
        assert!(q.dismiss(&id));
        assert!(rx.recv().is_err(), "dismiss drops the sender; recv errors");
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn question_answer_unknown_id_returns_false() {
        let mut q = QuestionRegistry::default();
        assert!(!q.answer("nope", "x".into()));
    }
}
