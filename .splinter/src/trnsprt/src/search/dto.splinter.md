# src/trnsprt/src/search/dto.rs — commentary

Keeping DTOs colocated with the transport keeps the `repl` palette free of any kern build dependency. All DTOs derive serde Serialize+Deserialize so either codec (line-delimited JSON envelope or length-delimited bincode) can shuttle them; codec choice is per-channel.

- `EntityKindLite`: the canonical seven-variant enum from the PRD; mirrored here so the palette never needs the `kern` crate.
- `search_req_cancel_token_roundtrips_through_bincode` (test): explicitly covers the `Option<u64>` serde path (None + boundary values) because a codec bug there would silently break cancellation.
- `entity_ref_with_no_edges_roundtrips_json` (test): wire back-compat — `#[serde(default)]` must let old payloads without `edges` deserialise cleanly.

Second-pass migration — detail moved out of doc comments:

- `EntityKindLite::from_label`: single source of truth for label→kind, shared by the mock and the kern RPC server so the mapping can't drift. Returns `None` for `"superseded"` because Superseded is a lifecycle *status* (`EntityStatusLite`), not a content kind — mirrors `kern::EntityKind`, which has no `Superseded` variant.
- `EdgeRef`: carries the sentence explaining the specific logical connection so callers can reason about WHY two entities are linked, not just THAT they are; `text` names the exact mechanism, cause, or logical dependency.
- `EntityRef`: cheap to clone. `scheme` lets the palette pick the source-glyph without parsing a full URI (full scheme set: file, ticket, session, agent, inline).
- `SearchReq.cancel_token`: "monotonic per-keystroke" — the mock's cancellation logic supersedes older tokens; production implementations may early-return stale work.
- `PreviewRes`: each variant carries everything its sub-renderer needs.
Design notes (moved from source comments during comment sweep). These DTOs are mirror types that intentionally do NOT depend on the kern crate; kern translates at the wire boundary.
- EntityKindLite mirrors kern::EntityKind; Claim is the default unverified statement (mirrors kern's own default). from_label (KEPT tightened in source): single source of truth for label->kind; None (unknown labels AND "superseded", a status not a kind) means "no kind filter", never "match nothing".
- EntityStatusLite mirrors kern::EntityStatus — an orthogonal lifecycle flag. EdgeKind mirrors kern::Reason kinds — one variant per typed edge.
- EdgeRef.text: LLM-generated sentence naming the from->to link mechanism; empty until kern tick enrichment — callers should skip unenriched edges. EdgeRef.score: cosine similarity between the two endpoint vectors.
- EntityRef is one result row delivered to the palette — only what Card needs to render plus the drill id. scheme = URI scheme without the :// (e.g. file, ticket, inline). snippet = short snippet under the label, already server-truncated. score = fused score (HNSW + BM25 + PageRank + heat), higher = better. edges: only edges with a non-empty text sentence are included; empty when none exist or the response predates this field.
- Facet: one filter chip; scheme and kind are independently optional — a facet constrains either axis or both (e.g. >file !fact).
- SearchReq.cancel_token: monotonic per-keystroke token; newer supersedes older; servers may use it to early-return stale work. SearchRes.fresh: true iff response was for the most-recent cancel_token the server has seen; the client may discard stale frames.
- NeighborsReq.edge_kinds (KEPT in source): empty = all edge kinds. NeighborsReq.depth: server clamps to [0,3].
- PreviewRes: preview pane payload; the palette dispatches a sub-renderer on the discriminant. File.language is a tree-sitter grammar id ("rust","python",...) or None for plain text. Text = generic entity body (Fact/Claim/Conclusion/etc). Edge = reason edge between two entities, rendered as a sentence.
