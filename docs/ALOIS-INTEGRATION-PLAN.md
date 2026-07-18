# Kern-side implementation plan for Alois integration

> **Scope.** Only the Kern-code-change workstreams from the Alois spec
> (`Kern upgrades needed for Alois`): P1 (ACL + principal), P2 (review lifecycle
>
> + in-Kern metering), P3 (source-trust weighting + retention). Alois-side
> workstreams (P0 tenant supervisor, gateway metering, redaction) are out of
> scope — they live in the Alois repo.
>
> **Method.** Every claim in the spec was audited against the codebase at
> v1.0.0 (commit `85c4fef`). File paths, line numbers, and "already built"
> assertions verified. The audit findings below are the evidence base; the
> change points are the work.

---

## Audit: spec claims vs actual source

| Spec claim | Verified | Evidence |
| --- | --- | --- |
| `Acl { scope, users, groups }` at `types.rs:95` | ✓ exact | `src/base/types.rs:95` |
| `Entity.acl` field at line 268 | ✓ exact | `src/base/types.rs:268` |
| `place.rs:56` always `Acl::default()` | ✓ exact | `src/ingest/place.rs:56` |
| `file_watcher.rs:136` always `Acl::default()` | ✓ exact | `src/ingest/file_watcher.rs:136` |
| Retrieval never reads `acl` | ✓ | `matches_filter` (`score.rs:107`) and `matches_keep` (`seed.rs`) — the two filter predicates — check source/kind/scheme/min_conf/since/before/valid_at/as_of, never `acl` |
| Ingest MCP schema has no `acl`/`principals`/`scope` | ✓ | `tools_mutate.rs:13-35` — properties are text/source/object_id/section/author/title/url/conf/descriptor/sync/kind |
| Query MCP has no `principals` param | ✓ | `tools_query.rs:13-31` — no identity param |
| `EntityStatus` only Active/Superseded | ✓ | `types.rs:44-49` — no review state |
| `EntityKind` Fact/Claim/Document/Question/Answer/Conclusion | ✓ | `types.rs:11-20` |
| `Source` enum File/Ticket/Session/Agent/Inline | ✓ | `types.rs:101-126` |
| `ReasonKind` incl. Provenance | ✓ | `types.rs:52-61` |
| `conf_alpha`/`conf_beta` on Entity | ✓ | `types.rs:256-257` |
| `valid_from`/`valid_to`/`invalidated_at` bi-temporal | ✓ | `types.rs:289-297` (serde(skip), bincode-stable) |
| `valid_until` TTL field exists | ✓ | `types.rs:281` |
| `llm.rs` has no metering | ✓ | `src/llm.rs` — no counters, no per-model accounting |
| `RetrievalConfig` has no source-trust field | ✓ | `config/retrieval.rs:28-71` |
| Facts GC-immune in `remove_entity` | ✓ | `reason.rs:139-143` — returns early if `is_fact() && !is_superseded()` |
| `tool_forget` rejects facts | ✓ | `tools_mutate.rs:341` — `if thought.is_fact() { return tool_error("cannot forget a fact") }` |
| Source trust constants `USER_SOURCE`/`AGENT_SOURCE` | ✓ | `constants.rs:118-119` |
| `search_all_filtered` takes a `keep` predicate | ✓ | `search.rs:75-88` — the pre-filtered ANN path |
| `matches_keep` builds the keep closure from `matches_filter` | ✓ | `seed.rs:97-103` — single shared filter, the two must never diverge |
| Query id-path bypasses all filters | ✓ | `tools_query.rs:167-174` — `find_entity` direct lookup, no ACL guard |

**Additional findings not in the spec:**

+ A third `Acl::default()` call site exists at `types.rs:598` (the `Entity`
  `Default` impl) — harmless but worth noting for migration: any code that
  constructs an `Entity` via `Default::default()` gets an empty ACL (public).
+ `clamp_confidence` (`tools_mutate.rs:83`) already clamps ingest confidence
  against `AGENT_SOURCE` — the wire boundary that prevents agent callers from
  minting Fact-trust entities. ACL population must respect the same boundary:
  an agent caller cannot escalate its own `source` string to `USER_SOURCE`
  trust (constant `AGENT_SOURCE`), and likewise must not be able to grant
  itself principals it doesn't hold. **ACL is caller-asserted; the daemon
  trusts the MCP transport boundary** (one Alois process = the trust root),
  same as confidence already works.
+ The `tool_query` **id-path** (`find_entity` direct lookup) returns an entity
  with no filter at all — an ACL guard is needed there too, or an unauthorized
  user can fetch any entity by id.

---

## P1 — ACL enforcement + request principal through MCP

**The gating change.** Without it, a tenant's team members can see each
other's private-scope reasoning. Everything else is incremental.

### P1.1 — Expose ACL on `ingest`

**Goal:** a caller can set `principals` + `scope` on ingest, populated into
`Entity.acl` instead of `Acl::default()`.

**Change points:**

1. **`src/mcp/tools_mutate.rs`** — `tool_schemas()` ingest schema (lines
   13-35): add two properties:
   + `principals`: `{"type": "array", "items": {"type": "string"}, "description": "principal ids allowed to see this entity (empty = public)"}`
   + `scope`: `{"type": "string", "description": "sub-tenant scope label (team/personal/shared)"}`

2. **`src/mcp/tools_mutate.rs`** — `IngestArgs` struct (lines 46-65): add

   ```
   principals: Vec<String>,
   scope: String,
   ```

3. **`src/mcp/tools_mutate.rs`** — `tool_ingest()` (line 83+): build an `Acl`
   from `p.principals` / `p.scope` and thread it into the worker call. The ACL
   is caller-asserted — the daemon trusts the MCP transport (Alois is the
   trust root, same as confidence clamping already assumes).

4. **`src/ingest/worker.rs`** — `Job` struct (line 15): add `acl: Acl` field.
   `Worker::run()` signature (line 81): add `acl: Acl` param, forward into
   `Job`.

5. **`src/ingest/place.rs`** — `place_entity` (line 56): replace
   `acl: Acl::default()` with the `acl` from the `Job`. This is the single
   construction site for claim entities.

6. **`src/ingest/file_watcher.rs`** — line 136: file-watcher-created `Document`
   entities. Decision needed: do file-watched documents get an ACL? **Recommend:
   yes** — the watcher is a per-tenant daemon, so its documents inherit the
   tenant's default ACL (configurable in `kern.toml`, default = public within
   the tenant). Add an `acl` field to the watcher config / job and thread it.

7. **`src/base/types.rs:598`** — `Entity::default()`: leave as
   `Acl::default()` (empty = public). This is the fallback for tests and
   non-ingest construction; it's correct because an empty ACL means public.

**Facts vs claims:** ACL lives on the `Entity`. Facts are GC-immune, **not**
ACL-immune. A Fact a user can't see still can't be returned to them. The
existing `remove_entity` fact guard (`reason.rs:139`) is about deletion, not
visibility — leave it; ACL is a read-time filter, orthogonal.

### P1.2 — Request principal through MCP `query`

**Goal:** the retrieval path knows who is asking.

**Change points:**

1. **`src/mcp/tools_query.rs`** — `tool_schemas()` query schema (lines 13-31):
   add:
   + `principals`: `{"type": "array", "items": {"type": "string"}, "description": "requesting user's principal set; entities whose acl shares no principal are hidden (empty = public only)"}`
   + `scope`: `{"type": "string", "description": "requesting scope (optional, sub-tenant)"}`

2. **`src/mcp/tools_query.rs`** — `QueryArgs` struct (line 92): add
   `principals: Vec<String>`, `scope: String`.

3. **`src/mcp/tools_query.rs`** — `build_query_options()`: set the new fields
   on `QueryOptions`.

4. **`src/retrieval/score.rs`** — `QueryOptions` struct (line 30): add

   ```
   principals: Vec<String>,
   scope: String,
   ```

   Update `is_active()` (line 49) to consider `!principals.is_empty()` as
   active (so the pre-filtered ANN path engages).

### P1.3 — Enforce at retrieval

**Goal:** entities whose `acl.users`/`acl.groups`/`acl.scope` share no
principal with the requester are dropped. Mirror Alois's `PUBLIC_PRINCIPAL`
sentinel: an entity with empty `acl` (all fields empty) is public — visible
to everyone.

**Change points:**

1. **`src/retrieval/score.rs`** — `matches_filter()` (line 107): add the ACL
   predicate at the **top** (cheapest short-circuit, highest selectivity):

   ```rust
   if !acl_visible(entity, opts) {
       return false;
   }
   ```

   New pure function `acl_visible(entity, opts) -> bool`:
   + Entity ACL empty (scope/users/groups all empty) → public → `true`.
   + Requester principals empty → can only see public → `false` (entity is
     restricted and requester claims nothing).
   + Else: `true` if requester principals intersect `entity.acl.users` ∪
     `entity.acl.groups` (groups treated as principal ids — Alois resolves
     group→principal before the call, or Kern treats group ids as principal
     ids; **recommend: Alois resolves, Kern sees flat principals** — simplest,
     one filter, no group-resolution logic in Kern).
   + Scope: if `opts.scope` is set and `entity.acl.scope` is set and they
     differ → `false`. Empty entity scope = visible in all scopes.

   This is the **pure filter function** the spec asks for — testable without
   the daemon.

2. **`src/retrieval/seed.rs`** — `matches_keep` (line 97): no change needed.
   It already delegates to `matches_filter`, so the ACL predicate is
   automatically shared by the pre-filtered ANN path. **This is the key
   invariant**: the comment at `score.rs:106` says "the two must never
   diverge" — they won't, because there's one predicate.

3. **`src/mcp/tools_query.rs`** — `tool_query()` **id-path** (line 167): the
   `find_entity` direct lookup bypasses `matches_filter`. Add an ACL guard:
   after finding the entity, check `acl_visible(&entity, &opts)`; if false,
   return `tool_error("thought not found: ...")` — **not** "forbidden", to
   avoid leaking existence (same as Alois's `acl.js` returns 404 not 403).

### P1.4 — Tests

**Done-when criteria (from spec):**

1. **Unit test** (`src/retrieval/score.rs` test module): `acl_visible` pure
   function — ingest with principals `[u1]`, query as `u2` → zero hits; query
   as `u1` → hit; empty ACL → visible to all; empty requester → public only.

2. **MCP-level test** (`src/test-utils/src/mcp_pipe.rs` or a new integration
   test): against a running daemon — ingest under principal `u1`, query under
   `u2` → no hit; query under `u1` → hit. Two concurrent `query` calls with
   different `principals` return different result sets for the same text.

3. **id-path test**: ingest under `u1`, direct lookup by id under `u2` →
   "not found" (not the entity, not a "forbidden" leak).

---

## P2 — Review / draft / trust lifecycle

**Goal:** three states — draft (just ingested, untrusted) → pending review →
curated (approved, durable). Today Kern has only `Active`/`Superseded`
(invalidation over time, not review state).

### P2.1 — `review_state` on Entity

**Decision: new field, not reusing an `EntityKind` slot.** The spec suggests
reusing an unused `EntityKind` slot, but every `EntityKind` variant has
semantic meaning already (Fact = curated-by-trust, not curated-by-review).
Conflating trust-tier with review-state couples two orthogonal axes. A
dedicated `ReviewState` enum is one field, three variants, zero ambiguity.

**Change points:**

1. **`src/base/types.rs`** — add:

   ```rust
   #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
   #[repr(u8)]
   pub enum ReviewState {
       #[default]
       PendingReview = 0,
       Curated = 1,
       // None represented how? — see decision below.
   }
   ```

   **Decision: two-variant enum + a separate "review exempt" flag, or
   three-variant (`PendingReview`/`Curated`/`None`).** Recommend three-variant:
   `None` = not subject to review (admin-promoted Facts, auto-trusted sources).
   Default for user-ingested Claims = `PendingReview`; default for
   admin-promoted Facts = `None`.

   Wait — `#[default]` can only be one variant. If default is `PendingReview`,
   every existing entity (migrated) becomes pending. If default is `None`,
   new ingests must explicitly set `PendingReview`. **Recommend: default
   `PendingReview`** — fail-safe: unknown = untrusted. Existing entities get
   `PendingReview` on migration, which is correct (they were never reviewed).
   Admins bulk-promote if needed. The migration is a serde default, not a
   data rewrite — backward compatible.

2. **`src/base/types.rs`** — `Entity` struct: add `#[serde(default)] pub
   review_state: ReviewState` (serde default = backward-compatible, old
   snapshots deserialize as `PendingReview`).

### P2.2 — Source-level review policy

**Goal:** per-source default review state on ingest (Confluence → Curated,
transcript → PendingReview).

**Change points:**

1. **`src/ingest/config.rs`** or **`src/config/ingest.rs`** — add a
   `review_policy` map: source-scheme → default `ReviewState`. Config in
   `kern.toml`:

   ```toml
   [ingest.review_policy]
   ticket = "curated"      # Confluence/Jira — trusted
   session = "pending"     # transcripts — review
   ```

2. **`src/ingest/place.rs`** — `place_entity`: set `review_state` from the
   policy based on `source.scheme()`, unless the caller (MCP ingest param)
   explicitly overrides.

3. **`src/mcp/tools_mutate.rs`** — ingest schema: add optional
   `review_state` param (`"pending"` / `"curated"` / `"none"`) for per-call
   override. Wire-boundary: an agent caller cannot set `Curated` directly
   (same as it cannot mint Facts) — **add to `validate_ingest_wire`**: if
   `review_state == Curated` and source is `AGENT_SOURCE`, reject (only
   admin/promotion path sets Curated).

### P2.3 — Query filter + promotion

**Change points:**

1. **`src/retrieval/score.rs`** — `QueryOptions`: add
   `exclude_pending: bool`, `review_state: Option<ReviewState>`.
   `matches_filter`: if `exclude_pending`, drop `PendingReview` entities; if
   `review_state` set, keep only matching.

2. **`src/mcp/tools_query.rs`** — query schema: add `exclude_pending` (bool)
   and `review_state` (enum) params. Wire into `build_query_options`.

3. **Promotion MCP tool** — new tool `promote` in `tools_mutate.rs`:
   `{"name": "promote", "inputSchema": {"required": ["id"], "properties":
   {"id": "...", "to": {"enum": ["curated", "none"]}}}}`. Flips
   `PendingReview` → `Curated` (or `None`). Optionally also flips `Claim` →
   `Fact` for durability — but **that's a separate axis** (trust-tier vs
   review-state); keep them independent. Promotion sets review state only;
   Fact promotion is a separate admin action (already gated by
   `validate_fact_source`).

### P2.4 — Tests

+ Unit: `matches_filter` with `exclude_pending` drops PendingReview, keeps
  Curated.
+ MCP: ingest as PendingReview, query with `exclude_pending: true` → not
  returned; `promote` → query again → returned.
+ "list everything pending review for tenant X" = query with
  `review_state: "pending"`.

### P2.5 — In-Kern token metering (assess, likely defer)

**Spec recommendation:** gateway metering first (Alois side), in-Kern only if
background distillation becomes a cost surprise.

**Assessment:** the spec is right. In-Kern metering is P3-at-earliest. The
gateway path (Alois proxies model endpoints, counts per tenant) needs zero
Kern change and works today. **Defer in-Kern metering unless a tenant's
daemon distillation causes a real cost overrun.** If needed later:

**Change points (deferred):**
+ `src/llm.rs` — record token usage per call (parse `usage` from API
  response) into an `Arc<Mutex<UsageCounters>>` keyed by model + job kind.
+ `src/mcp/tools_admin.rs` — new `usage` tool exposing the counters; add to
  `health` output.
+ `src/config/` — per-period ceiling; when hit, pause distillation (the
  outage-queue behavior at `src/ingest/queue.rs` already handles LLM
  outages — a budget pause is the same signal).

---

## P3 — Source-trust weighting + retention

### P3.1 — Source-trust weighting in retrieval

**Goal:** user-authored claims rank higher than auto-ingested claims of equal
heat. Today `apply_boosts` (`score.rs:77`) applies `fact_score_boost` for
Facts and QBST access/recency — no source-trust prior.

**Change points:**

1. **`src/config/retrieval.rs`** — `RetrievalConfig`: add

   ```rust
   pub source_trust_user: f64,      // weight for USER_SOURCE
   pub source_trust_agent: f64,     // weight for AGENT_SOURCE
   pub source_trust_auto: f64,     // weight for everything else
   ```

   Defaults: `1.0`, `0.8`, `0.6` (tunable in `kern.toml`).

2. **`src/retrieval/score.rs`** — `apply_boosts()` (line 77): multiply the
   fused score by the source-trust weight based on `entity.source.system()`
   → `USER_SOURCE` / `AGENT_SOURCE` / other. One line in the boost loop:

   ```rust
   let trust = source_trust_weight(cfg, &e.source);
   r.set_score(r.score() * confidence * trust + boost + fact_bonus);
   ```

   New helper `source_trust_weight(cfg, source) -> f64`.

3. **Test:** two entities of equal heat, one `Source::Agent` one
   `Source::File` with `author` set (user) — query returns user-authored
   ranked higher. Verify the weight is applied in the boost step, not the RRF
   step (RRF is rank-based, pre-boost; trust is a score multiplier,
   post-fusion).

### P3.2 — Retention / right-to-be-forgotten

**Goal:** deleting a source in Alois propagates to the reasoning graph.
`forget` exists but Facts are immune and there's no by-source cascade.

**Change points:**

1. **`src/mcp/tools_mutate.rs`** — new tool `forget_by_source`:

   ```json
   {
     "name": "forget_by_source",
     "inputSchema": {
       "required": ["scheme", "object_id"],
       "properties": {
         "scheme": {"type": "string", "enum": ["file","ticket","session","agent","inline"]},
         "object_id": {"type": "string", "description": "source object id to forget"}
       }
     }
   }
   ```

2. **`src/base/reason.rs`** — new `forget_by_source(g, scheme, object_id)`:
   scan all kerns/entities, collect ids where `entity.source.scheme() ==
   scheme && entity.source.object_id() == object_id`, call `remove_entity`
   on each. **Override fact immunity for legal deletion**: add a `force:
   bool` param to `remove_entity` (or a separate `remove_entity_forced` that
   skips the `is_fact()` guard). A legal deletion overrides GC-immunity.

3. **`src/mcp/tools_mutate.rs`** — `tool_forget_by_source()`: acquire write
   lock, call `forget_by_source`, count removed, save. Return
   `{"removed": N}`.

4. **Per-source TTL (optional):** `valid_until` already exists and is
   bi-temporal. Add an ingest-time `retention` param (duration string) that
   sets `valid_until = now + retention`. The existing TTL decay path
   (`matches_filter` checks `valid_until < valid_at` at `score.rs:134`)
   handles natural expiry. **This is one param + one timestamp set** — small.

### P3.3 — Tests

+ `forget_by_source`: ingest N entities from source X, M from source Y;
  `forget_by_source("ticket", X)` → N removed, M remain, subsequent query
  for X content returns nothing.
+ Fact override: ingest a Fact from source X, `forget_by_source` with
  `force: true` → Fact removed (legal deletion overrides GC-immunity).
+ TTL: ingest with `retention: "1s"`, query immediately → visible; query after
  2s with `valid_at: now` → not visible.

---

## Implementation order

```
P1.1  Expose ACL on ingest (schema + Job + place.rs)
P1.2  Request principal on query (schema + QueryOptions)
P1.3  Enforce at retrieval (matches_filter + id-path guard)
P1.4  P1 tests  ← gates "can Alois use Kern as reasoning store?"
─────────────────────────────────────────────────────
P2.1  ReviewState enum + Entity field
P2.2  Source-level review policy (config + place.rs)
P2.3  Query filter + promote tool
P2.4  P2 tests
P2.5  In-Kern metering  ← deferred unless cost surprise
─────────────────────────────────────────────────────
P3.1  Source-trust weighting (RetrievalConfig + apply_boosts)
P3.2  forget_by_source + per-source TTL
P3.3  P3 tests
```

P1 is self-contained and gates everything. P2 builds on P1's `QueryOptions`
additions (review filters are just more `matches_filter` predicates). P3 is
independent of P2 and can be done in parallel after P1.

---

## Migration / backward-compatibility notes

+ **`Acl` on `Entity`**: already serde-serialized; old snapshots have it as
  `Acl::default()` (empty = public). No migration needed — existing entities
  are public, which is correct (pre-ACL everything was visible to the single
  agent).
+ **`ReviewState` on `Entity`**: `#[serde(default)]` → old snapshots
  deserialize as `PendingReview`. This is fail-safe (unknown = untrusted).
  Bulk-promote via the `promote` tool if an admin wants existing entities
  curated. No data rewrite.
+ **`QueryOptions` additions**: all new fields default to empty/false — old
  queries behave identically (no principals = public-only filter is off when
  the field is empty; **wait** — empty requester should see public-only, but
  existing behavior is see-everything. **Decision: when `principals` is
  empty, treat as "no ACL filter"** (backward-compat) rather than "public
  only". A caller that wants strict public-only passes an explicit sentinel.
  This preserves existing single-agent behavior where no one passes
  principals.) — this needs a clear semantic: **empty `principals` param =
  no filter (see all)**, matching today's behavior. Alois, when it wants
  enforcement, always passes the user's principal set.
+ **`RetrievalConfig` additions**: default to `1.0` for all trust weights →
  no change to existing ranking until a tenant configures otherwise.
+ **`forget_by_source`**: new tool, no migration. Existing `forget` unchanged.

---

## Open decisions (need confirmation before implementation)

1. **Group resolution**: Kern sees flat principals (Alois resolves groups →
   principal ids before the MCP call), or Kern resolves groups from
   `acl.groups`? **Recommend: flat** — simplest, one filter, no
   group-membership logic in Kern. Alois already does this in `acl.js`.

2. **File-watcher ACL**: do file-watched `Document` entities get a
   tenant-default ACL, or stay public? **Recommend: configurable in
   `kern.toml`, default public-within-tenant** (the tenant boundary is the
   process, per P0).

3. **`review_state` default on migration**: `PendingReview` (fail-safe,
   everything existing becomes untrusted until promoted) vs `None` (existing
   entities are review-exempt, only new ingests get the policy). **Recommend:
   `PendingReview`** — fail-safe, but flag this to Alois as a one-time
   bulk-promote operation.

4. **In-Kern metering**: confirm defer to P3-or-later, gateway-first. The spec
   already recommends this.

5. **`forget_by_source` fact override**: confirm that a legal deletion
   (right-to-be-forgotten) overrides Fact GC-immunity. The spec says yes
   ("including Facts, a legal deletion overrides GC-immunity"). This is the
   one place we punch through the fact guard — gate behind `force: true`
   explicit param, not default.
