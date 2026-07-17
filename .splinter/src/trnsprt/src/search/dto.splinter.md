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
