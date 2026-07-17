# src/base/health.rs — commentary

Why shared: keeping the aggregation loop in one place stops the REPL and MCP
surfaces from drifting apart. Each caller layers its own extras on top — the
REPL adds queue depth, the MCP surface adds descriptor count.

Second-pass migration (comment -> note):
- The `//!` doc used to enumerate the roll-up's contents: kern/entity/reason
  counts, unnamed-kern count, and root anchor names. That is exactly the
  `HealthStats` field list, so the doc now names only the purpose and its two
  callers (REPL `health`, MCP `health` tool/resource).
- `graph_health_stats` is read-only: it walks every loaded kern once, and anchor
  names resolve through `accept::root_anchor_ids` filtered to loaded kerns — hence
  the invariant that `anchors.len() <= kerns`, which the empty-graph test pins.
