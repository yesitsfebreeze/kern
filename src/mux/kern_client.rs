//! Thin synchronous MCP client for the kern daemon.
//!
//! Wraps `trnsprt::Client` with a `TcpTransport` so we reuse the same
//! JSON-RPC framing and `initialize` handshake already used elsewhere.
//! One TCP connection per call — connections are infrequent (one per
//! `mux_delegate` / `mux_collect`) so pooling is not worth the complexity.

use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use anyhow::Context as _;
use trnsprt::{Client, Transport};

/// Number of candidate results to request from kern in a `query` call.
const QUERY_K: u32 = 3;

// ── TCP transport ─────────────────────────────────────────────────────────────

/// Implements `trnsprt::Transport` over a pair of cloned `TcpStream`s.
///
/// `TcpStream` is full-duplex; we clone it so we can hold independent
/// reader and writer ends without lifetime tangles.
struct TcpTransport {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl TcpTransport {
    fn connect(addr: &str) -> anyhow::Result<Self> {
        let stream = TcpStream::connect(addr)
            .with_context(|| format!("kern MCP TCP connect to {addr}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let writer = stream
            .try_clone()
            .context("kern MCP TCP clone for writer")?;
        let reader = BufReader::new(stream);
        Ok(Self { reader, writer })
    }
}

impl Transport for TcpTransport {
    fn reader(&mut self) -> &mut dyn Read {
        &mut self.reader
    }
    fn writer(&mut self) -> &mut dyn Write {
        &mut self.writer
    }
    /// TCP sockets close naturally on drop; no explicit kill needed.
    fn kill(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ── KernClient ────────────────────────────────────────────────────────────────

/// Synchronous MCP client for the kern daemon.
///
/// Each call opens a fresh TCP connection, performs the MCP handshake, calls
/// one tool, and drops the connection. Designed to be called from a
/// `std::thread` (the mux MCP server handler thread) — no tokio context needed.
pub struct KernClient {
    /// TCP address of the kern daemon MCP server, e.g. `"127.0.0.1:7778"`.
    addr: String,
}

impl KernClient {
    pub fn new(addr: impl Into<String>) -> Self {
        Self { addr: addr.into() }
    }

    /// Ingest `text` into kern under `key`.
    ///
    /// Prepends `[KEY={key}]` so the document is reliably retrievable via
    /// `query(text=key)` through kern's lexical index. Uses `sync=true` so
    /// the document is indexed and searchable before this call returns.
    pub fn ingest(&self, key: &str, text: &str) -> anyhow::Result<()> {
        let full_text = format!("[KEY={key}]\n{text}");
        let mut client = self.open()?;
        let result = client
            .call_tool(
                "ingest",
                &serde_json::json!({
                    "text":      full_text,
                    "source":    "agent",
                    "object_id": key,
                    "sync":      true,
                }),
            )
            .with_context(|| format!("kern ingest key={key}"))?;
        if result.is_error {
            let msg = Self::extract_text(&result.content).unwrap_or_default();
            anyhow::bail!("kern ingest key={key}: tool error: {msg}");
        }
        Ok(())
    }

    /// Query kern for documents matching `query_text`.
    ///
    /// Returns the content of the first text block in the result, or an empty
    /// string if kern returned no results. Callers should use the unique key
    /// string (e.g. `"mux:task:abc12345"`) as `query_text` to surface the
    /// document ingested by [`ingest`].
    pub fn query(&self, query_text: &str) -> anyhow::Result<String> {
        let mut client = self.open()?;
        let result = client
            .call_tool(
                "query",
                &serde_json::json!({ "text": query_text, "k": QUERY_K }),
            )
            .with_context(|| format!("kern query text={query_text:?}"))?;
        if result.is_error {
            let msg = Self::extract_text(&result.content).unwrap_or_default();
            anyhow::bail!("kern query text={query_text:?}: tool error: {msg}");
        }
        Ok(Self::extract_text(&result.content).unwrap_or_default())
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    fn open(&self) -> anyhow::Result<Client> {
        let transport = TcpTransport::connect(&self.addr)?;
        let mut client = Client::new(Box::new(transport));
        client
            .initialize("kern-mux", env!("CARGO_PKG_VERSION"))
            .context("kern MCP initialize")?;
        Ok(client)
    }

    /// Extract the first `"text"` content block from `content`.
    /// Public for unit testing without network I/O.
    pub fn extract_text(content: &[serde_json::Value]) -> Option<String> {
        content
            .iter()
            .find(|item| item.get("type").and_then(|t| t.as_str()) == Some("text"))
            .and_then(|item| item.get("text"))
            .and_then(|t| t.as_str())
            .map(str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kern_client_constructs() {
        // Verify that KernClient::new succeeds without panicking.
        let _c = KernClient::new("127.0.0.1:7778");
    }

    #[test]
    fn extract_text_gets_first_text_block() {
        let content = vec![serde_json::json!({"type": "text", "text": "hello world"})];
        assert_eq!(
            KernClient::extract_text(&content).as_deref(),
            Some("hello world")
        );
    }

    #[test]
    fn extract_text_skips_non_text_blocks() {
        let content = vec![
            serde_json::json!({"type": "image", "data": "..."}),
            serde_json::json!({"type": "text", "text": "found me"}),
        ];
        assert_eq!(
            KernClient::extract_text(&content).as_deref(),
            Some("found me")
        );
    }

    #[test]
    fn extract_text_returns_none_on_empty_content() {
        let content: Vec<serde_json::Value> = vec![];
        assert!(KernClient::extract_text(&content).is_none());
    }

    #[test]
    fn extract_text_returns_none_when_no_text_type() {
        let content = vec![serde_json::json!({"type": "image", "data": "..."})];
        assert!(KernClient::extract_text(&content).is_none());
    }
}
