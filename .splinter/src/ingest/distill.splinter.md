# src/ingest/distill.rs — commentary

- `DESCRIPTORS`: covers semantic + episodic knowledge plus `procedural` — Letta/MemGPT-style "how we do X" (learned workflows, rules, conventions), not just facts.

Second-pass migration (from source comments):

- `parse_claims` array unwrapping: only a LONE nested array (`items.len() == 1`) is unwrapped — an LLM quirk that wraps the result as `[[...]]`. Sibling arrays (`[...] [...]`) are invalid JSON across the first-`[`-to-last-`]` span and fail to empty; a len-2 array-of-arrays is neither unwrapped nor merged. Siblings must never be silently merged. Covered by `multiple_sibling_arrays_fail_gracefully_to_empty`.
- `procedural` descriptor: the Letta-style procedural scope must not fall back to `"fact"` (`procedural_kind_maps_through`).
- `parse_claims` uses `mem::take` on the unwrapped inner array to avoid cloning it.
- `Claim.valid_from`: optional bi-temporal world-time hint (ISO8601), stamped onto the ingested entity's `valid_from`. Parsed leniently — a garbage or absent hint yields `None` (falls back to ingestion time) and never blocks an otherwise-good claim (`valid_from_hint_is_parsed_when_present_and_ignored_when_garbage`).
