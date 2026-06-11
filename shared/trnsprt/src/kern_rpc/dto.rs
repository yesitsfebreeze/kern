//! Data transfer objects for [`KernRpc`](super::KernRpc).
//!
//! Mirror types — they intentionally do NOT depend on the `kern` crate.
//! The kern side translates its richer internal types to and from these
//! at the wire boundary. Many primitive types (`EntityKindLite`,
//! `EntityStatusLite`, `EdgeKind`, `EntityRef`) are imported from the
//! sibling [`search`](crate::search) module so the two services share
//! the same wire vocabulary — a Card flowing out of `SearchSvc::search`
//! can be drilled into via `KernRpc::neighbors` without a translation
//! step.
//!
//! All DTOs derive `serde::{Serialize, Deserialize}` so they can be
//! shuttled by either the line-delimited JSON envelope codec or the
//! length-delimited bincode codec — the codec choice is per-channel.

use serde::{Deserialize, Serialize};

// Shared with `SearchSvc`: same wire types so a search hit can flow
// straight into a neighbors/preview call without a translation step.
pub use crate::search::dto::{
    EdgeKind, EntityKindLite, EntityRef, EntityStatusLite, NeighborsReq, NeighborsRes,
};

// ---- Source ---------------------------------------------------------------

/// Lightweight mirror of `kern::Source`. One variant per URI scheme.
///
/// Each variant carries the minimum a caller needs to reconstruct the
/// kern-side `Source` enum on the server. Optional fields collapse to
/// the empty string on the wire (matches the kern-side `Default` impl).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SourceLite {
    File {
        path: String,
        #[serde(default)]
        section: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        author: String,
        #[serde(default)]
        url: String,
    },
    Ticket {
        system: String,
        object_id: String,
        #[serde(default)]
        section: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        author: String,
        #[serde(default)]
        url: String,
    },
    Session {
        session_id: String,
        #[serde(default)]
        section: String,
        #[serde(default)]
        title: String,
    },
    Agent {
        agent: String,
        #[serde(default)]
        object_id: String,
        #[serde(default)]
        title: String,
    },
    Inline {
        #[serde(default)]
        hash: String,
        #[serde(default)]
        section: String,
    },
}

impl Default for SourceLite {
    fn default() -> Self {
        SourceLite::Inline {
            hash: String::new(),
            section: String::new(),
        }
    }
}

impl SourceLite {
    /// Stable URI scheme tag — matches `kern::Source::scheme`.
    pub fn scheme(&self) -> &'static str {
        match self {
            SourceLite::File { .. } => "file",
            SourceLite::Ticket { .. } => "ticket",
            SourceLite::Session { .. } => "session",
            SourceLite::Agent { .. } => "agent",
            SourceLite::Inline { .. } => "inline",
        }
    }
}

// ---- query ----------------------------------------------------------------

/// Retrieval mode tag forwarded to kern's existing query pipeline.
/// Identical wire string to the MCP `query.mode` argument
/// (`"hybrid"`, `"vector"`, `"lexical"`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QueryReq {
    pub text: String,
    /// Number of hits to return. Server clamps to a sane maximum.
    pub k: u32,
    /// Retrieval mode. Empty string defaults to `"hybrid"`.
    #[serde(default)]
    pub mode: String,
    /// If true, kern attempts an LLM-synthesised answer alongside hits.
    #[serde(default)]
    pub answer: bool,
    /// Optional kind filter (lower-case label, e.g. `"fact"`).
    #[serde(default)]
    pub kind: String,
    /// Optional source-scheme filter (e.g. `"file"`).
    #[serde(default)]
    pub source: String,
    /// Cancellation/freshness token, mirrors `SearchSvc::SearchReq`.
    #[serde(default)]
    pub cancel_token: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QueryRes {
    /// Ranked entity hits. Reuses [`EntityRef`] from `SearchSvc` so a
    /// palette and a kern_rpc call render the same Card shape.
    pub hits: Vec<EntityRef>,
    /// Optional LLM answer when `QueryReq.answer == true`. Empty when
    /// no LLM is configured server-side.
    #[serde(default)]
    pub answer: String,
    /// True iff this response was for the most-recent `cancel_token`
    /// the server has seen. Mirrors `SearchRes::fresh`.
    #[serde(default = "default_true")]
    pub fresh: bool,
}

/// `fresh` defaults to `true` (a missing field on the wire means "not stale").
/// This needs a named fn because `#[serde(default = "...")]` takes a function
/// *path*, not a literal — there is no `#[serde(default = true)]` form, and the
/// derived `Default` for `bool` is `false`, which would be the wrong default here.
fn default_true() -> bool {
    true
}

// ---- ingest ---------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IngestReq {
    pub text: String,
    pub source: SourceLite,
    pub kind: EntityKindLite,
    /// Optional descriptor classifier passed through to the ingest
    /// pipeline. Use `None` to skip descriptor routing.
    #[serde(default)]
    pub descriptor: Option<String>,
    /// Confidence in [0.0, 1.0]. Server clamps to its agent-source
    /// ceiling (Fact tier requires user-source).
    #[serde(default)]
    pub conf: f64,
    /// If true, block until ingest completes; otherwise queue and
    /// return immediately with a placeholder doc id.
    #[serde(default)]
    pub sync: bool,
}

impl Default for IngestReq {
    fn default() -> Self {
        Self {
            text: String::new(),
            source: SourceLite::default(),
            kind: EntityKindLite::Claim,
            descriptor: None,
            conf: 0.0,
            sync: false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IngestRes {
    /// Newly created entity id (or doc id when `sync=false` and the
    /// pipeline hasn't yet committed the entity).
    pub entity_id: String,
    /// One of `"queued" | "ingested" | "duplicate" | "rejected"` —
    /// matches kern's `ingest::outcome::Status::as_str`.
    pub status: String,
    /// Optional human-readable note from the pipeline (rejection
    /// reason, dedup pointer, etc.).
    #[serde(default)]
    pub message: String,
}

// ---- link -----------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkReq {
    pub from_id: String,
    pub to_id: String,
    /// Reason edge kind. The kern_rpc server maps this to its
    /// internal [`ReasonKind`](kern::base::types::ReasonKind) where a
    /// 1:1 match exists; otherwise the closest semantic match is used
    /// and the edge text carries the original kind-name as a hint.
    pub reason_kind: EdgeKind,
    /// Free-text explanation of the relationship.
    #[serde(default)]
    pub text: String,
}

impl Default for LinkReq {
    fn default() -> Self {
        Self {
            from_id: String::new(),
            to_id: String::new(),
            reason_kind: EdgeKind::References,
            text: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LinkRes {
    pub reason_id: String,
}

// ---- anchor ---------------------------------------------------------------

/// Caller-context snapshot carried into a replicated fork.
///
/// `entity_id`/`source_uri` identify the addressable anchor (file path,
/// Document id, etc.). `byte_range` is `[start, end)` over the underlying
/// source bytes. `selection` carries the user's literal highlighted text
/// when present (small enough to inline in opening context).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Anchor {
    pub entity_id: String,
    pub source_uri: String,
    pub byte_range: (u64, u64),
    pub selection: Option<String>,
}

// ---- truncate -------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TruncateAfterReq {
    pub ts_ms: u64,
}

/// Ack-only response: `truncate_after` returns no data, so this is intentionally
/// an empty struct — receiving it IS the acknowledgement that the truncation ran.
/// Kept as a named type (rather than `()`) so the typed-RPC return shape stays
/// uniform and the method can grow fields later without a wire break.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TruncateAfterRes {}

// ---- forget --------------------------------------------------------------

/// Hard-delete an entity by id. The id is matched by prefix server-side
/// (matches the existing kern `tool_forget` semantics).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ForgetReq {
    pub id: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ForgetRes {
    /// True iff an entity with that id (or prefix) was found and removed.
    pub removed: bool,
}

// ---- degrade -------------------------------------------------------------

/// Decay confidence on an entity by id (prefix-matched). Mirrors the
/// kern `tool_degrade` MCP path. The legacy `strength` field on
/// memory_rpc had no kern-side counterpart and is intentionally dropped.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DegradeReq {
    pub id: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DegradeRes {
    /// True iff the entity was found and its confidence decayed.
    pub applied: bool,
}

// ---- health --------------------------------------------------------------

/// No request payload. The trait method takes no arguments.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HealthRes {
    /// True when the daemon is up and the store is loaded.
    pub ok: bool,
    /// Currently-active store data_dir (canonical path string).
    #[serde(default)]
    pub data_dir: String,
    /// Total kerns loaded across all attached stores.
    #[serde(default)]
    pub kerns: u64,
    /// Total entities loaded across all attached stores.
    #[serde(default)]
    pub entities: u64,
}

// ---- anchor --------------------------------------------------------------

/// Manage anchors (named top-level buckets the root routes into).
/// `action` is "list" (default), "add", or "remove". `add` needs name+text;
/// `remove` needs name.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AnchorReq {
    pub action: String,
    pub name: String,
    pub text: String,
}

/// The anchor tool's JSON result, serialized as a string for transport.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AnchorRes {
    pub result: String,
}

// ---- descriptor ----------------------------------------------------------

/// One of `"add"` or `"rm"`. Matches the existing kern descriptor CLI.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DescriptorReq {
    pub action: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DescriptorRes {}

// ---- pulse ---------------------------------------------------------------

/// Fire a stigmergic pulse through the root kern. The legacy
/// `query_id` field on memory_rpc was a Phase-1 holdover that the
/// kern side ignored; dropped here.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PulseReq {
    /// Pulse strength. `1.0` is the conventional default.
    #[serde(default = "default_pulse_strength")]
    pub strength: f64,
}

fn default_pulse_strength() -> f64 {
    1.0
}

impl Default for PulseReq {
    fn default() -> Self {
        Self {
            strength: default_pulse_strength(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PulseRes {}

// ---- call_tool -----------------------------------------------------------

/// Generic MCP-style tool dispatch — escape hatch used by the
/// `kern mcp` proxy subprocess to relay arbitrary stdio MCP
/// `tools/call` requests to the singleton daemon over `kern.sock`
/// without enumerating every tool as a typed RPC method.
///
/// `args` is the raw `tools/call.params.arguments` object the proxy
/// receives on stdio. The server-side handler forwards it verbatim to
/// the daemon's existing `mcp::Server::call_tool` and returns the full
/// MCP `{ content, isError? }` envelope so the proxy can pipe it back
/// to stdout unchanged.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CallToolReq {
    pub name: String,
    #[serde(default)]
    pub args: serde_json::Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CallToolRes {
    /// MCP envelope as emitted by the daemon-side
    /// `mcp::Server::call_tool` — `{ "content": [...], "isError": bool }`.
    pub envelope: serde_json::Value,
}

// ---- bincode + serde_json roundtrip smoke ---------------------------------

#[cfg(test)]
mod dto_serde_tests {
    use super::*;

    #[test]
    fn source_lite_roundtrips_through_serde_json() {
        let cases = vec![
            SourceLite::File {
                path: "src/main.rs".into(),
                section: String::new(),
                title: String::new(),
                author: String::new(),
                url: String::new(),
            },
            SourceLite::Ticket {
                system: "github".into(),
                object_id: "T-9".into(),
                section: String::new(),
                title: String::new(),
                author: String::new(),
                url: String::new(),
            },
            SourceLite::Session {
                session_id: "s-1".into(),
                section: String::new(),
                title: String::new(),
            },
            SourceLite::Agent {
                agent: "audit".into(),
                object_id: "o".into(),
                title: String::new(),
            },
            SourceLite::Inline {
                hash: "h".into(),
                section: String::new(),
            },
        ];
        for c in cases {
            let s = serde_json::to_string(&c).unwrap();
            let _back: SourceLite = serde_json::from_str(&s).unwrap();
        }
    }

    #[test]
    fn ingest_req_roundtrips_through_bincode() {
        let original = IngestReq {
            text: "hello".into(),
            source: SourceLite::Inline {
                hash: "h".into(),
                section: String::new(),
            },
            kind: EntityKindLite::Claim,
            descriptor: Some("note".into()),
            conf: 0.5,
            sync: true,
        };
        let cfg = bincode::config::standard();
        let bytes = bincode::serde::encode_to_vec(&original, cfg).unwrap();
        let (back, _): (IngestReq, _) = bincode::serde::decode_from_slice(&bytes, cfg).unwrap();
        assert_eq!(back.text, original.text);
        assert_eq!(back.kind, original.kind);
        assert_eq!(back.descriptor.as_deref(), Some("note"));
    }

    #[test]
    fn link_req_roundtrips_through_serde_json() {
        let original = LinkReq {
            from_id: "a".into(),
            to_id: "b".into(),
            reason_kind: EdgeKind::Supports,
            text: "A supports B".into(),
        };
        let s = serde_json::to_string(&original).unwrap();
        let back: LinkReq = serde_json::from_str(&s).unwrap();
        assert_eq!(back.from_id, original.from_id);
        assert_eq!(back.reason_kind, original.reason_kind);
    }

    #[test]
    fn anchor_roundtrips_through_serde_json_and_bincode() {
        let original = Anchor {
            entity_id: "e-1".into(),
            source_uri: "file:///tmp/x.rs".into(),
            byte_range: (10, 42),
            selection: Some("hello".into()),
        };
        let s = serde_json::to_string(&original).unwrap();
        let back: Anchor = serde_json::from_str(&s).unwrap();
        assert_eq!(back, original);

        let cfg = bincode::config::standard();
        let bytes = bincode::serde::encode_to_vec(&original, cfg).unwrap();
        let (back, _): (Anchor, _) = bincode::serde::decode_from_slice(&bytes, cfg).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn query_req_default_fresh_is_true() {
        let s = "{\"hits\":[],\"answer\":\"\"}";
        let back: QueryRes = serde_json::from_str(s).unwrap();
        assert!(back.fresh, "missing `fresh` should default to true");
    }

    #[test]
    fn query_req_roundtrips_through_json_and_bincode_with_cancel_token() {
        let original = QueryReq {
            text: "borrow checker".into(),
            k: 7,
            mode: "hybrid".into(),
            answer: true,
            kind: "fact".into(),
            source: "file".into(),
            cancel_token: Some(99),
        };

        // JSON: the Some(n) cancel_token survives, as do the scalar fields.
        let s = serde_json::to_string(&original).unwrap();
        let back: QueryReq = serde_json::from_str(&s).unwrap();
        assert_eq!(back.cancel_token, Some(99));
        assert_eq!(back.k, 7);
        assert!(back.answer);
        assert_eq!(back.kind, "fact");

        // bincode: same payload over the length-delimited codec.
        let cfg = bincode::config::standard();
        let bytes = bincode::serde::encode_to_vec(&original, cfg).unwrap();
        let (back2, _): (QueryReq, _) = bincode::serde::decode_from_slice(&bytes, cfg).unwrap();
        assert_eq!(back2.cancel_token, Some(99));
        assert_eq!(back2.mode, "hybrid");
        assert_eq!(back2.source, "file");

        // None path: the absent-token case round-trips as None, not Some(0).
        let none_req = QueryReq { cancel_token: None, ..Default::default() };
        let s = serde_json::to_string(&none_req).unwrap();
        let back3: QueryReq = serde_json::from_str(&s).unwrap();
        assert_eq!(back3.cancel_token, None);
    }
}
