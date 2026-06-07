# Anchors — replacing single-purpose routing with a multi-anchor root

Date: 2026-06-07
Status: Design (approved sections, pending written-spec review)

## Problem

Today each kern node carries one `purpose_text` / `purpose_vec`. The root
purpose is a single overarching vector; routing (`accept.rs::route_entity`)
descends the tree and, at each node with a purpose, gates an incoming entity by
`acceptance_probability(cosine_distance(e, purpose_vec), inner, outer)`.

A single root vector forces everything through one semantic lens. We want the
root to hold **multiple anchors** — independent overarching vectors — plus a
**generic** catch-all that absorbs whatever matches no anchor.

## Model

`types.rs`:

- Remove `purpose_text` and `purpose_vec` from `Kern`.
- Add to the root kern:

  ```rust
  pub struct Anchor {
      pub name: String,
      pub text: String,
      pub vec: Vec<f64>,
      pub inner_radius: f64,
      pub outer_radius: f64,
  }

  pub anchors: Vec<Anchor>,   // named anchors only
  ```

- `generic` is **implicit** — not stored as an `Anchor`, has no vector. It is the
  fallback bucket. Represented as a normal child kern of the root named
  `generic`, created lazily on first non-matching entity.

`has_purpose()` / related helpers are removed; replaced by anchor-aware checks.

## Routing (`accept.rs`)

At the **root only**:

1. For each named anchor, compute
   `p = acceptance_probability(cosine_distance(e.vec, anchor.vec), anchor.inner_radius, anchor.outer_radius)`.
2. Best anchor with `p >= 0.5` wins; the entity descends into that anchor's
   subtree.
3. If no anchor clears `0.5`, the entity goes to the `generic` child.

Below the chosen anchor (or inside generic), descent is the **existing**
`route_entity` tree logic — anchors and generic are ordinary kern subtrees from
that point down. No second anchor-matching layer below the root.

`acceptance_probability` and `cosine_distance` are already shared in `base`; no
duplication. Radii move from the kern node onto `Anchor`.

## Generic bucket

`generic` is a **normal subtree**: it clusters and descends like any kern. It is
not a flat list. Locality search works inside it before promotion.

## Emergent anchors (`tick/cluster.rs`)

A dense, cohesive cluster inside the `generic` subtree is promoted to a named
anchor:

1. Score generic-subtree clusters with the existing `is_core_cluster`.
2. On a qualifying cluster: name it via the existing `purpose_prompt` path (the
   same path that today names unnamed kerns), set the cluster centroid as the
   new `Anchor.vec`, inherit default radii.
3. Reparent the cluster's members under the new anchor; remove them from
   generic.

This reuses the current cluster → name machinery; no new naming pipeline.

## API surface

Rename `purpose` → `anchor` across the stack:

- MCP tool (`mcp/tools_admin.rs`): `purpose` tool becomes `anchor` with
  subcommands `add(name, text)`, `list`, `remove(name)`.
- CLI (`commands/admin.rs`), REPL (`repl.rs`), wire (`wire.rs`).
- RPC: `shared/trnsprt/src/kern_rpc/{dto,svc,mock}.rs` — rename the purpose
  method/DTO to anchor equivalents.
- `retrieval/digest.rs`, `viewer.rs`, resources — update references.

`anchor add` embeds `text` → `vec` (same embed path purpose used).

## No-compat

Per `CLAUDE.md` (no compat, clean base):

- No migration shim. The persisted `purpose_vec` in existing `.kern` files is
  dropped on reload.
- The single root purpose set this session is discarded. Zero named anchors
  exist yet, so the cost is nil; the graph starts with only `generic`.
- Persistence format (`base/persist.rs`) is rewritten to serialize `anchors`
  instead of `purpose_text` / `purpose_vec`. Old fields are not read.

## Affected files

- `src/base/types.rs` — `Anchor`, `anchors` field, drop purpose fields/helpers.
- `src/base/accept.rs` — root anchor-matching, generic fallback.
- `src/base/persist.rs` — serialize anchors; drop purpose.
- `src/tick/cluster.rs` + `src/tick/tasks.rs` — emergent promotion.
- `src/mcp/tools_admin.rs`, `src/mcp/tools.rs`, `src/mcp/resources.rs` — anchor tool.
- `src/commands/admin.rs`, `src/commands.rs`, `src/repl.rs`, `src/wire.rs`.
- `src/rpc/kern_rpc_server.rs`, `shared/trnsprt/src/kern_rpc/{dto,svc,mock}.rs`.
- `src/retrieval/digest.rs`, `src/viewer.rs`.
- Docs: `docs/book/src/guides/memory-bank.md`, architecture guide.

## Testing

- Unit: root routing picks best anchor over `0.5`; below-`0.5` falls to generic.
- Unit: two anchors, entity nearer one routes there; tie → higher `p`.
- Unit: generic subtree clusters and is searchable.
- Promotion: seeded dense generic cluster promotes to a named anchor and
  reparents members.
- Persist round-trip: anchors survive save/reload; no purpose fields remain.

## Out of scope

- Per-node (non-root) anchor sets.
- Multi-home entities (one home per entity).
- Data-folder relocation of `.kern` files — tracked separately.
