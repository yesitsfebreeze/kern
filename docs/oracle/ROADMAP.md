# Roadmap — the single source of truth

State and work, one file. `FEATURES.md` says what exists, `CHANGELOG.md` says
what was decided, `VISION.md` says what "built" means. This file is the only
place that says **what is left**. Nothing else in the repo plans work.

**Ordering is the content.** The topmost item is the most important open thing in
the repo; importance falls monotonically from there. Rank is assigned by severity
× reach, with sequencing constraints as hard edges — where item B cannot be done
before item A, A sits above B and says so. A tier heading explains why its band
sits where it does; the heading is commentary, the position is the plan.

**Position is rank; the number is only a name.** They agreed when the list was
founded 1..85 and they stop agreeing the moment anything closes or arrives. Items
cite each other by number ("blocked on item 13"), so renumbering to restore the
match would silently repoint those references — the cure is worse. A closed item
retires its number rather than compacting the list, and a new item takes the next
free number wherever it ranks.

Stamped 2026-07-21, re-verified against source rather than against documents.
Tier 0 is gone: "what measures retrieval quality with no LLM in the scoring loop"
is answered, and `e2e/` is the answer. That closure releases everything it gated
— items 32, 54, 55 and the whole of tier 8 are now judgeable the way item 86's two
candidate fixes were: apply, measure, keep only if `recall@1` holds.

A heading in this file was destroyed by an editing script that cut from an
item to the next one and swallowed the tier boundary between them; Tier 3 is
restored above its items. Say it here because a lost heading is invisible — the
items survive and simply appear to rank somewhere they do not. Tier 2 — "the
last cheap federation fix" — is a different absence and not a bug: item 16 was
its only item and closing it retired the whole band, exactly as Tier 0 went.
Tier numbers retire on close like item numbers do; a gap in the sequence is the
record, not a loss.

Context that is not work — north star, competitive position, non-goals, repo
laws, and what is closed — lives after the ranked list.

---

# Tier 1 — live defects on the default path that fail silently

These need no gossip, no flag and no unusual configuration. Every one produces a
wrong or missing result with no error, which is why they outrank both the
security work (armed only with federation on) and every feature.

### 9. Two live writers: `ingest`/`link`/`intake drain` still write locally `[surface]`

**Decided 2026-07-21, and the decision is implemented for the two commands it
fits.** The choice named here was between routing the one-shot writes through
the daemon's RPC and teaching them to detect a daemon and refuse. Routing won,
for the reason the item already gave: refusing makes the CLI useless whenever a
daemon runs, which is always. `src/commands/route.rs` is that route —
`route(name, args)` probes `Endpoint::kern()` once, never spawns, and returns
`Done` / `Refused` / `NoDaemon`, so an absent daemon is the ordinary case and a
daemon that answers owns the write outright (a tool error is reported, never
retried against the store behind its back). `kern forget` and `kern degrade`
take it (`cmd_forget`, `cmd_degrade` in `src/commands/graph_ops.rs`); the local
path is the `NoDaemon` fallback and prints through the same two printers, so
the two paths cannot drift in wording.

The destructive entrance was already shut and stays shut: `src/base/lock.rs` is
an advisory writer lock over the data dir (std `File::try_lock`, no dependency;
MSRV 1.82 -> 1.89), held for the daemon's lifetime, and `reembed`, `compact`,
`gc` refuse while it is held and name the holder. `kern status` reports daemon,
hub and lock.

**What is left, and why it is not just more of the same.** `ingest` and `link`
cannot ride this route as it stands, because the RPC's only mutation surface is
`call_tool` — the *agent* boundary. `tool_ingest` clamps against `AGENT_SOURCE`
"regardless of what `p.source` claims" and `tool_link` writes
`MAX_AI_CONFIDENCE`, while `cmd_ingest` mints at `clamp_confidence(1.0, "user")`
and `cmd_link` at `1.0`. Routing them unchanged would silently demote every
CLI-minted Fact to an agent Claim. Routing them *with* their trust intact means
putting a trust field on an unauthenticated socket, which is item 24's hole
widened into an escalation path. So this half is **blocked on item 24**, not on
effort. `intake drain` has no matching tool and would need one first.

That block is a new sequencing edge pointing *down* the file — item 24 sits in
tier 3 and this sits in tier 1 — and the list was not reordered for it. The
edge binds only the `ingest`/`link` half; item 9's other open half (`intake
drain`) needs no auth and keeps this position. Item 24 does
not move up because the trust field is one caller of it, not its severity: an
unauthenticated socket is armed the same either way.

**Closed 2026-07-21: the standalone fallback.** `kern mcp`'s standalone server
was the last long-lived second writer, and the route could not save it — it has
no daemon to hand the write to, and a *sibling* standalone binds no socket, so
no probe can see one. Only the lock can. `claim_standalone`
(`src/commands/mcp_cmd.rs`) now claims the dir as `mcp-standalone` **before**
the graph is read and holds it for the process, and a claim that fails does not
boot a second writer: it spends one more attach window on the endpoint (the
usual holder is the daemon this process just spawned, late to bind) and proxies
to it, or exits 1 naming the holder. The tradeoff, taken deliberately: a
`kern mcp` that loses this race now gives its client no kern where before it
gave one that silently overwrote the other's graph. Availability was never the
thing at risk.

**Closed 2026-07-21: the read side.** `kern get` and `kern query` route before
they touch disk (`cmd_get` in `src/commands/graph_ops.rs`, `cmd_query` in
`src/commands/query.rs`), both over the existing `query` tool — `{id}` for the
detail read, `{text, mode, k}` for the ranked one. A serving daemon's live graph
answers; the local load is the `NoDaemon` fallback, and both paths render
through one printer (`print_detail`, `print_results`) over the tool's own JSON,
so wording cannot drift. One id resolver serves both
(`mcp::tools_query::entity_detail_by_id`), which is what stops a routed and a
local `get` disagreeing about what a prefix means.

Two things that were not free. The `k` had to be sent: the tool defaults to
`seed_k`, well under the pool the local path delivers, so routing without it
made `kern query` return 25 hits with a daemon up and 36 without, on the same
corpus — the hit count silently depending on whether something was serving.
`retrieval::score::delivery_cap` is now the one owner of that number and both
sides read it. And routing `query` sends the text to the *daemon's* embedder,
not this process's: correct, since it owns the index the query hits, but it
means `--embed-model` on a routed `kern query` is ignored rather than honoured.
That is the same "the daemon owns it" rule the write half took, and it is
stated here rather than discovered.

**`search` and `list` stay local, by decision.** Not oversight and not
deferred work: `search` is the raw-ANN probe with no matching tool behind it,
and `list` prints the on-disk kern tree. Both are what a developer reaches for
to inspect *the store*, and routing them would remove the only way to see what
is actually on disk while a daemon is up — which is also what makes them the
control in `e2e/test_daemon_reads.py`.

**Neither read was a copy of the `forget` route, and the reason is the
constraint this item had been missing:** the daemon's tool surface is narrower
than the CLI's read commands, so a naive route trades staleness for lost
capability. `get` only became routable after `query{id}` was widened to match
what `cmd_get` resolves — a prefix, and the cold tier (`find_entity_by_prefix`
in `src/base/search.rs`, and the `cold_get` fallback both now reach through
`entity_detail_by_id` in `src/mcp/tools_query.rs`). `query` only became routable
after `tool_query` learned to return path chains, without which a routed
`kern query` would have silently lost its "--- Connections ---" section. Each
route cost a widening of the tool first; that is the shape of the remaining work
too.

Two things remain, and the title names them. `ingest` and `link` (blocked on
item 24) and `intake drain` (needs a tool first) still write the store directly.
They are one-shot, so the exposure is a lost write rather than a lost graph —
and all three are now guarded. `cmd_ingest` (`src/commands/ingest_cmd.rs`) and
`intake drain`'s `flush` (`src/commands/intake_cmd.rs`) retry through
`persist::flush_guarded`; `cmd_link` joined them 2026-07-21 via
`link_and_persist` (`src/commands/graph_ops.rs`) and `save_graph_guarded`, so a
daemon committing underneath any of them gets a refused flush and a reload
rather than a clobber. The unguarded entry point is now named
`save_graph_unguarded` and carries its precondition, which is what made the
remaining two unlocked callers visible: `cmd_hub_merge`
(`src/commands/admin.rs`) writes a destination graph it holds no lock on, and
`maybe_self_heal_store` (`src/commands.rs`) rewrites the store during boot
recovery. Both are narrower than the CLI race — hub merge stops both daemons
first, self-heal runs before the daemon serves — but neither has been proven
safe, and neither belongs to this item.

The item does not close on the read side alone.

---

# Tier 3 — the embeddable-endpoint track

kern's competitive claim is "everything a hosted service structurally cannot
do". The flip side is that a hosted service serves *many callers* and kern
assumes exactly one. This is the most valuable track in the file now that item 1 is
answered, because it converts kern from "my agent's memory" into "the memory layer
any agentic workflow embeds". It ranks below tier 1 because none of it is a
live defect, and above tier 5 because no shipped host is blocked on federation.

Two constraints hold across all of it. **ACL is caller-asserted** — the daemon
cannot verify a caller's principals, exactly like the existing
`validate_fact_source` boundary, so trust ends at the process edge. And **Facts
are GC-immune, not ACL-immune** — a Fact the requester cannot see must still not
be returned. Default semantics: empty `principals` means *no filter*, not
*public only*, or every single-agent caller goes blind.

### 18. ACL + request principal — gates everything else in this tier `[surface]`

`Entity` already carries `Acl` (`src/base/types.rs:287`; struct `{scope, users,
groups}` at `:120-124`), and it is only ever written as `Acl::default()`
(`src/ingest/place.rs:56`, `src/ingest/file_watcher.rs:136`), so nothing can
populate it. Four parts:

- Expose `principals` / `scope` on the MCP `ingest` schema
  (`src/mcp/tools_mutate.rs:19-31`), threaded through `ingest::Job` into
  `place.rs`.
- Accept `principals` on `query` — no identity param exists
  (`QueryArgs`, `src/mcp/tools_query.rs:76-107`).
- Enforce in `matches_filter` (`src/retrieval/score.rs:205-243`), which has no
  ACL predicate.
- **Guard the id path.** `src/mcp/tools_query.rs:129-137` returns
  `entity_detail_by_id(&g, &p.id)` directly, before `build_query_options`
  (`:157`) is ever called — no filter of any kind runs. Without this guard ACL is
  decorative, and the read-side route of item 9 put `kern get` behind this same
  unfiltered path.

Decide alongside: does the file watcher give `Document` entities a tenant-default
ACL, or leave them public? Recommend configurable, default public-within-tenant,
since the tenant boundary is the process. `src/ingest/file_watcher.rs:136`
hardcodes `Acl::default()` today.

### 19. `forget_by_source(scheme, object_id)` with an explicit `force` `[store]`

Deleting a source in the host must cascade into the graph. `forget` exists but is
per-entity and Facts are immune; neither symbol exists in `src/`. Needs a `force`
param that punches through the Fact guard — a legal deletion outranks
GC-immunity. **This is the only place that guard may be bypassed, and it must be
explicit, never default.**

### 20. Source-trust weighting `[retrieval]`

User-authored claims should outrank auto-ingested claims of equal heat.
`apply_boosts` has no source-trust prior (`src/retrieval/score.rs:84-96`). Add
`source_trust_user` / `_agent` / `_auto` to `RetrievalConfig`, default all `1.0`
so ranking does not move until configured, and multiply in the boost step —
**post-fusion, not in RRF**, which is rank-based. Independent of 21; can run
parallel after 18.

### 21. Review / draft lifecycle `[surface]`

`ReviewState` on `Entity` (added with a store format-version bump — alpha
rejects old stores rather than defaulting them) + source-level review policy in
config + an
`exclude_pending` query filter and a `promote` tool. Lets a host hold
auto-distilled claims out of retrieval until a human curates them. No
`ReviewState`, `exclude_pending` or `promote` exists in `src/`. Requires 18's
`QueryOptions` work first — review filters are more `matches_filter` predicates.

### 88. A retention that lands on a duplicate is silently dropped `[ingest]`

Item 22 gave `retention_secs` a writer, but only where an entity is *created*.
`place_document` and `place_chunks` both return through `find_duplicate` →
`update_existing_entity` (`src/ingest/dedup.rs:23`) before they ever reach
`new_statement_entity`, and that merge touches text, confidence and kind — never
`valid_until`. So ingesting text you asked to expire in an hour, over text
already in the graph, reports `deduped` and leaves an entity that never expires.
The caller is told it deduped; it is not told the TTL went nowhere.

Ranks above 89 because it is a correctness gap in a shipped flag rather than
coverage the flag never claimed, and below tier 1 because it is reachable only
by opting into retention. The fix is not one line: `valid_until` is LWW with a
lamport/producer pair and a pending delta (`place.rs:124-139`), so a merge that
sets it has to stamp and push the same way or the value cannot federate — decide
alongside whether a *shorter* incoming retention may shorten an existing
deadline, or only extend it.

### 89. Retention exists on two entrances of four, and in no config `[ingest]`

`retention_secs` is on the MCP `ingest` schema and the `kern ingest` flag. It is
absent from the `.txt` distillation path (`drain_entry`,
`src/ingest/intake.rs:131` — the per-claim `Config` there sets `valid_from` from
the distilled claim and nothing else), from the file-watcher sink
(`src/ingest/file_watcher.rs`), and from `IngestConfig`
(`src/config/ingest.rs:7`, whose only key is `dedup_threshold`), so a host
cannot say "everything from this source expires in 30 days" — the exact sentence
item 22 was named for. The config key is the load-bearing half: per-*source*
retention is a policy, and a policy expressed only as a per-call argument has to
be remembered by every caller.

### 24. RPC socket has no auth `[surface]`

`FEATURES.md:607-608`. The missing auth is the same boundary as 18's
caller-asserted principals — decide them together or the principal stops at the
MCP surface only. The item's second half is **retired 2026-07-21 — verified
false**: `KernRpc` does not mirror MCP 1:1 and never did. The contract is four
methods (`health`/`shutdown`/`call_tool`/`list_tools`,
`src/trnsprt/src/kern_rpc/svc.rs`) and every tool reaches the daemon through the
one `call_tool` passthrough, so there is no second dispatch copy to drift
against repo law 3 — as the closed list already recorded while this item claimed
the opposite.

---

# Tier 4 — scaling cliffs

None of these is wrong today. Each converts "works on my corpus" into "does not
work at 10×". Item 1's instrument now exists, so a fix here is judgeable the way
tier 8 is — but only at the corpus size `e2e/` builds, which is small: the cliff
itself stays unmeasured until something generates a large one. Latency is the
half the harness can already claim.

### 25. O(N) importance scan per retrieve `[retrieval]`

`seed_important` iterates `g.all()` × `kern.entities.values()`
(`src/retrieval/seed.rs:127-174`), called unconditionally once per retrieve
(`src/retrieval/query.rs`, in `retrieve_profiled`). Rayon-parallel, but still full-corpus per
query. Top structural debt in the repo.

### 26. PageRank runs a full power iteration per query, persisted nowhere `[retrieval]`

Up to 25 iterations over the whole entity adjacency on every retrieve, with
nothing cached between queries (`decisions/pagerank-authority.mdx:102-105`). The
second query-time cliff, and it was recorded on the site but in no plan.

### 27. The GC sweep is superlinear in two remaining places `[lifecycle]`

One item because one sweep pays all four costs. **Two are closed**; the two that
remain are the scans, not the accumulation:

- Victim selection is O(entities) per kern per sweep (`src/tick/stigmergy.rs:87-92`).
- The cold tier is a brute-force cosine scan with no index — `cold_search` decodes
  and scores every row (`src/base/store.rs:629-648`).
- ~~`cold_cap` decodes the entire 50k-row cold table on every individual spill~~
  **Closed 2026-07-21.** `cold_spill` now calls `cold_cap_amortized`, which trims
  only once the tier is a slack margin (1024 rows) past the cap and then cuts back
  to it — one full-table pass per 1024 spills instead of one per spill. The tier
  may sit up to 2% over its cap between passes; the cap is a disk bound, not a
  correctness boundary. Direct `cold_cap` callers still get the exact trim.
- ~~`HnswIndex::delete` is O(nodes × edges), once per victim~~ **Closed
  2026-07-21.** Deletion marks the node dead (searches skip a `None` node, so it
  is immediately invisible) and queues the slot; one scrub pass clears every slot
  deleted since the last one, and a slot only enters the free list after it. A
  sweep pays one pass instead of V. Symmetry could not be used to do better:
  insert links both ways, but pruning an over-cap neighbour drops its back-edge
  while the forward edge remains, so a node's own layers are not a complete list
  of who points at it.

The previous version of this file listed "HNSW tombstone compaction — dead nodes
accumulate" here. **That was wrong.** There are no tombstones: `delete` scrubs
inbound edges, sets the node to `None`, and pushes the slot onto a `free` list
for reuse (`src/base/hnsw.rs:136-149`, scrub `:153`, alloc reuse `:109-125`),
guarded by the test "deleted slots were recycled, arena did not grow" (`:755`). The cost is the
scan, not the accumulation.

### 28. GNN training runs synchronously on the tick `[lifecycle]`

`TaskKind::GnnPropagate => do_gnn_propagate(...)` runs inline in `process_task`
on the single tick loop (`process_task`, `src/tick.rs:85`; the arm at `:97`),
stalling large kerns — and, per item
2, taking every other maintenance task down with it if it panics.

### 29. A spilled kern still carries two resident indexes `[retrieval]`

DiskANN spill is entity-index-only: `rebuild_index` (`src/base/graph.rs:286`) hardcodes
`gnn_entity_idx` and `reason_idx` to `VectorBackend::resident(...)` (`:289-290`)
while only `entity_idx` takes the spill branch (`:296-300`). The memory ceiling
is pushed back, not removed. Compounded: `disk_threshold` defaults to
`KERN_CAP_DISABLED` and nothing auto-tunes it
(`decisions/diskann-spill.mdx:131-134`, `src/config/graph.rs:20`), so the
ceiling DiskANN exists to remove is undefended in every default deployment, with
no signal on approach.

### 30. Ingest queue `enqueue` detaches with no backpressure `[ingest]`

`Worker::enqueue` fires `tokio::spawn(async move { tx.send(job).await })` and
returns immediately (`src/ingest/worker.rs:76-78`). The channel bound is 64
(`:44`); the spawn set is unbounded. Distinct from the *tick* queue, which is
bounded at 512 with real backpressure (`FEATURES.md:377`) — the two read as
one and are not.

Beside it: **the distill leg has no timeout budget** (no `timeout` in
`src/ingest/distill.rs` or `src/ingest/worker.rs`). The queue-depth half is
**narrowed 2026-07-21** — closing item 8 gave `kern intake` a
`pending=/stuck=/failed=/done=` readout (`src/commands/intake_cmd.rs:43-45`),
so the file-backed queue reports its depth; the in-process `Worker` channel
still does not. "The LLM call is the only unbounded step on the path"
(`concepts/acceptance.mdx:189-192`), and with the answer leg removed (2026-07-21)
the distill leg is now the only LLM on any path — no latency work has landed on it.

### 31. Routing and structural debt in the hot types `[retrieval]`

Recorded in `FEATURES.md` gap blocks, planned nowhere:

- Routing does a vector lookup per level, O(depth·log n), and unnamed children
  are unbounded per parent (`FEATURES.md:120-121`).
- `Entity` is a ~30-field flat struct (serialization cost on every store round
  trip) and `Kern` carries no per-kern stats — mean heat, fill ratio — that
  clustering could reuse (`FEATURES.md:84-86`).
- DiskANN is build-once; the lexical index is RAM-only (`FEATURES.md:229-230`).
- LMDB compaction is manual and offline-only, and is the only way to shrink the
  high-water mark (`FEATURES.md:300-302`).

### 32. Tree depth is an unlisted eviction bias `[lifecycle]`

The pulse reaches ~4 levels, so entities in kerns far from the root stop being
reinforced even though nothing about them changed
(`concepts/heat-and-compaction.mdx:32-35`). Retention therefore tracks tree
position, not usage — directly against the stigmergy thesis, and invisible to
any metric that does not exist yet (item 1).

---

# Tier 5 — federation, the rest

All of this is gated behind gossip being on, which is off by default and marked
`building`. Item 33's transport work is the hinge: several items below cannot be
built without it, and the hub's phase 3 ships with it.

Phase 1 landed inline — lamport-stamped LWW on `Reason.score` and `valid_until`
(`src/base/merge.rs`), a `PendingDelta` queue and a `start_delta_flush` Delta
sender. `crdt.rs` is still 90 LoC of `GCounter` only; the LWW semantics live as
inline fields, not named types. Fine. The OR-Set-for-`statements` plan was
**reversed, not deferred**: `id == content_hash(text)`, so importing remote
statement text both breaks content-addressing and resurrects locally-cleared
statements. Merge never imports them (`src/base/merge.rs:115`) and the wire
target is rejected on receipt (`src/gossip/handler.rs:502`), kept as a refused
variant so an older peer cannot inject text under a content-addressed id.

### 33. Transport security `[federation]`

Raw TCP, no TLS. `network_id` broadcast cleartext over UDP multicast. No
signature on `GossipMessage` — it carries `kind` / `id` / `origin` / `payload`
only (`src/gossip/types.rs:18-23`) — and `handle_conn` accepts any stream while
`handle_peer_exchange` trusts any `msg.origin`. Needs `tokio-rustls` + `rcgen` as
direct deps; neither is in `Cargo.toml`. **This one gates any deployment off a
trusted LAN / WireGuard mesh**, and it gates the counter-slot identity half of
item 13.

### 34. The `Question` path is an unauthenticated membership oracle `[federation]`

A peer sends `Question` messages carrying arbitrary embedding vectors and gets a
yes/no on whether you hold a fact above cosine 0.80. Content existence is
extractable one probe at a time without ever receiving the content.

**Mitigated 2026-07-21, not closed.** A per-origin budget (`src/gossip/rate.rs`,
30/minute) makes bulk extraction expensive; the oracle is still there for a
patient prober, and `origin` is an unauthenticated self-declared string, so the
budget is evadable by rotating it. Closing this needs an identity to refuse on —
**gated on item 33**, and the same rotation problem is item 35.

### 35. Namespace rotation is unbounded storage `[federation]`

`network_id` / `kern_id` are attacker-chosen, and the quarantine cap is global
per remote kern, not per peer (`concepts/security.mdx:243-246`,
`decisions/knowledge-not-gradients.mdx:113-114`). One host cycling identifiers
creates unlimited `remote-*` kerns, each with its own 50k allowance. Item 39's
Sybil work covers ranking; this is disk.

### 36. Anti-entropy `[federation]`

No `AntiEntropy` variant in `GossipKind` (`src/gossip/types.rs:7-15`). The sender
sorts by heat and truncates to `ENTITY_SYNC_BATCH = 32` per heartbeat
(`src/gossip/handler.rs:156`, sorted `:181`),
so cold entities may never propagate and a partitioned node that rejoins never
catches up. (`Fetch` is live — `wire_fetch` installs the handler at
`src/commands.rs:1003` and the question path issues it — but it is single-id, not a
catch-up mechanism.) Two pieces adopted on paper and unscheduled: **back-off
pacing** with exponential jitter keyed to a divergence estimate
(`docs/kern/fl-vs-knids-federation.md:163-168`), and **batch-size / push-vs-pull
tuning** at scale (`howto/memory-bank.mdx:149-150`) — the top-32 is hard-coded and
the push-only choice was never revisited.

### 37. Backpressure, divergence metric, and delta write-lock starvation `[federation]`

The only per-origin budget is the `Question` one item 34 records
(`src/gossip/handler.rs:318`, 30/min); the `Delta` path — the one that takes the
write lock — has none. `HealthStats` has no divergence field
(`src/base/health.rs:4-34`). Sharper than previously recorded: the four
full-corpus loops in `handle_crdt_delta` (`src/gossip/handler.rs:432`, `:448`,
`:461`, `:482` — two `g.all_ids()`, two `remote_kern_ids(&g)`, which is
`all_ids()` filtered at `:545`) run under the graph **write** lock, once per
inbound delta, unlimited — a cheap remote write-lock-starvation vector
independent of the local-row mutation in item 13.

~~Beside it: `start_entity_sync` clones the entire local corpus every
heartbeat~~ **Closed 2026-07-21.** `hottest_local` selects over references and
deep-clones only the winners — linear, with the same comparator and therefore the
same chosen set. The rest of this item (per-peer rate limits, a divergence field
on `HealthStats`, and the write-lock starvation from the four full-corpus loops in
`handle_crdt_delta`) is still open. The `Delta` path still has no per-origin
budget; only `Question` does.

(Remote heat is no longer pinnable: entry to a `remote-*` kern strips heat,
access counts and confidence to neutral — `src/base/merge.rs:20`, applied `:153`.
The pin risk that remains is item 15's unclamped `Pulse`.)

### 38. Peer authority is unbuilt, so Sybil defence has nothing to weight `[federation]`

No DB- or peer-level authority signal exists: "a peer cited by thousands of other
peers was treated identically to a brand-new peer"
(`decisions/pagerank-authority.mdx:96-98`). The full design — an
`AuthorityTable`, TrustRank seeding, `authority_weight` / `authority_floor`
config, a `kern authority seed` admin command — is written out at
`docs/kern/pagerank-authority.md:66-71, 184-198, 216-220, 257` and was never
scheduled. It ranks immediately above the Sybil work because the defences in 39
are weightings of a signal that does not exist.

### 39. No Sybil defence is in effect `[federation]`

And, corrected on inspection, none ever was. Two were written and never wired:
`RateClipper` (the since-deleted `gossip/sybil.rs`, 175 LoC) whose `set_clipper()`
had no call site in any commit, and `trimmed_mean_merge_hits`
(`gossip/merge.rs`, 241 LoC), self-described as "a Sybil-resistant alternative"
for fusing per-peer hit lists, also callerless. Both were deleted in `dc02a18` as
verified-unreachable; the deletion changed no behaviour because neither had ever
run. This is **unbuilt work with a reference implementation in git**, not a
regression — materially cheaper than it first appears. The layered defences from
the authority design (edge-weight caps, pulse-coupled edge validation, temporal
slashing of frequently-superseded producers) were never written at all.

### 40. Remote-injected text is retrievable and reaches an agent's context `[federation]`

Remote entities are vector-indexed on insert, so with gossip on, recall output —
and therefore any agent consuming it — extends to every host on the segment
(`concepts/security.mdx:247-251`). Bounded by ranking-signal stripping
(`src/retrieval/score.rs:101-111`), not by exclusion. Decide whether `remote-*`
should be opt-in-per-query rather than indexed by default.

### 41. Two FL-derived bounds adopted on paper, neither in effect `[federation]`

Trimmed-mean / median materialisation for federated scalars (written, never
called, deleted in `dc02a18` — see item 39; recoverable from git), and a
provenance ledger of per-thought `(origin, lamport, confidence)` enabling
retrospective down-weighting of a peer later deemed untrusted, which was never
written — the shipped `Ledger` (`src/gossip/ledger.rs`) is a TTL- and cap-bounded
routing cache (`:24, :28, :63`), enough to know where to fetch, not who told you
what. Adopted-partial and unscheduled beside them: **secure aggregation for
pulses and counters** (`docs/kern/fl-vs-knids-federation.md:131-136`).

### 42. The gossip wire has no version negotiation `[federation]`

The `anchor_*` → `graviton_*` rename "breaks federation peers that predate the
change" (`concepts/stigmergy.mdx:173-175`) and nothing on the wire negotiates a
version. Known impact of planned changes: `GossipMessage.signature` is breaking
(alpha: peers upgrade together, no wire mitigation); `AntiEntropy` is additive. Confidence isolation
(`conf_alpha` / `conf_beta` / `unlinked_count` never imported from remote) must
survive every change — and note that `decisions/crdts-over-consensus.mdx:116-117`
frames the same `unlinked_count` behavior as a **PN-Counter that was never
built**, where this file treats it as a load-bearing invariant. Reconcile: it is
an invariant.

### 43. CRDT growth and re-embedding across replicas `[federation]`

Two leads from `docs/kern/crdts-federation.md`, adopted and never scheduled:

- **Tombstone and LWW-history growth is unbounded** (`:259-261`); the note's own
  follow-up was "time-bounded compaction".
- **Vector LWW is coarse across heterogeneous embedding models** (`:264-266`),
  and `docs/kern/fl-vs-knids-federation.md:200-204` explicitly *allows*
  per-node model choice. Item 3 covers the local swap; the federated case — no
  model-identity stamp on the wire — is separate and unfunded.

### 44. Bi-temporal stamps are never federated `[federation]`

`valid_from` / `valid_to` / `invalidated_at` are `#[serde(skip)]`
(`src/base/types.rs:302-307`), so each node re-derives its own `as_of` view and
two *converged* nodes can answer the same point-in-time query differently
(`docs/kern/crdts-federation.md:54-62`). The federated twin of item 4.

### 45. Multicast discovery is unreliable with no health signal `[federation]`

Wireless APs, container bridges and VPN interfaces all break it, with no
fallback and no way to distinguish discovery-failed from no-peers-present
(`concepts/federation.mdx:68-70`).

### 46. One fresh TCP connection per gossip message `[federation]`

`TcpStream::connect` per call at `src/gossip/transport.rs:37` (`send_msg`) and
`:45` (`send_and_receive`). No pooling. Separately, the `trnsprt` client has no
pooling either (`FEATURES.md:832-833`) — that one is not gossip and is not gated
on 33.

### 47. Hub phase 3: gossip moves hub-side `[hub]`

One UDP endpoint and one node identity per machine; nodes stop binding the
network entirely (`src/config/gossip.rs:7-16` today). **Ordering decided
2026-07-20:** the senders and semantics build per-node first; this transport move
ships together with item 33 — same wire layer, migrate once. Not blocked,
sequenced. One clause of the previous version was wrong: there is **no** per-project
port-clash validation in `src/config/serve.rs` to collapse. (Corrected again
2026-07-21: that file is not `mcp_token` handling only — `ServeConfig` is
`{mcp_token, mcp_addr}` at `:6-11`, and `mcp_addr` is the reader-less field
item 84 owns.)

Beside it: **hub↔node version skew is unmanaged** beyond same-binary spawning
(`FEATURES.md:938-939`).

### Decisions owed before the federation build

Deciding behavior: **none yet — amend first.**

- (a) *Subsumed by item 13* — `Reason.score` stays LWW. It was owed as a
      trust-signalling question (max-join would silently revert deliberate
      `degrade_entity_reasons` lowering); the local-row exposure settles it.
      Recorded here rather than deleted so the reasoning survives.
- (b) Anti-entropy watermark shape: vector clock or content-hash bloom?
- (c) TLS cert authority: operator PKI or TOFU pin?
- (d) Does `network_id` derive from the cert or stay config-owned?
- (e) Does graviton `mass` federate at all, or stay per-node tuning? Two peers can
      currently disagree on a graviton's pull.
- (f) **New.** `superseded_by` conflicts resolve to the lexically greater id,
      which "guarantees both replicas agree, not that they agree on the better
      answer" (`concepts/federation.mdx:205-210`). Two peers can deterministically
      converge on the wrong successor. Keep, or resolve on lamport then id?
- (g) **New.** Do peers running different embedding models federate at all
      (item 43)? Today it is allowed on paper and unrepresented on the wire.

---

# Tier 6 — ingest quality

Below the cliffs because none of it is a defect, above the belief model because
every claim in the graph is shaped here first.

### 48. Source-keyed idempotency at ingest `[ingest]`

`find_duplicate` is pure cosine-over-HNSW (`src/ingest/dedup.rs:8-21`), which is
paraphrase-evadable. **The shape of this item changed and the old wording is
retired:** `CHANGELOG.md` 2026-07-20 shipped chunk external ids keyed on the full
source identity (`source_id()` + chunk index, not the bare section), and CLI
`kern ingest` deriving its inline source hash from the text. What remains is the
*dedup* key, not the external id. Beside it: the dedup threshold is global, not
per-kind (`FEATURES.md:365`).

### 49. The distill prompt is one-shot and global `[ingest]`

One `format!` over the whole conversation, no per-kind branch, no chunking
(`src/ingest/distill.rs:28-47`). The `kind` taxonomy has overlapping categories
(decision/project, fact/code-fact) and label accuracy was measured at ~33% even
at 7B — **that figure came from the deleted harness and is unreproducible; treat
it as a lead, not a number** (item 1's claim standard). Long deltas are not
chunked at all.

### 50. Intake distillation lacks relative-date resolution `[ingest]`

The prompt injects no current date (`src/ingest/distill.rs:29-47`), and
`valid_from` is only requested when the statement states an absolute date — so
dropped text containing "last Tuesday" stores unresolved. The eval path got this
and the product path never did; the eval path is now deleted, so the capability
exists nowhere.

### 90. `DirectJob` carries `valid_until` but drops `valid_from` `[ingest]`

The durable direct intake serializes one bi-temporal stamp and not the other:
`DirectJob` (`src/ingest/direct.rs:11-21`) has a `valid_until` and no
`valid_from`, and `drain_direct_once` overlays only the former onto the drain
loop's `Config`, so `valid_from` is whatever the loop's shared config says —
always `None`. **Not a live loss**: the only producer of `valid_from` is the
distillation path (`intake.rs:191`, from `distill.rs`), which calls the worker
directly and never goes through `direct/`, and the MCP `ingest` schema has no
`valid_from` field to lose. It is a hole that opens the moment either of those
changes — which item 50 and item 89 would both do. Ranks here, next to 50, for
that reason and not for any damage it does today.

### 51. Require reason text on supersede `[ingest]`

`ReasonKind::Supersedes` edges are minted at `src/base/accept.rs:438` and `:533`
with `fallback_label()` text (`src/base/types.rs:103`), never a caller-supplied
rationale. The *why* is the thing the graph exists to hold.

### 52. A single-line graviton seed still truncates at the embed context window `[ingest]`

**Narrowed 2026-07-21 — the old wording is retired.** It said "acknowledged in
source at `src/mcp/tools_admin.rs:116`" with "chunk + mean-pool" as the unbuilt
upgrade path. Both halves moved in `08c9971`: the acknowledgement comment was
deleted, and chunk + mean-pool **shipped** for the multi-line case —
`seed_examples` (`src/base/accept.rs:612-624`) splits a seed on newlines and
`mean_pool` (`:626`) averages the per-line embeddings, wired at
`src/mcp/tools_admin.rs:119-136`. Line 116 now carries the mean-pool rationale,
i.e. the opposite of what it was cited for.

What is left is the case `seed_examples` deliberately does not split: a seed
with fewer than two non-empty lines is embedded whole (`:619-621`), so one long
paragraph still goes to the model as a single call and truncates past its
context window with no signal. Chunking *that* wants a length-based split, not a
newline one, and is still blocked on a real document long enough to truncate.

### 53. Clustering is vector-only `[lifecycle]`

No semantic or structural features (`FEATURES.md:437`), and naming plus
enrich are a cold LLM call per kern. The adopted-but-unbuilt upgrade is
thought-level PageRank feeding the split heuristic — high-rank nodes become
gravitons, bridge nodes become sub-kerns
(`docs/kern/pagerank-authority.md:242-247`, `decisions/pagerank-authority.mdx:120-121`).
Graph structure informs ranking today and never informs the tree shape that
routing depends on.

### 54. GC has no convergence gate `[lifecycle]`

The adopted loop-closing design gated forgetting on convergence — `G ≥ 0.6`
**and** heat below floor for `forget_ttl`
(`docs/kern/stigmergy-self-improving.md:228-233`). Shipped GC has no gate at all.
Depends on item 62 (the convergence metric) existing.

### 55. Two freshness signals, different half-lives, neither ever tuned `[retrieval]`

A 24-hour one for ranking (`qbst_recency_half_life_secs`,
`src/config/retrieval.rs:31`, defaulted from `QBST_RECENCY_HALF_LIFE`,
`src/base/constants.rs:12`) and the retention one on `HeatConfig`. The offline
NDCG sweep meant to tune either was never run
(`decisions/stigmergy-over-gardening.mdx:117`). Third input nobody reconciled:
`docs/kern/stigmergy-self-improving.md:160-170` derives a 1–2 day half-life.

**Restated 2026-07-21 — the old "7-day retention" wording was stale.** The 7 days
at `src/base/heat.rs:18` is the struct default and is never what runs:
`Config::load` applies the preset unconditionally (`src/config/mod.rs:104`,
`:132`) and `Preset::apply` is the only writer of `heat.half_life_secs`
(`src/config/preset.rs`). The shipped default is `relaxed` = **30 days**; medium
is 7, tight is 3. So the two signals are 24h vs 30d by default, and the gap to
the derived 1–2 days is a factor of 15–30, not 3.5. The knobs also stopped being
config edits — a retention retune is now a commit against `preset.rs`, which is
item 87's surface, and the two should be swept together. Now measurable:
`e2e/test_recall.py` scores a half-life change directly
(`recall@1`/`recall@5`/`MRR`), which is the sweep that was never run.

---

# Tier 7 — belief model

`decisions/bayesian-confidence.mdx` and `decisions/edit-convergence.mdx`. None
funded before now. Ranked here because the model is coherent and merely
incomplete — no item below produces a wrong answer today.

### 56. An agent cannot register disagreement at all `[ingest]`

There is no `Contradicts` reason kind (`src/base/types.rs:77-86`) and no `stance`
parameter on the ingest schema (`src/mcp/tools_mutate.rs:19-31`);
`observe_contradict` (`src/base/types.rs:411`) has exactly one caller, GNN
alignment (`src/tick/gnn_propagate.rs:157`). Observer-reputation weighting is
also unbuilt.

### 57. No evidence decay `[lifecycle]`

`conf_alpha` and `conf_beta` only grow — the sole zeroing is the remote strip
(`src/base/merge.rs:25-26`) — so stale consensus takes proportionally many new
observations to unseat. Tick-based γ damping is an open design
(`decisions/bayesian-confidence.mdx:137`).

### 58. Supersede chains are unbounded while contested `[lifecycle]`

No `ReasonKind::Edit` rationale edge (`src/base/types.rs:77-86`) and no producer
rate-limit, so an A/B ping-pong on one `external_id` grows without bound
(`decisions/edit-convergence.mdx:107`). Compounding it: the three trigger
conditions that would flip kern to full versioning have **no instrumentation**
(`docs/kern/wikipedia-edit-convergence.md:100-105`), so the flip is undetectable
even in principle.

### 59. `degrade` has no floor, no audit trail and no undo `[retrieval]`

It lowers edge scores unboundedly (`howto/mcp.mdx:104-106`,
`concepts/why-kern.mdx:127-129`); nothing stops repeated degrades from
permanently erasing a correct path, and nothing records that they happened.

### 60. No re-classification when a contradiction pair changes `[lifecycle]`

Either side of a classified pair can move and nothing re-runs the call, and no
tool exposes the supersede chain beyond `include_history` (`FEATURES.md:160-161`).
Two open questions beside it, from `docs/kern/bayesian-belief.md:145-148`: should
`Reason` edges carry belief symmetrically, and does superseding reset or inherit
belief?

---

# Tier 8 — retrieval quality, now measurable

These were unacceptable-in-principle until an instrument existed. It does now
(`e2e/test_recall.py`, and the invariants beside it), so each of these can be
judged the way item 86's two candidate fixes were: apply it, measure, keep it only
if `recall@1` holds and the exact-match probe stays at rank 1. They are ranked
among themselves by expected effect, not by confidence.

What the instrument still cannot judge, and what that costs: the fake embedder is
bag-of-words, so a change that only a real embedding model would reward looks
neutral here. Item 64's stemmer swap is the clearest case.

### 61. Move `merge_hits` onto RRF `[retrieval]`

The raw `0.4·content + 0.6·GNN` blend (`src/base/search.rs:60-61`, applied `:74`)
is fragile across scales; `fuse::rrf` (`src/retrieval/fuse.rs:4`) already fuses
the seed layer (`src/retrieval/query.rs`, `fuse_hybrid_seeds`) and never reaches `merge_hits`.
**Record the cost:** RRF keeps rank information only, so a dense hit that is
overwhelmingly better than rank 2 gets no credit for the margin
(`concepts/retrieval.mdx:88`). This is a trade, not a strict win.

### 62. The self-organisation claim is unmeasured `[retrieval]`

The convergence metrics — Gini over access, top-10 stability — were never built
(no `gini` anywhere in `src/`), so "the corpus converges on efficient paths", a
central product claim, is a design intention
(`decisions/stigmergy-over-gardening.mdx:128`). Belongs with item 1's
replacement. Its surfacing half is separate and also unbuilt: export `HeatStats`
via health and `kern://health`
(`docs/kern/stigmergy-self-improving.md:263-266`). Item 54 depends on this.

### 64. Normalize and re-found the scoring stack `[retrieval]`

Three that must be judged together, since each moves the others:
min-max normalize `apply_boosts`, which is purely additive and unnormalized today
(`score * confidence + boost + fact_bonus`, `src/retrieval/score.rs:84-96`); swap
the hand-rolled stemmer (`src/base/lexical.rs:206`, no stopword list, no
`rust-stemmers` in `Cargo.toml`) for `rust-stemmers` 1.2.0 + stopwords, which
needs a BM25 rebuild; and validate-or-remove GNN reranking, whose only expression
is the 0.6 blend in item 61.

### 65. Rank on the lower confidence bound `[retrieval]`

`p − k·√var` instead of the mean (`docs/kern/bayesian-belief.md:135`) — a
one-line ranking change that makes a single-observation claim stop outranking a
well-evidenced one at equal mean.

### 66. RRF weights and mode blends are configurable but never auto-tuned `[retrieval]`

Was two ceilings; the rerank half left with the rerank stage itself
(2026-07-21). What remains: RRF weights plus mode blends are configurable but
never auto-tuned (`FEATURES.md:197`).

### 67. Binary quantization stays non-user-selectable `[retrieval]`

Its recall floor is too low without a rescoring pass; deliberately excluded from
`parse` (`src/quant.rs:20-21`). Beside it: no int4 path and the quantization
scale is fixed at encode time (`FEATURES.md:249`).

### 69. Speculative decode for the distill leg `[ingest]`

qwen3.5:0.8b draft → 4b generator. With the answer leg gone (2026-07-21) the
only LLM latency that matters is distillation throughput; no `draft` or
`speculative` anywhere in `src/llm.rs`. Latency is the one axis item 1 does not
gate — the e2e harness can still judge this.

---

# Tier 9 — process, packaging, and things that rot unnoticed

Last because none of it affects a running kern. First within its tier because a
contract nobody enforces is a contract nobody has.

### 70. The oracle pre-commit hook is untracked and has no installer `[process]`

`ORACLE.md` rule 1 is enforced by `.git/hooks/pre-commit`, which lives only in
`.git/` and is created by nothing in the repo — no `justfile` recipe, no
`install.sh` step, no `.pi/update.sh` line. A fresh clone has **zero enforcement
of the ruling every commit is supposed to answer to**, and nothing announces it.
The hook itself calls this out as a "per-clone install product"; the install half
does not exist. Wanted: track it under `scripts/` and install it via
`core.hooksPath` or a `just` recipe run by `.pi/update.sh`.

### 75. Crash consistency on the DiskANN path `[store]`

The disk graph, the vectors and the bincode metadata can diverge on a mid-write
crash; there is no WAL and no atomic-rename-per-segment
(`docs/kern/diskann-disk-index.md:117-118`). Adopted as a known risk, scheduled
nowhere. Beside it: mmap file-locking and flush semantics differ on Windows
(`:112-113`), and PQ codebook training/drift has no retrain trigger — "a bad
codebook silently degrades recall" (`:110-113`) — which lands in item 1's lap
the moment PQ is promoted out of the non-goals.

### 76. The watchdog force-exit skips the final guarded flush `[store]`

It force-exits with 101 on a 30s async stall
(`concepts/architecture.mdx:300-301`), and that path skips the flush
`howto/install-run.mdx:187-190` says is required to avoid losing RAM-only state.
Combined with item 10, the default posture can lose up to a tick interval of
writes with no log.

### 77. Hash composition is an unguarded breaking change `[store]`

"Changing how a hash input is composed is a breaking change to every existing
graph" (`concepts/graph.mdx:86-88`). Repo law 1 guards bincode schema round
trips; nothing guards or versions hash composition, and there is no migration
path. Wanted: a round-trip test over `content_hash` inputs, same shape as the
bincode guard.

### 78. A non-local LLM URL egresses everything, silently `[surface]`

"The full text of everything kern captures transits that provider"
(`concepts/security.mdx:81-96`) when a non-local endpoint is configured — no
redaction, no allowlist, no warning at config load, no egress log. For a project
whose first claim is "local-first, zero egress", the one setting that voids it is
unremarked.

### 79. `validate_fact_source` is dead code `[surface]`

Called **once** (corrected 2026-07-21 — it was twice; the second site left with
the ingest `kind` arg in `216730d`), with the literal `AGENT_SOURCE`
(`src/mcp/tools_mutate.rs:115`), and it accepts `USER_SOURCE` / `AGENT_SOURCE`
(`src/base/validate.rs:22`), so it can never fail. Decision:
thread a real auth identity (item 18/24), or delete. Delete is correct for a
single local daemon and needs only sign-off.

### 81. `resources/list` and `prompts/list` return `-32601` on the proxy path `[surface]`

`ProxyServer` implements `tools_list` / `call_tool` / `extra_capabilities` only
(`src/commands/mcp_cmd.rs:290`, `:306`, `:343`) with no `handle_method` override,
so the trait default returns `None` (`src/trnsprt/src/server.rs:21`). Meanwhile
`extra_capabilities` advertises `{"resources": {}, "prompts": {}}` (`:346`) to
match standalone, which *does* serve them (`src/mcp.rs:208-211`, advertised `:157`). Advertised on
the normal path, non-functional there. Either forward them or stop advertising.

### 82. Standalone `kern mcp` runs no gossip `[surface]`

**Corrected:** the previous version said "no maintenance tick and no gossip". The
tick *is* started (`src/commands/mcp_cmd.rs:455-466`); only gossip is absent
(`broadcast_q: None` at `:461`, `broadcast_pulse: None` at `:475`). A graph
served that way decays, clusters and GCs normally, and simply does not federate.

### 83. Per-kern entity cap is `KERN_CAP_DISABLED` and marked unsafe to enable `[lifecycle]`

`max_kerns` and `disk_threshold` both default to `usize::MAX`
(`src/config/graph.rs:18,20`, `src/base/constants.rs:30`).

### 84. Remaining operational odds and ends `[surface]`

- **`serve.mcp_addr` is a config field with no reader.** Added when item 11
  landed; `src/commands.rs` still resolves `cli.mcp_addr` alone, so setting it in
  `kern.toml` starts no listener and warns about nothing — a silently-ignored
  setting shipped by the tier that exists to delete them. Needs the CLI-over-config
  resolve at the one call site.
- **`kern merge` still defaults on a broken foreign config.**
  `src/commands/admin.rs` does `Config::load(..).unwrap_or_else(..)` for both the
  source and destination roots, the exact swallowing item 11 deleted at boot.
  Not a mechanical port: a merge should probably refuse rather than default, which
  is a decision to record.
- **`num_ctx` / `keep_alive` / `num_gpu` cannot warn when ignored on the `/v1`
  path**, because they are not config keys at all — they are constants in
  `src/llm.rs`, and the native-vs-compat decision is a private fn there. Warning
  requires either promoting them to real per-endpoint config or exposing that
  predicate. Was listed under item 11; it is a different job.
- Hand-rolled tool schemas; no batch query
  (`FEATURES.md:566-567`).
- The LLM client is Ollama-centric with no retry/backoff policy object
  (`FEATURES.md:806-807`).
- Watcher `.gitignore` parsing is approximate; no rename tracking
  (`FEATURES.md:984-985`).
- `unnamed` lists only; there is no `promote` (`FEATURES.md:711`).
- GNN has no GPU path, weights are per-kern rather than shared, and the objective
  is link-prediction only (`FEATURES.md:528-530`).
- Under WSL2 NAT a loopback Ollama URL must be hand-pinned; kern neither rewrites
  nor warns (`FEATURES.md:1000`).
- RPC socket bind→chmod race — sub-millisecond, umask default — recorded as an
  accepted risk (`concepts/security.mdx:40-43`); revisit only if the umask
  alternative stops being worse.

### 87. Do the preset tiers earn their numbers, now that `relaxed` is the default? `[eval]`

**Deciding behavior: verify-before-claiming.**

The preset tiers shipped 2026-07-21 with hand-picked values, and the default
flipped from the medium-era values to `relaxed` the same day
(`src/config/preset.rs`). Every prior measurement — including the LoCoMo
0.137 baseline — ran on medium-era defaults, and a configless run now
exercises `relaxed`. Question: run the retrieval suite once per preset;
first decide whether the standing baseline is re-pinned to `preset =
"medium"` or re-recorded on `relaxed`, then adjust any knob whose tier value
scores worse than medium on the posture it claims to serve (relaxed →
recall, tight → precision). Preset values live in code now, so adjustments
are commits, not config edits. Blocked on nothing; gated only on an idle GPU
and item 1's instrument staying the scorer.

### 85. Documentation owed, tracked here so it is not lost `[docs]`

- `kern hub` appears on no site page, including its 1800s idle-unload default —
  a user whose daemon vanishes has nothing to read. Verified absent: `grep -rl
  "kern hub" docs/site/content/docs/` returns nothing.
- The `id` path in `query` bypasses every filter (item 18) — undocumented at
  `howto/mcp.mdx:50`, which says only "Needs `text` or `id`" and then lists the
  filters as if they applied to both.
- (retired 2026-07-21 — the tables were filled in) the `move` MCP tool is listed
  in `README.md:352` and `FEATURES.md:550`, and the site's count is now twelve
  (`howto/mcp.mdx:5, :75`), so the "Eleven tools" note is retired with it.
- `docs/kern/README.md:60` declares the directory holds "never plans"; five of
  its notes contain execution plans, migration stages and phase orderings
  (`diskann-disk-index.md:30, :86-105`, `crdts-federation.md:215-255`,
  `fl-vs-knids-federation.md:248-254`, `stigmergy-self-improving.md:298-301`,
  `pagerank-authority.md:279-280`). Either the notes lose their plans to this
  file, or the declaration is false. Repo law 4 says the former.
- `README.md:116` still sells "**tarpc `KernRpc`**" on the front page. There is
  no tarpc in `Cargo.toml`, in `Cargo.lock` or anywhere in `src/` — the service
  is generated by this repo's own `service!` macro (`src/trnsprt/macros/`), and
  `FEATURES.md` §13 says so outright. A dependency we do not have, named where a
  reader looks first (found 2026-07-21, same sweep that corrected the identical
  line in `SPECIALISTS.md`).
- (retired 2026-07-21 — corrected at the source) the four stale `docs/kern/`
  research-note claims are gone: `crdts-federation.md:6-14` now states outright
  that no `PnCounter` was ever built, that `Delta` has a live sender, and that
  OR-Set for `statements` was *reversed, not deferred*;
  `diskann-disk-index.md:28-29` marks the PQ recall claim **withdrawn**.
- (retired 2026-07-21 — withdrawn in place) the quality claims item 1's standard
  forbids no longer survive. `FEATURES.md:182` keeps only the `+7% p50` latency
  half and says the retrieval-quality half is withdrawn;
  `docs/kern/diskann-disk-index.md:26` says the note "previously published" the
  `recall@10 ≥ 0.90` figure; this file's own "recall@10 A/B" citation was struck
  earlier.
- (retired 2026-07-21 — `README.md` and `VISION.md` were corrected) neither
  opens on "takes in durable facts from your sessions" or "learns on its own"
  any more; `VISION.md:51` now *states* there is no recorded baseline instead of
  gating claims on one; `README.md:393` says the Question and Pulse senders and
  the fetch RPC are live, and `:398` pins the version at `1.1.0`, matching
  `FEATURES.md`.
- (retired 2026-07-21 — all three fixed) `FEATURES.md:54` now lists `Entity`'s
  `acl`; the retired query-cache finding is gone from the file entirely; and
  `:568` marks prompts and resources "served on the standalone path only",
  which is item 81's note.

Deferred design calls, still owed, no blocker and no urgency: quarantine
representation (bool vs `EntityStatus::Quarantined` vs `Source` trust band);
contradiction-reconcile gating band; temporal-aware as-of retrieval scoring;
episodic abstraction as a tick task; chunking strategy (contextual-prepend vs
proposition self-containment). The threat model and staged hardening list
(ed25519 signing, peer trust, Sybil binding, replay protection, ACL enforcement)
lived in `docs/kern/safety-architecture.md`, deleted 2026-07-20 for stale paths —
recover from git history when tier 5 opens. A private-disclosure policy still
exists at `docs/FEDERATION-SECURITY.md:24-28` with nothing behind it.

---

# Context — not work

## North star

kern is the memory layer an agent recalls from: local-first, in-process,
per-cwd, offline-capable, self-forgetting, with no query-time LLM on the default
path — and it **retrieves the right thing**, provably.

**There is no recorded baseline.** The LoCoMo eval, the retrieval bench, and
`docs/kern/locomo-baseline-2026-07-19.json` were all deleted in `8d8b19e`
(2026-07-20). That deletion was correct and is not to be undone as-was: the
LoCoMo score collapsed ingest × retrieval × answering into one LLM-judged number
in which the **answering term dominated**. Measured the same day, a grounded run
— whole conversation in the prompt, kern bypassed entirely — scored 0.187 on a
slice where kern scored 0.027. The ceiling was set by a 3B answerer, not by
memory, so the number could not steer memory work. Three eval-side prompt changes
moving one slice from 0.131 to 0.027 in a single day confirmed it was measuring
the harness.

The previously published figures (overall 0.137 ± 0.018, "gap 0.46") are
therefore **withdrawn, not superseded** — no current number replaces them.

Claim standard, unchanged by item 1's closure: **no quality claim of any kind.**
Not SOTA, not parity, not regression, not improvement. Latency claims remain
permitted from the e2e harness. What item 1 delivered is a *scorer*
(`e2e/test_recall.py`) — it steers work and catches regressions against a
bag-of-words embedder, and it certifies nothing. The standard stands until
something can.

## How we supersede Zep / Mem0 / Letta / Qdrant

Not by matching feature lists. By owning a combination none of them hold, then
proving it — on a *comparable* measurement, which still does not exist. Item 1
shipped a scorer that measures kern against its own past, not against them.

| property | kern | Zep/Graphiti | Mem0 | Letta | Qdrant |
|---|---|---|---|---|---|
| Per-project self-maintaining graph (per-cwd) | ✅ | ❌ hosted | ❌ | ❌ | ❌ |
| Default recall touches no LLM (sub-ms) | ✅ | ❌ | ❌ | ❌ | n/a |
| Local-first, single binary, no network hop | ✅ | ❌ | ❌ | partial | ❌ |
| Self-forgetting (decay / stigmergy GC / cold spill) | 🟡 items 32, 54 | ❌ | partial | ❌ | ❌ |
| Graph + dense ANN + BM25 + GNN in one process | ✅ | partial | ❌ | ❌ | ❌ |
| Bi-temporal supersede off the recall path | ✅ local / 🟡 item 44 federated | ✅ | ❌ | ❌ | ❌ |
| Coordinator-free CRDT federation | 🟡 building | ❌ | ❌ | ❌ | ❌ |
| Published eval numbers | ❌ withdrawn | ✅ | ✅ | ✅ | n/a |

Two rows carried an unqualified ✅ in an earlier version while this same file
funded the defects that break them. They are 🟡 with the items named — a
scoreboard that disagrees with its own plan is not a scoreboard. Re-pointed
2026-07-21: the items they originally named (2, 5, 6 and 4, 17) have all closed,
and a qualifier citing a retired number reads as an open defect that no longer
exists. Self-forgetting is qualified by what is still open — tree-depth
eviction bias and the missing GC convergence gate — and supersede is ✅ locally,
qualified only where the stamps stop, at the wire.

**The three moves, in order:**

1. **Get a measurement worth steering by.** The architecture argument is won on
   paper and currently unprovable. We are the only one in this table with no
   published number — honest, and the single biggest gap.
2. **Ship what a hosted service structurally cannot.** Offline, per-cwd, zero
   egress, sub-ms default recall, self-forgetting. These are not features they
   are behind on — they are features their business model forbids. Federation is
   the same bet: no shipped competitor has it.
3. **Refuse the vector-DB fight.** Qdrant parity (PQ, payload indexes, sharding,
   RBAC, SDKs, multitenancy) is a non-goal. Mounting Qdrant as a backend yields a
   superset, not supersession, and forfeits the only structural advantage kern
   has. Repo law forbids a pluggable backend.

Closest rivals per axis: **YourMemory** (decay + published LoCoMo, claims +16pp
over Mem0 — read before quoting ourselves), **Graphiti** (temporal semantics),
**mnemo** / **AgentDB** (Rust + embedded + MCP stack), **Cognee** (self-hosted
KG). Full survey: `docs/kern/`.

## Non-goals

None of these move an agent-memory eval score. All are table stakes only in a
multi-tenant hosted-DB business kern is not in. Revisit only if that business
materializes.

Distributed sharding (Raft) · replication + write-consistency factor · API key /
JWT-RBAC / TLS-for-clients / audit logging · public REST + gRPC + multi-language
SDKs · multitenancy · GPU index building · product quantization / SPLADE sparse
vectors / ColBERT multi-vector — re-promote any one of these **iff** item 1's
replacement metric shows a retrieval-quality gap it would close.

**At-rest encryption is refused at this layer**, not omitted
(`concepts/security.mdx:61-70`): the store is a local file under the user's own
account, and encrypting it there buys nothing an OS-level FDE does not already
give while adding a key-management surface. Recorded here because a reasoned
refusal that appears in no non-goal list reads as an oversight.

**Parked indefinitely:** the v2 self-training track (LoRA in Rust, teacher
pipeline over mature graph regions, per-graviton adapters hot-swapped at query
time, adapters gossiped by content hash). Nothing built, nothing scheduled. Gate:
an overall eval score that makes specialization worth funding.

## Repo laws

1. **Append-only bincode.** Persisted enums/structs grow by appending only; guard
   schema touches with a round-trip test.
2. **No pluggable/fallback backend.** All-internal, in-process, self-contained.
3. **One dispatch core.** Every surface goes through `mcp::Server::call_tool`,
   never a second copy.
4. **This file is the only plan.** New work goes here, not into a new document.

## Closed and verified — do not re-open

- **Per-source TTL has a writer** — was item 22, closed 2026-07-21. The reader
  (`score::drop_expired`) had been waiting for one; `valid_until` is now set at
  ingest from a `retention_secs` on the MCP `ingest` schema and a
  `kern ingest --retention-secs N` flag, through the single conversion
  `ingest::valid_until_from_retention` so the two entrances cannot drift. It
  reaches the entity on every path that creates one: MCP sync, MCP durable
  direct intake (`DirectJob` carries the resolved *instant*, since the job may
  sit a whole poll interval before draining), MCP RAM enqueue, and the CLI —
  and on the chunk path as well as the document path, which were two separate
  hardcoded `None`s. `e2e/test_retention.py` proves the round trip against the
  real binary: recallable before the deadline, gone after, with a control fact
  that stays. What the item did *not* buy is now items 88, 89 and 90 — the
  dedup branch swallows a retention, only two of four entrances offer one and
  no config key does, and `DirectJob` still drops `valid_from`.
- **The intake is visible and drivable** — was item 8, closed 2026-07-21.
  `kern intake` (alias `kern intake status`) prints pending with age, the last
  error for anything stuck, quarantined `failed/` entries and the `done` count;
  `kern intake drain` runs one pass in-process so the CLI works with no daemon,
  sharing `drain_once` with the daemon's loop so the two can never diverge.
  Building it exposed the real hole: the three paths that leave a delta queued
  **recorded no sidecar at all** — no `[reason]` endpoint, a reason model
  answering prose, and a transient read error. Those are exactly the
  retry-forever cases this item exists to surface, so each now writes why, and
  `a_transcript_left_queued_records_why_it_is_stuck` holds the line. The drain
  flushes through the same guarded retry as `cmd_ingest`, so a running daemon
  makes it a refused-flush-and-reload, never a clobber (item 9 still owns the
  lock).
- **A reason edge lifts its neighbour into the results** — was item 86, the
  instrument's first finding, closed 2026-07-21. The design question — how does
  a walk pay without letting a well-connected node outrank a direct match — is
  answered with bounded source-weighted traversal credit: every edge the walk
  examines credits its far endpoint with `source_score × edge_evidence`, once
  per (edge, endpoint), summed, weighted (×2), capped (0.5), and **clamped just
  below the strongest voucher's own walk score**, so a neighbour rides up
  behind what vouched for it, never past it. The clamp, not the weighting, is
  what protects direct answers — measured directly: unclamped, `recall@1` fell
  0.9167 → 0.7639. Root cause beneath cause 2 (`results` keeps max, so a seed
  score swallowed edge evidence): `link_entities` scored a deliberate link by
  `cosine(from, to)` — weakest exactly where the edge is the only evidence.
  Deliberate links now carry asserted confidence (user 1.0 / agent 0.95); auto
  similarity edges keep their measured cosine. The item's stated bar cited
  `recall@1` 0.9583, stale — that figure predates the answer-leg removal, whose
  own closure records the master baseline this was judged against: 0.9167 /
  0.9722 / 0.9462. Landed: all 8 pairs in the top 5
  (`test_a_reason_edge_makes_its_neighbour_reachable`, xfail marker removed),
  exact-match probe still rank 1, and recall *improved*: 0.9306 / 0.9722 /
  0.9471. Weighting variants swept (source² and edge-reliability² damping):
  linear source weighting was the only one that beat baseline.
- **The answer leg is deleted** — closes items 63, 68, and 80 (2026-07-21).
  Synthesis, HyDE, LLM rerank, and the query cache are gone; `query` returns
  passages/edges/chains and the calling agent synthesizes. Retrieval is
  LLM-free end to end, which is exactly the path item 1's instrument scores;
  e2e floors held unchanged through the removal (recall@1 0.9167, MRR 0.9462).

Each of these was listed as open at some point and is not. Kept as one line so a
future audit does not resurrect it; the proof is the citation.

Numbers are stable identifiers, retired on close — a closed item leaves its
number behind rather than compacting the list, because items cite each other by
number ("blocked on item 13") and renumbering would silently repoint them.

- **`(belief, uncertainty)` is already on `query`** — was item 23, closed on
  inspection 2026-07-21 rather than by building anything. The item said
  `conf_variance` is "surfaced nowhere"; every result payload carries both terms
  — `conf` (the beta mean) and `conf_uncertainty` (the variance) — in
  `base_entity_json` for the ranked list and in `entity_detail` for the id path
  (`src/mcp/tools_query.rs`), and has since the initial commit. Nothing was
  scheduled against a gap that never existed; what remains is item 65, which is
  about *ranking* on the lower bound, not surfacing it.
- **In-memory drops are documented and counted** — was item 5. Spill-before-drop
  is a guarantee of a persisted kern; with no store bound there is nowhere to spill
  to, so the victim is dropped outright. Intended, now stated in `README.md` and
  carried by `unspilled_drops` on all three health surfaces, so an in-memory
  deployment cannot read as a durable one.
- **Every fail-open path is counted** — was item 7. Dead embed endpoint, clock
  skew stalling GC, the `min_deliver_score` bypass and the 50k remote ceiling each
  carry a counter, a throttled log and a health field on MCP, RPC and `kern health`
  (CHANGELOG 2026-07-21). Fail-open is still the behaviour; it is no longer silent.
- **`commit_access` is rate-limited** — was item 16. Retrieval stamps every
  delivered result, so replaying one query pumped a thought's count and heat for
  free. One reinforcement per thought per minute; genuine reuse across a session
  still counts. The local twin of item 13's exposure, and the one that needed no
  peer.
- **An unauthenticated peer cannot reach a local row** — were items 13, 14 and 15.
  `ValidUntil`/`ReasonScore` LWW deltas confined to `remote-*` (the G-Counters keep
  their reach by design); entity bodies hash-checked against their claimed id;
  `handle_pulse` rejects unknown kern ids, confines deposits to `remote-*` and
  clamps strength. Item 13 was the hard edge that gated item 17.
- **`valid_until` is enforced on the default recall path** — was item 17.
  `drop_expired` runs on every retrieve, skipped when the query names its own
  instant so point-in-time history stays queryable. Unblocks item 22.
- **The retrieval instrument exists** — was item 1. `e2e/` scored by
  `recall@1`/`recall@5`/`MRR` over a test-authored corpus with no LLM in the
  scoring loop: 0.9583 / 1.0000 / 0.9792, reproducible bit-for-bit. Floors make it
  a regression detector, not a quality claim; the bag-of-words fake embedder means
  it measures kern's machinery, not a real model's semantics. Both limits recorded
  (CHANGELOG 2026-07-21). Its first finding is item 86.
- **CI runs the lint gate and the e2e suite** — were items 71 and 72. `just check`
  and `just e2e` as jobs in `.github/workflows/ci.yml`. The banned-vocabulary step
  beside them could never fail and was fixed in the same change.
- **`.pi/update.sh` ships** — was item 73. It was gitignored, so the fresh-checkout
  guarantee did not exist; now the single tracked exception to `/.pi/*`, running
  `just docs-install` and `just e2e-install`.
- **A panicking tick task no longer kills maintenance** — was item 2. `catch_unwind`
  in `tick::start`, counted and surfaced on MCP, RPC and `kern health`. The GNN
  forward/backward chain is fallible end to end, so a failed propagation persists
  nothing (CHANGELOG 2026-07-21).
- **The embedding-model swap is caught** — was item 3. Model and dimension stamped
  on flush, checked at open, guarded fail-open on the query path.
- **`as_of` no longer lies over the cold tier** — was item 4. The temporal triple
  round-trips through `cold_spill`.
- **Cold evictions are counted and surfaced** — was item 6. The 50k FIFO bound
  stays; it is no longer invisible.
- **The detached daemon logs** — was item 10. Per-arg, owner-only, append-only
  `<data_dir>/logs/{hub,daemon}.log` via `src/config/detached_log.rs`, on both the
  hub-first path (`src/hub/node.rs`) and the direct `spawn_daemon` fallback
  (CHANGELOG 2026-07-21).
- **An invalid config stops startup** — was item 11. Exit 78 (`EX_CONFIG`) for
  both an unparseable file and a failed `validate()`; an absent config still
  defaults silently (`src/main.rs`). Still owed and tracked in item 84:
  `serve.mcp_addr` has no reader, and `src/commands/admin.rs` still defaults on a
  foreign root's config error.
- **Config scopes deep-merge per key** — was item 12. `merge_deep`
  (`src/config/io.rs`), arrays as leaves. A scope that redirects a section's `url`
  does not inherit that section's `key` (`src/config/secrets.rs`) — the tradeoff
  the merge decision failed to name up front (CHANGELOG 2026-07-21).
- **`docs_check.py` scans all four documentation directories** — was item 74.
  `docs/site/content/`, `docs/kern/`, `docs/oracle/` and `README.md`, 876
  references, with a `<!-- docs-check: historical -->` page marker and a
  same-line deletion escape so a record of what was removed is not forced to
  resolve. The item's two named casualties were already fixed before it was
  scheduled; the three it actually caught were live lies in `FEATURES.md` and
  `SPECIALISTS.md` (CHANGELOG 2026-07-21).
- **Pulse and Question senders are live.** `broadcast_pulse` / `broadcast_q` built
  in `start_gossip` (`src/commands.rs:966-1040`), pulse wired into the maintenance
  tick (`:709`) and the `pulse` MCP tool (`src/mcp/tools_admin.rs:218`),
  `broadcast_q` invoked by `do_resolve` (`src/tick/tasks.rs:372`), `handle_question`
  live-dispatched (`src/gossip/handler.rs:44`).
- **`Fetch` is wired** — `wire_fetch` installs the handler at
  `src/commands.rs:1003`. Single-id, so it is not anti-entropy (item 36), but it
  is not dead.
- **`union_statements` never existed**; remote heat is no longer pinnable
  (`src/base/merge.rs:20`, applied `:153`).
- **The query cache already matches paraphrases.** `QueryCache::lookup` keyed on
  cosine ≥ `theta` (0.97) against the stored query vector; `lookup_text` was a
  pre-embed fast path, not the only key. Historical: the cache itself was
  deleted with the answer leg (2026-07-21).
- **There are no HNSW tombstones.** See item 27.
- **`Kind` cannot be claimed at the wire boundary**, and the premise that it
  could was wrong on all three counts. Stronger than when this was written: the
  `ingest` schema no longer takes a `kind` at all (`src/mcp/tools_mutate.rs`),
  so there is nothing left for the since-deleted `validate_kind` to reject;
  `Superseded` is an `EntityStatus`, not an `EntityKind`
  (`src/base/types.rs:19-28`), so it was never claimable; and a forged `Fact` is
  unreachable regardless, since the MCP path runs `clamp_confidence(p.conf,
  AGENT_SOURCE)` capping at `MAX_AI_CONFIDENCE` 0.95 and `kind` is *derived* from
  confidence, which needs 1.0 for `Fact` (`src/base/math.rs:205-210`,
  `src/base/constants.rs:69`). Only the CLI reaches `Fact`, via
  `clamp_confidence(1.0, "user")` (`src/commands/ingest_cmd.rs:49`).
- **`conf` is clamped to [0,1]** — `validate_conf` (`src/base/validate.rs:14`)
  called at `src/mcp/tools_mutate.rs:113`.
- **A prose-answering reason model no longer archives deltas having stored
  nothing.** `parse_claims` returns `Option`; a reply with no parseable JSON array
  leaves the delta queued for retry, a well-formed empty array still archives
  (`src/ingest/distill.rs`, guarded by `prose_reply_carrying_knowledge_is_not_lost`).
  The retry-forever tradeoff it introduces is surfaced by item 8.
- **The intake accepts anything readable as text**, routing by what the file is:
  `.txt` is a transcript and is distilled, everything else is a `Document` stored
  whole via the watcher's path, binary is quarantined into `failed/`.
- **One vocabulary** — `IntakeConfig`, `[intake]`, `.kern/intake/`,
  `spawn_intake`, `kern.intake`, with the legacy directory self-migrating on
  first start.
- **Automatic session capture is a non-goal**, closed by decision rather than by
  building a producer. Two caller-driven entries exist — MCP `ingest` (primary)
  and a drop into `.kern/intake/` (backup) — and kern ships no session hook.
- **Filtered ANN end-to-end** (all three seed sources on `is_active`); RRF at the
  answer layer; the `answer:false` sub-ms no-LLM path; the semantic query cache;
  the lock-scoped answer path. The A/B that accompanied this shipped a recall
  number from the deleted bench; the change stands, the number is withdrawn.
- **Durability: `snapshot_if_dirty` on the maintenance tick.** WAL rejected —
  LMDB already orders recovery. (Item 75 is the DiskANN path, which this does not
  cover.)
- **Hub phases 1, 2 and 4.** The supervisor (resolve / spawn / adopt / unload,
  hub-first `kern mcp`, graceful shutdown via `KernRpc::shutdown`); idle
  lifecycle (`HealthRes.idle_ms` stamped in the MCP dispatch core and every typed
  RPC method, reaper double-checking under the root lock, `--idle-unload-secs`
  default 1800, 0 disables, hub-owned nodes only); `kern hub merge <src> <dst>`
  offline CRDT union via `absorb_graph` with both daemons stopped and src never
  written; and hub auto-start (`[hub] auto_start = false` opts out, `kern hub
  stop` ends it over RPC). Cross-kern navigation beyond resolve/status remains
  future work. Phase 3 is item 47.
- **Chunk external ids are keyed on full source identity** (`source_id()` + chunk
  index, not the bare section); an identity-less source gets an empty external id
  and never supersedes. Item 48 is the remaining *dedup* half.
- **There is one typed transport surface, not two.** The former "kern_rpc +
  search with overlapping DTOs" item was wrong: `trnsprt` exposes `kern_rpc` and
  `hub_rpc` (`src/trnsprt/src/lib.rs:20-21`), and `kern_rpc` is
  `health` / `shutdown` / `call_tool` / `list_tools`
  (`src/trnsprt/src/kern_rpc/svc.rs:5-8`) — a generic envelope with no query DTOs
  to overlap. Nothing to kill. (`KernRpc` mirroring the MCP tool list 1:1 is a
  real and separate concern — item 24.)
