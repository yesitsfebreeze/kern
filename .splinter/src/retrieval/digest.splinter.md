# src/retrieval/digest.rs — commentary

- `build_digest`: the curation design is card #49. Rationale: ranking by `heat * conf_mean` makes a hot but low-confidence claim sink below a warm, well-corroborated one. `min_trust` gating exists because the digest is replayed into every future session — it is the persistent re-injection surface for memory-poisoning, so low-trust / repeatedly-contradicted claims are quarantined off it. `token_budget` exists because of context rot (attention degrades with length); near-dup skipping keeps restatements from wasting the budget.
- `est_tokens`: the ~4 chars/token figure is what OpenAI/BGE-class tokenizers average for English.
- `build_connections`: the entity cache avoids an O(N×M) nested kerns scan per reason during scoring and formatting.
Second-pass migration:

- `build_digest` gate semantics (resolves the `(see note)` on its doc comment): `min_trust` and `token_budget` are both disabled by a value of 0 — they are opt-in curation gates. `k` is NOT one of them: it is applied unconditionally as a hard item cap, so a caller cannot get an unbounded digest by zeroing the other two. Order matters — the trust gate quarantines first, then ranking by `heat * conf_mean`, then near-dup collapse, then the token budget trims the tail; `k` bounds the result regardless of which gates are live.
- `build_digest` token budget: the first bullet is always admitted even when it alone exceeds the budget, so a digest is never empty purely because of a tight `token_budget`; later bullets are trimmed.
- Multibyte regression: digest text is sliced by byte budget, and slicing at a raw byte offset panicked when a multibyte char straddled the boundary (the pinned case is a 3-byte `→` spanning bytes 38..41 cut at 39). Any future budget-trim edit must stay on a char boundary.

# Ratings — scope: src/retrieval/digest.rs

Scope rating: 8/10 — builds the session-injected digest.md (anchors + hottest thoughts + connections). Token-budgeted, dedup-aware, low-trust quarantine. Two sort-by-score paths lacked id tiebreaks (non-deterministic on ties); fixed to use cmp_rank with entity/reason id.

## Function ratings

- `build_digest` — 8/10→9/10: anchors + ranked bullets (heat×conf), token-budgeted with dedup. Sort was non-deterministic on ties (no id tiebreak); fixed to cmp_rank with entity id. Digest is injected into every session, so determinism matters.
- `build_connections` — 8/10→9/10: semantic reason edges ranked by from-entity heat×conf, token-budgeted with dedup. Same sort fix with reason id.
- `dedup_key` — 9/10: normalizes for near-duplicate skipping.
- `est_tokens` — 9/10: cheap token estimate for budget enforcement.
- `write_digest` — 9/10: atomic-ish file write, parent-dir creation.
- `ranks_by_heat_times_confidence` — 9/10: covers the ranking invariant.
- `token_budget_trims_body_greedily` — 9/10: covers budget enforcement.
- `near_duplicate_claims_are_skipped` — 9/10: covers dedup.
- `low_trust_claim_quarantined_even_when_hottest` — 9/10: covers trust gating.
- `documents_are_excluded_claims_kept` — 9/10: covers kind filtering.
Recall digest: a markdown snapshot of purpose + hottest thoughts, written for clients to read into new sessions. Pure builder (build_digest) + thin writer (write_digest); no live query path.

- est_tokens: ~4 chars/token; the digest only needs an approximate budget.
- dedup_key: near-duplicate key (whitespace-collapsed, lowercased, first 80 chars) so restatements of the same fact collapse to one key and only the first (hottest) survives.
- build_digest: purpose header + active claims ranked by heat * conf_mean. min_trust and token_budget of 0 disable those gates; k stays a hard item cap regardless. Excludes Document and Question kinds.
- Connections get 1/3 of the remaining token budget so they don't crowd the bullets (default 500 when no cap).
- build_connections: the ## Connections section = top enriched SEMANTIC edges ranked by from-entity heat*conf. Structural kinds (Spawn/Supersedes) carry no prose and are excluded (is_semantic() gate). entity_cache is built once (id -> (display, heat*conf)) and shared by scoring and formatting.
- Connection entity display truncates on a CHAR boundary via char_indices().nth(39): REGRESSION guard — slicing at raw byte 39 panicked when a multibyte char (e.g. '→') straddled the boundary.
- write_digest is best-effort: creates parent dirs and logs a warning on write failure.
