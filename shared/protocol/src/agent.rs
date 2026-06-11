use serde::{Deserialize, Serialize};
use trnsprt::kern_rpc::dto::Anchor;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageSummary {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_micro_usd: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TurnResult {
    Ok {
        reply: String,
        usage: Option<UsageSummary>,
    },
    Err(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SlashOutcome {
    Ok(String),
    Unknown(String),
    Err(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AgentLifecycle {
    Idle,
    Thinking,
    Running,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum OutputEvent {
    /// Full response text delivered in one shot (not streamed). The drill view
    /// renders this as a complete reply; live turns emit `Chunk` deltas instead.
    Token(String),
    /// Per-chunk streaming delta emitted during a turn.
    Chunk(String),
    Lifecycle(AgentLifecycle),
    Done,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginSummary {
    pub name: String,
    pub summary: String,
}

/// Role a fork plays in the agent hierarchy.
///
/// `Chat` is the user-facing dispatcher (cheap/fast model). It routes
/// simple queries from knowledge and delegates complex work to `Orchestrator`.
/// `Orchestrator` is the reasoning/planning fork spawned by `Chat`.
/// `FileHandle` serialises edits to a single file path; one per active file.
/// `Worker` executes a bounded task cheaply and exits when done.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", content = "c")]
#[derive(Default)]
pub enum ForkKind {
    /// User-facing conversational fork. Fast, cheap, dispatches to Orchestrator.
    #[default]
    Chat,
    /// Reasoning/planning fork. Uses the orchestrator model from auth.
    Orchestrator,
    FileHandle { path: String },
    Worker,
}


/// Wire-level fork lifecycle. Mirrors the client-internal `ForkState`
/// enum used by the agent strip; the client converts at the boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForkStateLite {
    Idle,
    Thinking,
    Running,
    Done,
    Error,
}

/// Snapshot of one fork as exposed by `AgntRpc::list_forks`. Carries
/// just enough for a client to paint a tile + drill summary.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForkSnapshot {
    pub fork_id: String,
    /// Role in the orchestrator → file-handle → worker hierarchy.
    #[serde(default)]
    pub kind: ForkKind,
    /// Parent fork id for FileHandle and Worker forks; `None` for Orchestrator.
    #[serde(default)]
    pub parent_fork_id: Option<String>,
    pub state: ForkStateLite,
    /// Wall-clock millis since UNIX_EPOCH of the last log entry on
    /// the fork. `0` means "never".
    pub last_msg_ts_ms: u64,
    /// First user message truncated to 60 chars; falls back to last
    /// assistant message when the fork has not received user input
    /// yet. Empty when the log is empty.
    pub summary: String,
}

trnsprt::service! {
	pub trait AgntRpc {
    async fn fork_open() -> String;
    async fn fork_resume(fork_id: String) -> ();
    async fn run_turn(fork_id: String, body: String) -> TurnResult;
    async fn run_slash(fork_id: String, body: String) -> SlashOutcome;
    async fn poll_output(fork_id: String) -> Vec<OutputEvent>;
    async fn list_plugins() -> Vec<PluginSummary>;
    /// Snapshot every live fork on the server. Used by a client to
    /// paint the bottom-row agent strip. Best-effort projection of each
    /// `Session`'s log into a `ForkSnapshot`.
    async fn list_forks() -> Vec<ForkSnapshot>;
    /// Best-effort cancel of any in-flight or queued turn for `fork_id`.
    /// Sets a cancel flag and drops queued output events. The currently-
    /// running turn (if any) is checked at coarse boundaries and aborts as
    /// soon as it next polls the flag.
    async fn cancel_turn(fork_id: String) -> ();
    /// Replace a prior message at `target_ts_ms` with `new_text`. Truncates
    /// session history after the edit point and re-ingests the rewritten
    /// message so subsequent turns see the corrected transcript.
    async fn edit_message(fork_id: String, target_ts_ms: u64, new_text: String)
        -> Result<(), String>;
    /// Branch the conversation at `from_ts_ms`: create a new session whose
    /// history mirrors `fork_id` up to and including that timestamp, and
    /// return the new fork id.
    async fn fork_at(fork_id: String, from_ts_ms: u64) -> Result<String, String>;
    /// Replicate `parent_fork_id` into a fresh fork seeded with caller
    /// context (file path, byte range, selection). The new fork's first
    /// turn opens with a synthesised "Replicating <parent>: anchor=..."
    /// message so the agent has the originating context. Returns the new
    /// fork id. Mirror of `KernRpc::fork_at(parent, anchor)` from slice J.
    async fn fork_at_anchor(parent_fork_id: String, anchor: Anchor)
        -> Result<String, String>;
    /// Open a child fork with an explicit role in the agent hierarchy.
    /// The new fork's `parent_fork_id` is set to `parent_fork_id` and its
    /// system prompt is tailored to `kind`. If `model_override` is `Some`,
    /// this fork uses that model instead of the default. Returns the new fork id.
    async fn fork_open_child(parent_fork_id: String, kind: ForkKind, model_override: Option<String>) -> String;
    /// Update the `ForkKind` of an existing fork without changing its history.
    /// Used to tag the startup fork as `Orchestrator`.
    async fn set_fork_kind(fork_id: String, kind: ForkKind) -> ();
}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fork_snapshot_serde_roundtrip() {
        let snap = ForkSnapshot {
            fork_id: "01HABC".into(),
            kind: ForkKind::Worker,
            parent_fork_id: Some("01HPARENT".into()),
            state: ForkStateLite::Running,
            last_msg_ts_ms: 1_700_000_000_000,
            summary: "first user msg".into(),
        };
        let json = serde_json::to_string(&snap).expect("encode");
        let back: ForkSnapshot = serde_json::from_str(&json).expect("decode");
        assert_eq!(back.fork_id, snap.fork_id);
        assert_eq!(back.state, snap.state);
        assert_eq!(back.last_msg_ts_ms, snap.last_msg_ts_ms);
        assert_eq!(back.summary, snap.summary);
    }

    #[test]
    fn fork_state_lite_variants_roundtrip() {
        for s in [
            ForkStateLite::Idle,
            ForkStateLite::Thinking,
            ForkStateLite::Running,
            ForkStateLite::Done,
            ForkStateLite::Error,
        ] {
            let j = serde_json::to_string(&s).unwrap();
            let back: ForkStateLite = serde_json::from_str(&j).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn fork_kind_variants_serde_roundtrip() {
        for k in [
            ForkKind::Chat,
            ForkKind::Orchestrator,
            ForkKind::FileHandle { path: "src/main.rs".into() },
            ForkKind::Worker,
        ] {
            let j = serde_json::to_string(&k).unwrap();
            let back: ForkKind = serde_json::from_str(&j).unwrap();
            assert_eq!(k, back, "round-trip for {k:?}");
        }
    }

    #[test]
    fn fork_kind_uses_adjacent_tag_content_tagging() {
        // #[serde(tag = "t", content = "c")]: the content variant FileHandle carries
        // its inner `path` under "c", unit variants carry only the tag.
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&ForkKind::FileHandle { path: "p.rs".into() }).unwrap())
                .unwrap();
        assert_eq!(v["t"], "FileHandle");
        assert_eq!(v["c"]["path"], "p.rs");

        let vc: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&ForkKind::Chat).unwrap()).unwrap();
        assert_eq!(vc["t"], "Chat");
        assert!(vc.get("c").is_none(), "a unit variant has no content payload");
    }

    #[test]
    fn fork_kind_default_is_chat() {
        assert_eq!(ForkKind::default(), ForkKind::Chat);
    }

    #[test]
    fn usage_summary_and_turn_result_roundtrip() {
        let usage = UsageSummary { input_tokens: 12, output_tokens: 34, cost_micro_usd: 56 };
        let back: UsageSummary =
            serde_json::from_str(&serde_json::to_string(&usage).unwrap()).unwrap();
        assert_eq!(back.input_tokens, 12);
        assert_eq!(back.output_tokens, 34);
        assert_eq!(back.cost_micro_usd, 56);

        // TurnResult::Ok carries an optional usage; Err carries a message.
        let ok = TurnResult::Ok { reply: "hi".into(), usage: Some(usage) };
        let ok_back: TurnResult =
            serde_json::from_str(&serde_json::to_string(&ok).unwrap()).unwrap();
        assert!(matches!(ok_back, TurnResult::Ok { reply, usage: Some(u) } if reply == "hi" && u.output_tokens == 34));
        let err_back: TurnResult =
            serde_json::from_str(&serde_json::to_string(&TurnResult::Err("boom".into())).unwrap()).unwrap();
        assert!(matches!(err_back, TurnResult::Err(m) if m == "boom"));
    }
}
