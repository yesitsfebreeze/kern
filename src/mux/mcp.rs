//! MCP server for the mux PTY multiplexer.
//!
//! Exposes four tools (mux_spawn, mux_send, mux_list, mux_status) that
//! agents running inside panes can call to manage sibling panes.
//! Only active in mux mode — never registered when running `--daemon`.
//!
//! Transport: TCP loopback, one thread per accepted connection (see run_mux).

use std::sync::{Arc, Mutex};

use serde::Deserialize;
use trnsprt::{McpError, McpServer, ToolResult, ToolSchema};

use crate::mux::registry::PaneRegistry;

pub struct MuxMcpServer {
    pub registry: Arc<Mutex<PaneRegistry>>,
    pub agent_cmd: String,
}

// ── Tool schemas ──────────────────────────────────────────────────────────────

pub fn tool_schemas() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "mux_spawn",
            "description": "Spawn a new agent sub-pane in the mux TUI.",
            "inputSchema": {
                "type": "object",
                "required": ["label"],
                "properties": {
                    "label": { "type": "string", "description": "Human label for the new pane (e.g. 'worker-1')" },
                    "cmd":   { "type": "string", "description": "Command to run (defaults to configured agent_cmd)" },
                },
            },
        }),
        serde_json::json!({
            "name": "mux_send",
            "description": "Write text to a pane's PTY stdin.",
            "inputSchema": {
                "type": "object",
                "required": ["session_id", "text"],
                "properties": {
                    "session_id": { "type": "string", "description": "Session id from mux_spawn or mux_list" },
                    "text":       { "type": "string", "description": "Text to write to the pane's PTY stdin" },
                },
            },
        }),
        serde_json::json!({
            "name": "mux_list",
            "description": "List all active panes.",
            "inputSchema": {
                "type": "object",
                "properties": {},
            },
        }),
        serde_json::json!({
            "name": "mux_status",
            "description": "Get the current visible screen content of a pane.",
            "inputSchema": {
                "type": "object",
                "required": ["session_id"],
                "properties": {
                    "session_id": { "type": "string", "description": "Session id of the pane to inspect" },
                },
            },
        }),
    ]
}

// ── McpServer impl ────────────────────────────────────────────────────────────

impl McpServer for MuxMcpServer {
    fn server_name(&self)    -> &str { "kern-mux" }
    fn server_version(&self) -> &str { env!("CARGO_PKG_VERSION") }

    fn tools_list(&self) -> Vec<ToolSchema> {
        tool_schemas()
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect()
    }

    fn call_tool(&self, name: &str, args: &serde_json::Value) -> Result<ToolResult, McpError> {
        let result = match name {
            "mux_spawn"  => self.tool_spawn(args),
            "mux_send"   => self.tool_send(args),
            "mux_list"   => self.tool_list(),
            "mux_status" => self.tool_status(args),
            _            => tool_error(&format!("unknown mux tool: {name}")),
        };
        Ok(value_to_tool_result(result))
    }
}

// ── Tool handlers ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SpawnArgs { label: String, cmd: Option<String> }

#[derive(Deserialize)]
struct SendArgs { session_id: String, text: String }

#[derive(Deserialize)]
struct IdArgs { session_id: String }

impl MuxMcpServer {
    fn tool_spawn(&self, args: &serde_json::Value) -> serde_json::Value {
        let p: SpawnArgs = match serde_json::from_value(args.clone()) {
            Ok(v) => v,
            Err(e) => return tool_error(&format!("invalid args: {e}")),
        };
        let cmd = p.cmd.unwrap_or_else(|| self.agent_cmd.clone());
        let mut reg = match self.registry.lock() {
            Ok(g) => g,
            Err(_) => return tool_error("registry lock poisoned"),
        };
        let cols = reg.cols;
        let rows = reg.rows;
        match reg.spawn_pane(p.label, cmd, cols, rows) {
            Ok(id)  => tool_ok(serde_json::json!({ "session_id": id })),
            Err(e)  => tool_error(&format!("spawn failed: {e}")),
        }
    }

    fn tool_send(&self, args: &serde_json::Value) -> serde_json::Value {
        let p: SendArgs = match serde_json::from_value(args.clone()) {
            Ok(v) => v,
            Err(e) => return tool_error(&format!("invalid args: {e}")),
        };
        let mut reg = match self.registry.lock() {
            Ok(g) => g,
            Err(_) => return tool_error("registry lock poisoned"),
        };
        if reg.send_to(&p.session_id, &p.text) {
            tool_ok(serde_json::json!({}))
        } else {
            tool_error(&format!("no pane with id: {}", p.session_id))
        }
    }

    fn tool_list(&self) -> serde_json::Value {
        let reg = match self.registry.lock() {
            Ok(g) => g,
            Err(_) => return tool_error("registry lock poisoned"),
        };
        let panes: Vec<serde_json::Value> = reg.panes.iter().map(|p| {
            serde_json::json!({
                "session_id": p.id,
                "label":      p.label,
                "exited":     p.exited,
            })
        }).collect();
        tool_ok(serde_json::json!(panes))
    }

    fn tool_status(&self, args: &serde_json::Value) -> serde_json::Value {
        let p: IdArgs = match serde_json::from_value(args.clone()) {
            Ok(v) => v,
            Err(e) => return tool_error(&format!("invalid args: {e}")),
        };
        let reg = match self.registry.lock() {
            Ok(g) => g,
            Err(_) => return tool_error("registry lock poisoned"),
        };
        let Some(pane) = reg.find(&p.session_id) else {
            return tool_error(&format!("no pane with id: {}", p.session_id));
        };
        let screen = pane.parser.screen();
        let (rows, cols) = screen.size();
        let (cursor_row, cursor_col) = screen.cursor_position();
        let screen_text = pane.screen_text();
        tool_ok(serde_json::json!({
            "session_id":  pane.id,
            "label":       pane.label,
            "exited":      pane.exited,
            "cols":        cols,
            "rows":        rows,
            "cursor_row":  cursor_row,
            "cursor_col":  cursor_col,
            "screen_text": screen_text,
        }))
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn tool_ok(v: serde_json::Value) -> serde_json::Value {
    let s = serde_json::to_string(&v).unwrap_or_default();
    serde_json::json!({ "content": [{ "type": "text", "text": s }] })
}

fn tool_error(msg: &str) -> serde_json::Value {
    serde_json::json!({
        "isError": true,
        "content": [{ "type": "text", "text": msg }],
    })
}

fn value_to_tool_result(v: serde_json::Value) -> ToolResult {
    let is_error = v.get("isError").and_then(|v| v.as_bool()).unwrap_or(false);
    let content  = v.get("content").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    ToolResult { content, is_error, structured_content: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_definitions_are_well_formed() {
        let defs  = tool_schemas();
        let names: Vec<&str> = defs.iter()
            .map(|d| d["name"].as_str().expect("name"))
            .collect();
        assert_eq!(names, ["mux_spawn", "mux_send", "mux_list", "mux_status"]);
        for d in &defs {
            let name = d["name"].as_str().unwrap();
            assert!(d["inputSchema"].is_object(), "{name}: needs inputSchema");
            assert_eq!(d["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn mux_spawn_schema_requires_label() {
        let defs  = tool_schemas();
        let spawn = defs.iter().find(|d| d["name"] == "mux_spawn").unwrap();
        let req   = spawn["inputSchema"]["required"].as_array().unwrap();
        let strs: Vec<&str> = req.iter().filter_map(|v| v.as_str()).collect();
        assert!(strs.contains(&"label"));
    }

    #[test]
    fn mux_send_schema_requires_session_id_and_text() {
        let defs = tool_schemas();
        let send = defs.iter().find(|d| d["name"] == "mux_send").unwrap();
        let req  = send["inputSchema"]["required"].as_array().unwrap();
        let strs: Vec<&str> = req.iter().filter_map(|v| v.as_str()).collect();
        assert!(strs.contains(&"session_id"));
        assert!(strs.contains(&"text"));
    }

    #[test]
    fn mux_status_schema_requires_session_id() {
        let defs   = tool_schemas();
        let status = defs.iter().find(|d| d["name"] == "mux_status").unwrap();
        let req    = status["inputSchema"]["required"].as_array().unwrap();
        let strs: Vec<&str> = req.iter().filter_map(|v| v.as_str()).collect();
        assert!(strs.contains(&"session_id"));
    }
}
