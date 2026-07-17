# src/retrieval/digest.rs — commentary

- `build_digest`: the curation design is card #49. Rationale: ranking by `heat * conf_mean` makes a hot but low-confidence claim sink below a warm, well-corroborated one. `min_trust` gating exists because the digest is replayed into every future session — it is the persistent re-injection surface for memory-poisoning, so low-trust / repeatedly-contradicted claims are quarantined off it. `token_budget` exists because of context rot (attention degrades with length); near-dup skipping keeps restatements from wasting the budget.
- `est_tokens`: the ~4 chars/token figure is what OpenAI/BGE-class tokenizers average for English.
- `build_connections`: the entity cache avoids an O(N×M) nested kerns scan per reason during scoring and formatting.
Second-pass migration:

- `build_digest` gate semantics (resolves the `(see note)` on its doc comment): `min_trust` and `token_budget` are both disabled by a value of 0 — they are opt-in curation gates. `k` is NOT one of them: it is applied unconditionally as a hard item cap, so a caller cannot get an unbounded digest by zeroing the other two. Order matters — the trust gate quarantines first, then ranking by `heat * conf_mean`, then near-dup collapse, then the token budget trims the tail; `k` bounds the result regardless of which gates are live.
- `build_digest` token budget: the first bullet is always admitted even when it alone exceeds the budget, so a digest is never empty purely because of a tight `token_budget`; later bullets are trimmed.
- Multibyte regression: digest text is sliced by byte budget, and slicing at a raw byte offset panicked when a multibyte char straddled the boundary (the pinned case is a 3-byte `→` spanning bytes 38..41 cut at 39). Any future budget-trim edit must stay on a char boundary.
