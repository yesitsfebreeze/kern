# splinter: src/mcp/resources.rs


# Ratings — scope: src/mcp/resources.rs

Scope rating: 8/10 — MCP resource surface (thoughts list, individual thought/reason rendering). `resource_thoughts` sort was non-deterministic on score ties; fixed.

## Function ratings

- `resource_thoughts` — 7/10→9/10: top-thoughts listing, sorts by score then truncates to TOP_THOUGHTS. Sort was `partial_cmp.unwrap_or(Equal)` (non-deterministic on score ties → which thoughts appear in the resource listing varies); fixed to `cmp_rank` with entity id extracted from the JSON value.
- `resource_thought` — 9/10: individual thought rendering with edges.
- `resource_reason` — 9/10: individual reason rendering with endpoints.
- `resource_thought_missing_returns_error_json` — 9/10: covers missing-entity error path.
