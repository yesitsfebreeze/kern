//! Task delegation helpers for `kern mux`.
//!
//! Defines the key naming convention for kern-stored tasks/results and the
//! boot message template sent to freshly spawned worker panes. No I/O here —
//! these are pure functions used by both the MCP handlers and by tests.

/// Return the kern `object_id` / query key for the task assigned to `session_id`.
///
/// Convention: `"mux:task:<session_id>"`. Unique enough that lexical retrieval
/// reliably surfaces the right document without cross-contamination.
pub fn task_key(session_id: &str) -> String {
    format!("mux:task:{session_id}")
}

/// Return the kern `object_id` / query key where a worker publishes its result.
pub fn result_key(session_id: &str) -> String {
    format!("mux:result:{session_id}")
}

/// Build the text stored in kern for `session_id`'s task.
///
/// Prepends `[KEY=<task_key>]` so the document is retrievable via
/// `query(text="mux:task:<session_id>")` through kern's lexical index.
pub fn kern_ingest_text(session_id: &str, task_description: &str) -> String {
    format!("[KEY={}]\n{}", task_key(session_id), task_description)
}

/// Spec for delegating a task to a new pane.
pub struct DelegateSpec {
    /// Human label for the new pane, e.g. `"worker-1"`.
    pub label: String,
    /// Full task description ingested into kern before spawning.
    pub task:  String,
    /// Command override; `None` falls back to `MuxConfig::agent_cmd`.
    pub cmd:   Option<String>,
}

/// Generate the compact boot message sent to a freshly spawned worker pane.
///
/// The message tells the agent exactly what to do in its first turn:
/// 1. Connect to kern MCP at `kern_mcp_addr` (already done automatically
///    when kern is in the MCP server list, but stated explicitly for clarity).
/// 2. Call `mcp__kern__query` with the task key to get the full task description.
/// 3. Ingest results back to kern under the result key when done.
///
/// Ends with `\n` so the PTY fires the Enter key immediately.
pub fn boot_message(session_id: &str, kern_mcp_addr: &str) -> String {
    let tkey   = task_key(session_id);
    let rkey   = result_key(session_id);
    format!(
        "[kern-mux bootstrap | session {session_id}]\n\
         Retrieve your task from kern before starting:\n\
           kern MCP addr : {kern_mcp_addr}\n\
           task key      : {tkey}\n\
           result key    : {rkey}\n\
         \n\
         Step 1 — call mcp__kern__query with text=\"{tkey}\" to read your task.\n\
         Step 2 — do the work described in the query result.\n\
         Step 3 — call mcp__kern__ingest with:\n\
                    text    = \"[KEY={rkey}]\\n<your summary>\"\n\
                    source  = \"agent\"\n\
                    object_id = \"{rkey}\"\n\
         \n",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_key_format() {
        assert_eq!(task_key("abc12345"), "mux:task:abc12345");
    }

    #[test]
    fn result_key_format() {
        assert_eq!(result_key("abc12345"), "mux:result:abc12345");
    }

    #[test]
    fn boot_message_contains_all_required_pieces() {
        let msg = boot_message("abc12345", "127.0.0.1:7778");
        assert!(msg.contains("abc12345"),            "must contain session_id: {msg:?}");
        assert!(msg.contains("127.0.0.1:7778"),      "must contain kern addr: {msg:?}");
        assert!(msg.contains("mux:task:abc12345"),   "must contain task key: {msg:?}");
        assert!(msg.contains("mux:result:abc12345"), "must contain result key: {msg:?}");
    }

    #[test]
    fn boot_message_ends_with_newline() {
        let msg = boot_message("abc12345", "127.0.0.1:7778");
        assert!(msg.ends_with('\n'), "boot message must end with newline: {msg:?}");
    }

    #[test]
    fn delegate_spec_can_be_constructed() {
        let spec = DelegateSpec { label: "worker-1".into(), task: "do X".into(), cmd: None };
        assert_eq!(spec.label, "worker-1");
        assert!(spec.cmd.is_none());
    }

    #[test]
    fn kern_ingest_text_embeds_key_for_lexical_retrieval() {
        let text = kern_ingest_text("abc12345", "do X");
        assert!(text.starts_with("[KEY=mux:task:abc12345]"), "key header missing: {text:?}");
        assert!(text.contains("do X"), "task body missing: {text:?}");
    }
}
