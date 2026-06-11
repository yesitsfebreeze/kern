//! Data transfer objects for [`SearchSvc`](super::SearchSvc).
//!
//! These are mirror types — they intentionally do NOT depend on the
//! `kern` crate. The kern side translates its richer internal types to
//! and from these on the boundary. Keeping DTOs colocated with the
//! transport keeps the `repl` palette free of any kern dependency.
//!
//! All DTOs derive `serde::{Serialize, Deserialize}` so they can be
//! shuttled by either the line-delimited JSON envelope codec or the
//! length-delimited bincode codec — the codec choice is per-channel.

use serde::{Deserialize, Serialize};

// ---- entity kind / status -------------------------------------------------

/// Lightweight mirror of `kern::EntityKind`.
///
/// The canonical seven-variant enum from the PRD. Held here so the
/// `repl` palette never needs the `kern` crate as a build dependency.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum EntityKindLite {
    Fact,
    /// Default unverified statement — mirrors `kern::EntityKind`'s own default.
    #[default]
    Claim,
    Document,
    Question,
    Answer,
    Conclusion,
    Superseded,
}

impl EntityKindLite {
    /// Parse a wire-side lower-case kind label (e.g. `"fact"`) into the lite
    /// enum. The single source of truth for label→kind, shared by the mock and
    /// the kern RPC server so the mapping can't drift between them.
    ///
    /// Returns `None` for unknown labels AND for `"superseded"`: Superseded is a
    /// lifecycle *status* ([`EntityStatusLite`]), not a content kind — it mirrors
    /// `kern::EntityKind`, which has no `Superseded` variant. Callers treat `None`
    /// as "no kind filter", not "match nothing".
    pub fn from_label(s: &str) -> Option<Self> {
        match s {
            "fact" => Some(Self::Fact),
            "claim" => Some(Self::Claim),
            "document" => Some(Self::Document),
            "question" => Some(Self::Question),
            "answer" => Some(Self::Answer),
            "conclusion" => Some(Self::Conclusion),
            _ => None,
        }
    }
}

/// Lightweight mirror of `kern::EntityStatus` — orthogonal lifecycle flag.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum EntityStatusLite {
    #[default]
    Active,
    Superseded,
}

// ---- edges ----------------------------------------------------------------

/// Mirror of `kern::Reason` kinds. One variant per typed edge in the
/// connected index.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum EdgeKind {
    #[default]
    Answers,
    Supports,
    Contradicts,
    Extends,
    Requires,
    References,
    Derives,
    Instances,
    PartOf,
    Consolidates,
}

// ---- edge reference (relationship) ----------------------------------------

/// One enriched relationship edge attached to a search hit. Carries the
/// sentence that explains the specific logical connection so callers can
/// reason about WHY two entities are linked, not just THAT they are.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EdgeRef {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    /// LLM-generated sentence naming the exact mechanism, cause, or logical
    /// dependency linking `from` → `to`. Empty until kern tick enrichment;
    /// callers should skip unenriched edges.
    pub text: String,
    /// Cosine similarity between the two endpoint vectors.
    pub score: f32,
}

// ---- entity reference (search hit) ----------------------------------------

/// One result row delivered to the palette. Cheap to clone; carries
/// only what `Card` needs to render plus the id used to drill in.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EntityRef {
    pub id: String,
    pub kind: EntityKindLite,
    pub status: EntityStatusLite,
    /// URI scheme without the `://` (e.g. `file`, `ticket`, `session`,
    /// `agent`, `inline`). Lets the palette pick the source-glyph
    /// without parsing a full URI.
    pub scheme: String,
    pub label: String,
    /// Short snippet shown under the label; already truncated by the
    /// server.
    pub snippet: String,
    /// Fused score (HNSW + BM25 + PageRank + heat). Higher = better.
    pub score: f32,
    /// Enriched relationship edges incident to this entity. Only edges with
    /// a non-empty `text` sentence are included. Empty when no enriched
    /// edges exist or the response predates this field.
    #[serde(default)]
    pub edges: Vec<EdgeRef>,
}

// ---- search ---------------------------------------------------------------

/// One filter chip applied to a query. `scheme` and `kind` are
/// independently optional so a facet can constrain by either axis (or
/// both, when the user types e.g. `>file !fact`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Facet {
    pub scheme: Option<String>,
    pub kind: Option<EntityKindLite>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchReq {
    pub query: String,
    pub facets: Vec<Facet>,
    pub k: u32,
    /// Monotonic per-keystroke token. Newer tokens supersede older
    /// ones in the server's mock cancellation logic; production
    /// implementations may use it to early-return stale work.
    pub cancel_token: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchRes {
    pub hits: Vec<EntityRef>,
    /// True iff this response was for the most-recent `cancel_token`
    /// the server has seen. The client may discard stale frames.
    pub fresh: bool,
}

// ---- neighbors ------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NeighborsReq {
    pub entity_id: String,
    /// Empty = all edge kinds.
    pub edge_kinds: Vec<EdgeKind>,
    /// Server clamps to `[0, 3]`.
    pub depth: u8,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NeighborsRes {
    pub neighbors: Vec<EntityRef>,
}

// ---- preview --------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PreviewReq {
    pub entity_id: String,
}

/// Preview pane payload. The variant carries everything the renderer
/// needs — the palette decides which sub-renderer to dispatch to based
/// on the discriminant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PreviewRes {
    /// File-backed entity. `language` is a tree-sitter grammar id
    /// (`"rust"`, `"python"`, ...) or `None` for plain text.
    File {
        path: String,
        content: String,
        language: Option<String>,
    },
    /// Generic entity body — Fact, Claim, Conclusion, etc.
    Text { content: String },
    /// Reason edge between two entities; rendered as a sentence in the
    /// preview pane.
    Edge {
        from_label: String,
        to_label: String,
        kind: EdgeKind,
        sentence: String,
    },
}

// ---- bincode + serde_json roundtrip smoke ---------------------------------

#[cfg(test)]
mod dto_serde_tests {
    use super::*;

    #[test]
    fn entity_ref_roundtrips_through_serde_json() {
        let original = EntityRef {
            id: "e1".into(),
            kind: EntityKindLite::Fact,
            status: EntityStatusLite::Active,
            scheme: "file".into(),
            label: "main.rs".into(),
            snippet: "fn main() {}".into(),
            score: 0.92,
            edges: vec![EdgeRef {
                from: "e1".into(),
                to: "e2".into(),
                kind: EdgeKind::Supports,
                text: "e1 provides the indexing mechanism that e2 depends on".into(),
                score: 0.87,
            }],
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: EntityRef = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, original.id);
        assert_eq!(back.kind, original.kind);
        assert_eq!(back.scheme, original.scheme);
        assert!((back.score - original.score).abs() < f32::EPSILON);
        assert_eq!(back.edges.len(), 1);
        assert_eq!(back.edges[0].text, original.edges[0].text);
    }

    #[test]
    fn entity_ref_with_no_edges_roundtrips_json() {
        // Ensure #[serde(default)] lets old payloads without `edges` deserialise cleanly.
        let json = r#"{"id":"e0","kind":"Fact","status":"Active","scheme":"inline","label":"x","snippet":"y","score":0.5}"#;
        let back: EntityRef = serde_json::from_str(json).unwrap();
        assert!(back.edges.is_empty(), "missing edges field defaults to empty vec");
    }

    #[test]
    fn entity_ref_roundtrips_through_bincode() {
        let original = EntityRef {
            id: "e2".into(),
            kind: EntityKindLite::Question,
            status: EntityStatusLite::Superseded,
            scheme: "ticket".into(),
            label: "T-9".into(),
            snippet: "why?".into(),
            score: 0.1,
            edges: vec![],
        };
        let cfg = bincode::config::standard();
        let bytes = bincode::serde::encode_to_vec(&original, cfg).unwrap();
        let (back, _): (EntityRef, _) =
            bincode::serde::decode_from_slice(&bytes, cfg).unwrap();
        assert_eq!(back.id, original.id);
        assert_eq!(back.status, original.status);
    }

    #[test]
    fn preview_res_variants_roundtrip_json() {
        let cases = vec![
            PreviewRes::File {
                path: "a.rs".into(),
                content: "x".into(),
                language: Some("rust".into()),
            },
            PreviewRes::Text { content: "claim".into() },
            PreviewRes::Edge {
                from_label: "A".into(),
                to_label: "B".into(),
                kind: EdgeKind::Supports,
                sentence: "A supports B.".into(),
            },
        ];
        for c in cases {
            let s = serde_json::to_string(&c).unwrap();
            let back: PreviewRes = serde_json::from_str(&s).unwrap();
            // PreviewRes now derives PartialEq, so a round-trip is a single `==`
            // instead of per-variant field matching.
            assert_eq!(back, c, "PreviewRes survives a JSON round-trip");
        }
    }

    #[test]
    fn search_req_cancel_token_roundtrips_through_bincode() {
        // Explicitly cover the Option<u64> serde path: None and the boundary
        // values, since a codec bug there would silently break cancellation.
        let cfg = bincode::config::standard();
        for token in [None, Some(0u64), Some(42), Some(u64::MAX)] {
            let req = SearchReq { query: "q".into(), facets: vec![], k: 5, cancel_token: token };
            let bytes = bincode::serde::encode_to_vec(&req, cfg).unwrap();
            let (back, _): (SearchReq, _) = bincode::serde::decode_from_slice(&bytes, cfg).unwrap();
            assert_eq!(back.cancel_token, token, "Option<u64> cancel_token survives bincode");
        }
    }

    #[test]
    fn entity_ref_default_is_empty_with_sensible_kind_and_status() {
        let d = EntityRef::default();
        assert!(d.id.is_empty() && d.edges.is_empty());
        assert_eq!(d.kind, EntityKindLite::Claim, "default kind mirrors kern's Claim");
        assert_eq!(d.status, EntityStatusLite::Active, "default status is Active");
        // Two defaults compare equal now that EntityRef derives PartialEq.
        assert_eq!(EntityRef::default(), d);
    }
}
