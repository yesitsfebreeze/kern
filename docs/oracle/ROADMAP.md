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

Two headings in this file were destroyed by an editing script that cut from an
item to the next one and swallowed the tier boundary between them; Tier 2 and
Tier 3 are restored above their items. Say it here because a lost heading is
invisible — the items survive and simply appear to rank somewhere they do not.

Context that is not work — north star, competitive position, non-goals, repo
laws, and what is closed — lives after the ranked list.

---

# Tier 1 — live defects on the default path that fail silently

These need no gossip, no flag and no unusual configuration. Every one produces a
wrong or missing result with no error, which is why they outrank both the
security work (armed only with federation on) and every feature.

### 86. A reason edge still cannot lift its neighbour into the results `[retrieval]`

**Measured, and half fixed.** The first finding the new instrument produced
(`e2e/test_invariants.py`, recorded `xfail(strict=True)`): probe with A's own
text, and B — linked A->B, sharing no content words — ranked *identically to four
decimals* with and without the edge, across 8 pairs.

Two independent causes were isolated. The first is fixed; the second is why this
item is still open.

**Cause 1 — the beam threshold compared two different scales. FIXED.**
`expand` pruned a neighbour whose score fell below `global_best * decay`, where
`global_best` was set by the best *seed*: a pure query cosine, up to 1.0. But a
neighbour scores `w.content*cos(q,n) + w.reason*cos(q,edge) + w.edge*edge_score`,
so with the default weights a neighbour the query does **not** match directly has
a ceiling of `0.15 + 0.15 = 0.30` against a threshold of `0.25`. Measured on a
minimal graph: neighbour 0.2411 against threshold 0.2500 — pruned by 0.0089, and
`chains` came back empty. Whenever a seed matched well, which is the common case,
the walk was structurally dead. The threshold is now taken from the best score
seen *among neighbours*, so the first hop off any seed is always explored and the
bar is set by the frontier. Regression test:
`expand::tests::a_strong_seed_no_longer_prunes_the_walk_off_it`.

**Cause 2 — traversal evidence is discarded, not combined. OPEN, and the naive
fix is measured wrong.** `visited` allows exactly one pop per entity and
`results` keeps `max` per entity, so when B is *already* a content hit — 7 of the
8 pairs were — the seed score wins the max and the edge contributes nothing.
Effect of cause 1's fix alone: 1 of 8 pairs moved (rank 22 -> 21). The other 7 are
this.

Pooling the evidence instead of taking the max makes all 8 move, dramatically —
5 of 8 reach the top 5, e.g. rank 14 -> 4 and 7 -> 2. **It also regresses the
primary metric**, and the instrument caught it: the exact-match probe "where does
ada store her bicycle" fell from rank 1 to rank 3 behind two unrelated facts
(`e2e/test_retrieval.py`). The reason is structural and must be designed around,
not tuned away: the best-matching entity pops *first*, so by the time its
neighbours are expanded it is already `visited` and can never *receive* hop
evidence, only give it. Any co-equal pooling therefore systematically penalises
the best answer.

So the open question is a design one: **how does a walk pay without letting a
well-connected node outrank a direct match?** A bounded bonus rather than
co-equal evidence, evidence weighted by hop distance, or giving the seed its own
traversal credit before it is consumed. Whatever is chosen, the bar is now
concrete and cheap to check: all 8 pairs must move, `recall@1` must hold at
0.9583, and `test_query_ranks_the_matching_fact_first` must stay green.

It ranks here because "a graph, not a bag — recall can walk them" is a
`VISION.md` criterion, and the walk still changes almost no outcome.

### 8. `kern intake` — no way to see or drive the intake `[ingest]`

**Half done.** The *why* now exists: a failed drain writes the last error to
`<intake>/errors/<name>.txt`, clears it on the next success, and
`ingest::intake_status::scan` reports pending (with age and last error), failed
and done. What is still missing is the surface — there is no `Intake` variant in
`Commands`, so nothing exposes any of it. Wanted: `kern intake` (list pending +
failed with the last error) and a one-shot drain so the CLI works with no daemon
running.

Raised in rank by `220af94`: a reason model that persistently replies prose now
retries forever rather than losing the delta — the safe side, and deliberately
chosen, but that tradeoff is only acceptable while it is *visible*. This item is
what makes it visible.

### 9. Two live writers against one LMDB environment `[surface]`

Merged from what were two items, because it is one failure with two entrances.
The CLI reads the on-disk graph while the daemon holds newer state, and
`kern mcp`'s standalone fallback opens the same store as a second writer
(`src/commands/mcp_cmd.rs:241`). Partly mitigated since it was written: both go
through `save_graph_guarded`, which refuses a stale flush
(`src/commands.rs:303-306`), so the documented symptom ("tools work but the
graph never grows", `howto/mcp.mdx:56-61`) is now a refused-flush retry rather
than a silent clobber. Two live writers still exist. Needs `kern status` +
advisory locking; no `flock` or `Status` subcommand exists anywhere in `src/`.

`README.md:159-161` still presents the auto-spawn fallback as "all you need to
bring kern up", with no caveat.

Observed live 2026-07-21 during the all-granite reembed of this repo's own
store: `kern reembed` opens the store directly per its "daemon must be
stopped" comment, but that precondition is unenforceable — killing the hub
does not keep it dead, because any surviving `kern mcp` proxy auto-respawns
it (`hub.auto_start` default true), and the respawned hub then flushed its
stale in-memory graph over the completed re-embed, losing the rewrite and
one thought. `reembed` (and any direct-writer admin command) needs the same
advisory lock this item already calls for, or a hub RPC that performs the
re-embed inside the single writer.

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
be returned. Backward compatibility: empty `principals` means *no filter*, not
*public only*, or every existing single-agent caller goes blind.

### 18. ACL + request principal — gates everything else in this tier `[surface]`

`Entity` already carries `Acl` (`src/base/types.rs:268`; struct `{scope, users,
groups}` at `:94-99`), and it is only ever written as `Acl::default()`
(`src/ingest/place.rs:56`, `src/ingest/file_watcher.rs:136`), so nothing can
populate it. Four parts:

- Expose `principals` / `scope` on the MCP `ingest` schema
  (`src/mcp/tools_mutate.rs:19-31`), threaded through `ingest::Job` into
  `place.rs`.
- Accept `principals` on `query` — no identity param exists
  (`QueryArgs`, `src/mcp/tools_query.rs:95-108`).
- Enforce in `matches_filter` (`src/retrieval/score.rs:130-179`), which has no
  ACL predicate.
- **Guard the id path.** `src/mcp/tools_query.rs:119-127` returns
  `find_entity(&g, &p.id)` directly, before `build_query_options` is ever
  called — no filter of any kind runs. Without this guard ACL is decorative.

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
`apply_boosts` has no source-trust prior (`src/retrieval/score.rs:82-94`). Add
`source_trust_user` / `_agent` / `_auto` to `RetrievalConfig`, default all `1.0`
so ranking does not move until configured, and multiply in the boost step —
**post-fusion, not in RRF**, which is rank-based. Independent of 21; can run
parallel after 18.

### 21. Review / draft lifecycle `[surface]`

`ReviewState` on `Entity` (`#[serde(default)]` → old rows decode as
`PendingReview`, fail-safe) + source-level review policy in config + an
`exclude_pending` query filter and a `promote` tool. Lets a host hold
auto-distilled claims out of retrieval until a human curates them. No
`ReviewState`, `exclude_pending` or `promote` exists in `src/`. Requires 18's
`QueryOptions` work first — review filters are more `matches_filter` predicates.

### 22. Per-source TTL `[ingest]`

An ingest-time `retention` duration setting `valid_until`. Nearly free — one
param plus one timestamp — and the bi-temporal expiry path now enforces it on
every retrieve, so the setting has a reader the moment it has a writer
(unblocked 2026-07-21; `drop_expired`, `src/retrieval/score.rs`).

### 23. Surface `(belief, uncertainty)` on `query` `[surface]`

`conf_variance` is computed (`FEATURES.md:47-49`) and surfaced nowhere. A host
mounting kern as a reasoning store cannot tell a well-evidenced claim from a
single-observation one. Adopted on paper at `docs/kern/bayesian-belief.md:149-151`.

### 24. RPC socket has no auth, and `KernRpc` mirrors MCP 1:1 `[surface]`

`FEATURES.md:477-478`. The mirroring is a drift risk against repo law 3, and the
missing auth is the same boundary as 18's caller-asserted principals — decide
them together or the principal stops at the MCP surface only.

---

# Tier 4 — scaling cliffs

None of these is wrong today. Each converts "works on my corpus" into "does not
work at 10×", and none is measurable-as-fixed until item 1 exists (except by
latency, which the e2e harness can still claim).

### 25. O(N) importance scan per retrieve `[retrieval]`

`seed_important` iterates `g.all()` × `kern.entities.values()`
(`src/retrieval/seed.rs:138-171`), called unconditionally once per retrieve
(`src/retrieval/query.rs`, in `retrieve_profiled`). Rayon-parallel, but still full-corpus per
query. Top structural debt in the repo.

### 26. PageRank runs a full power iteration per query, persisted nowhere `[retrieval]`

Up to 25 iterations over the whole entity adjacency on every retrieve, with
nothing cached between queries (`decisions/pagerank-authority.mdx:102-105`). The
second query-time cliff, and it was recorded on the site but in no plan.

### 27. The GC sweep is superlinear in three separate places `[lifecycle]`

One item because one sweep pays all four costs. **Two are closed**; the two that
remain are the scans, not the accumulation:

- Victim selection is O(entities) per kern per sweep (`src/tick/stigmergy.rs:51-56`).
- The cold tier is a brute-force cosine scan with no index — `cold_search` decodes
  and scores every row (`src/base/store.rs:515-529`).
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
for reuse (`src/base/hnsw.rs:118-138`, alloc reuse `:103-116`), guarded by the
test "deleted slots were recycled, arena did not grow" (`:729`). The cost is the
scan, not the accumulation.

### 28. GNN training runs synchronously on the tick `[lifecycle]`

`TaskKind::GnnPropagate => do_gnn_propagate(...)` runs inline in `process_task`
on the single tick loop (`src/tick.rs:66`), stalling large kerns — and, per item
2, taking every other maintenance task down with it if it panics.

### 29. A spilled kern still carries two resident indexes `[retrieval]`

DiskANN spill is entity-index-only: `rebuild_index` hardcodes `gnn_entity_idx`
and `reason_idx` to `VectorBackend::resident(...)` (`src/base/graph.rs:227-228`)
while only `entity_idx` takes the spill branch (`:234-235`). The memory ceiling
is pushed back, not removed. Compounded: `disk_threshold` defaults to
`KERN_CAP_DISABLED` and nothing auto-tunes it
(`decisions/diskann-spill.mdx:131-134`, `src/config/graph.rs:20`), so the
ceiling DiskANN exists to remove is undefended in every default deployment, with
no signal on approach.

### 30. Ingest queue `enqueue` detaches with no backpressure `[ingest]`

`Worker::enqueue` fires `tokio::spawn(async move { tx.send(job).await })` and
returns immediately (`src/ingest/worker.rs:74-77`). The channel bound is 64
(`:43`); the spawn set is unbounded. Distinct from the *tick* queue, which is
bounded at 512 with real backpressure (`FEATURES.md:318-320`) — the two read as
one and are not.

Beside it, and equally unbounded: **the distill leg has no timeout budget and no
queue-depth metric.** "The LLM call is the only unbounded step on the path"
(`concepts/acceptance.mdx:189-192`), and with the answer leg removed (2026-07-21)
the distill leg is now the only LLM on any path — no latency work has landed on it.

### 31. Routing and structural debt in the hot types `[retrieval]`

Recorded in `FEATURES.md` gap blocks, planned nowhere:

- Routing does a vector lookup per level, O(depth·log n), and unnamed children
  are unbounded per parent (`FEATURES.md:111-113`).
- `Entity` is a ~30-field flat struct (serialization cost on every store round
  trip) and `Kern` carries no per-kern stats — mean heat, fill ratio — that
  clustering could reuse (`FEATURES.md:77-79`).
- DiskANN is build-once; the lexical index is RAM-only (`FEATURES.md:210-211`).
- LMDB compaction is manual and offline-only, and is the only way to shrink the
  high-water mark (`FEATURES.md:265-266`).

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
statements. Merge never imports them (`src/base/merge.rs:112`) and the wire
target is rejected on receipt (`src/gossip/handler.rs:448`), kept as a refused
variant so an older peer cannot inject text under a content-addressed id.

### 33. Transport security `[federation]`

Raw TCP, no TLS. `network_id` broadcast cleartext over UDP multicast. No
signature on `GossipMessage` — it carries `kind` / `id` / `origin` / `payload`
only (`src/gossip/types.rs:17-21`) — and `handle_conn` accepts any stream while
`handle_peer_exchange` trusts any `msg.origin`. Needs `tokio-rustls` + `rcgen` as
direct deps; neither is in `Cargo.toml`. **This one gates any deployment off a
trusted LAN / WireGuard mesh**, and it gates the counter-slot identity half of
item 13.

### 34. The `Question` path is an unauthenticated membership oracle `[federation]`

A peer sends `Question` messages carrying arbitrary embedding vectors and gets a
yes/no on whether you hold a fact above cosine 0.80, with no rate limit
(`concepts/security.mdx:212-215`). Documented on the site, in no plan. Content
existence is extractable one probe at a time without ever receiving the content.

### 35. Namespace rotation is unbounded storage `[federation]`

`network_id` / `kern_id` are attacker-chosen, and the quarantine cap is global
per remote kern, not per peer (`concepts/security.mdx:243-246`,
`decisions/knowledge-not-gradients.mdx:113-114`). One host cycling identifiers
creates unlimited `remote-*` kerns, each with its own 50k allowance. Item 39's
Sybil work covers ranking; this is disk.

### 36. Anti-entropy `[federation]`

No `AntiEntropy` variant in `GossipKind` (`src/gossip/types.rs:6-14`). The sender
sorts by heat and truncates to 32 per heartbeat (`src/gossip/handler.rs:167-169`),
so cold entities may never propagate and a partitioned node that rejoins never
catches up. (`Fetch` is live — `wire_fetch` installs the handler at
`src/commands.rs:894` and the question path issues it — but it is single-id, not a
catch-up mechanism.) Two pieces adopted on paper and unscheduled: **back-off
pacing** with exponential jitter keyed to a divergence estimate
(`docs/kern/fl-vs-knids-federation.md:163-168`), and **batch-size / push-vs-pull
tuning** at scale (`howto/memory-bank.mdx:149-150`) — the top-32 is hard-coded and
the push-only choice was never revisited.

### 37. Backpressure, divergence metric, and delta write-lock starvation `[federation]`

No per-peer rate limit anywhere; `HealthStats` has no divergence field
(`src/base/health.rs:4-10`). Sharper than previously recorded: the four
`for kern_id in g.all_ids()` loops in `handle_crdt_delta`
(`src/gossip/handler.rs:378, 394, 407, 428`) run under the graph **write** lock,
once per inbound delta, unlimited — a cheap remote write-lock-starvation vector
independent of the local-row mutation in item 13.

~~Beside it: `start_entity_sync` clones the entire local corpus every
heartbeat~~ **Closed 2026-07-21.** `hottest_local` selects over references and
deep-clones only the winners — linear, with the same comparator and therefore the
same chosen set. The rest of this item (per-peer rate limits, a divergence field
on `HealthStats`, and the write-lock starvation from the four `all_ids()` loops in
`handle_crdt_delta`) is still open.

(Remote heat is no longer pinnable: entry to a `remote-*` kern strips heat,
access counts and confidence to neutral — `src/base/merge.rs:20`, applied `:139`.
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
(mitigate with `serde(default)`); `AntiEntropy` is additive. Confidence isolation
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
(`src/base/types.rs:291-296`), so each node re-derives its own `as_of` view and
two *converged* nodes can answer the same point-in-time query differently
(`docs/kern/crdts-federation.md:54-62`). The federated twin of item 4.

### 45. Multicast discovery is unreliable with no health signal `[federation]`

Wireless APs, container bridges and VPN interfaces all break it, with no
fallback and no way to distinguish discovery-failed from no-peers-present
(`concepts/federation.mdx:68-70`).

### 46. One fresh TCP connection per gossip message `[federation]`

`TcpStream::connect` per call at `src/gossip/transport.rs:37` (`send_msg`) and
`:45` (`send_and_receive`). No pooling. Separately, the `trnsprt` client has no
pooling either (`FEATURES.md:637-638`) — that one is not gossip and is not gated
on 33.

### 47. Hub phase 3: gossip moves hub-side `[hub]`

One UDP endpoint and one node identity per machine; nodes stop binding the
network entirely (`src/config/gossip.rs:7-16` today). **Ordering decided
2026-07-20:** the senders and semantics build per-node first; this transport move
ships together with item 33 — same wire layer, migrate once. Not blocked,
sequenced. One clause of the previous version was wrong: there is **no** per-project
port-clash validation in `src/config/serve.rs` to collapse — that file is
`mcp_token` handling only (`:1-78`).

Beside it: **hub↔node version skew is unmanaged** beyond same-binary spawning
(`FEATURES.md:677-678`).

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
per-kind (`FEATURES.md:305-306`).

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

### 51. Require reason text on supersede `[ingest]`

`ReasonKind::Supersedes` edges are minted at `src/base/accept.rs:438` and `:533`
with `fallback_label()` text (`src/base/types.rs:80`), never a caller-supplied
rationale. The *why* is the thing the graph exists to hold.

### 52. Document gravitons truncate at the embed context window `[ingest]`

Acknowledged in source at `src/mcp/tools_admin.rs:116`. Chunk + mean-pool is the
upgrade path, blocked on a real document long enough to truncate. The guidance
half is being fixed on the docs side now; the truncation stays open.

### 53. Clustering is vector-only `[lifecycle]`

No semantic or structural features (`FEATURES.md:358-360`), and naming plus
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
`src/config/retrieval.rs:32/90`) and a 7-day one for retention
(`src/base/heat.rs:18`). The offline NDCG sweep meant to tune either was never
run (`decisions/stigmergy-over-gardening.mdx:117`). Third input nobody
reconciled: `docs/kern/stigmergy-self-improving.md:160-170` derives a 1–2 day
half-life and the shipped value is 7 days. Now measurable: `e2e/test_recall.py`
scores a half-life change directly (`recall@1`/`recall@5`/`MRR`), which is the
sweep that was never run.

---

# Tier 7 — belief model

`decisions/bayesian-confidence.mdx` and `decisions/edit-convergence.mdx`. None
funded before now. Ranked here because the model is coherent and merely
incomplete — no item below produces a wrong answer today.

### 56. An agent cannot register disagreement at all `[ingest]`

There is no `Contradicts` reason kind (`src/base/types.rs:66-75`) and no `stance`
parameter on the ingest schema (`src/mcp/tools_mutate.rs:19-31`);
`observe_contradict` (`src/base/types.rs:400`) has exactly one caller, GNN
alignment (`src/tick/gnn_propagate.rs:144`). Observer-reputation weighting is
also unbuilt.

### 57. No evidence decay `[lifecycle]`

`conf_alpha` and `conf_beta` only grow — the sole zeroing is the remote strip
(`src/base/merge.rs:25-26`) — so stale consensus takes proportionally many new
observations to unseat. Tick-based γ damping is an open design
(`decisions/bayesian-confidence.mdx:137`).

### 58. Supersede chains are unbounded while contested `[lifecycle]`

No `ReasonKind::Edit` rationale edge (`src/base/types.rs:66-75`) and no producer
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
tool exposes the supersede chain beyond `include_history` (`FEATURES.md:143-145`).
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

The raw `0.4·content + 0.6·GNN` blend (`src/base/search.rs:25-26`, applied `:39`)
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
(`score * confidence + boost + fact_bonus`, `src/retrieval/score.rs:82-94`); swap
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
never auto-tuned (`FEATURES.md:180`).

### 67. Binary quantization stays non-user-selectable `[retrieval]`

Its recall floor is too low without a rescoring pass; deliberately excluded from
`parse` (`src/quant.rs:20-21`). Beside it: no int4 path and the quantization
scale is fixed at encode time (`FEATURES.md:229`).

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

Called twice, both with the literal `AGENT_SOURCE`
(`src/mcp/tools_mutate.rs:119`, `:123`), and it accepts `USER_SOURCE` /
`AGENT_SOURCE` (`src/base/validate.rs:33-34`), so it can never fail. Decision:
thread a real auth identity (item 18/24), or delete. Delete is correct for a
single local daemon and needs only sign-off.

### 81. `resources/list` and `prompts/list` return `-32601` on the proxy path `[surface]`

`ProxyServer` implements `tools_list` / `call_tool` / `extra_capabilities` only
(`src/commands/mcp_cmd.rs:194-239`) with no `handle_method` override, so the trait
default returns `None` (`src/trnsprt/src/server.rs:21`). Meanwhile
`extra_capabilities` advertises `{"resources": {}, "prompts": {}}` (`:238`) to
match standalone, which *does* serve them (`src/mcp.rs:176-186`). Advertised on
the normal path, non-functional there. Either forward them or stop advertising.

### 82. Standalone `kern mcp` runs no gossip `[surface]`

**Corrected:** the previous version said "no maintenance tick and no gossip". The
tick *is* started (`src/commands/mcp_cmd.rs:293-304`); only gossip is absent
(`broadcast_q: None` at `:298`, `broadcast_pulse: None` at `:319`). A graph
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
  (`FEATURES.md:460-462`).
- The LLM client is Ollama-centric with no retry/backoff policy object
  (`FEATURES.md:598-600`).
- Watcher `.gitignore` parsing is approximate; no rename tracking
  (`FEATURES.md:695-696`).
- `unnamed promote` is manual (`FEATURES.md:510`).
- GNN has no GPU path, weights are per-kern rather than shared, and the objective
  is link-prediction only (`FEATURES.md:425-427`).
- Under WSL2 NAT a loopback Ollama URL must be hand-pinned; kern neither rewrites
  nor warns (`FEATURES.md:710-712`).
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
  `howto/mcp.mdx:73`.
- The `move` MCP tool exists in source (`src/mcp/tools_mutate.rs:70`) and appears
  in neither `README.md`'s tool table nor `FEATURES.md:436`. The site is correct
  ("Eleven tools", `howto/mcp.mdx:65`).
- `docs/kern/README.md:47-48` declares the directory holds "never plans"; five of
  its notes contain execution plans, migration stages and phase orderings
  (`diskann-disk-index.md:30, :86-105`, `crdts-federation.md:215-255`,
  `fl-vs-knids-federation.md:248-254`, `stigmergy-self-improving.md:298-301`,
  `pagerank-authority.md:279-280`). Either the notes lose their plans to this
  file, or the declaration is false. Repo law 4 says the former.
- `docs/kern/` research notes carry four stale claims against current source:
  `crdts-federation.md:6-7` (GCounter/PnCounter — no `PnCounter` exists),
  `:11-13` (Delta has no live sender — it does), `:13-14, :276-281` (OR-Set for
  statements is "not built" — it was *reversed*), and
  `diskann-disk-index.md:28-29` (PQ is "the next step" — it is a non-goal below).
- Surviving quality claims that item 1's standard forbids: `FEATURES.md:166`
  ("recall/NDCG unchanged", from the deleted bench — the `+7% p50` half is
  latency and stays), `docs/kern/diskann-disk-index.md:25` ("recall@10 ≥ 0.90 vs
  brute force"), and this file's own former citation of a "recall@10 A/B" as
  evidence, now struck.
- `README.md` and `VISION.md` still open by saying kern "takes in durable facts
  from your sessions" and "learns on its own", contradicting the recorded
  non-goal that kern captures nothing on its own. `VISION.md:13, :47-48` still
  gate claims on "the recorded baseline", which item 1 says does not exist.
  `README.md:344-346` still says the Delta/Question/Pulse senders are dead (they
  are live) and pins the version at 1.0.0 against `FEATURES.md`'s 1.1.0.
- `FEATURES.md` omits `Entity.acl` from its field list while claiming to scrape
  "everything that actually exists", still carries the retired query-cache
  finding as a live opportunity (`:843-844`), and marks MCP prompts/resources
  `active` with no note of item 81.

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

Claim standard, until a replacement exists: **no quality claim of any kind.** Not
SOTA, not parity, not regression, not improvement. Latency claims remain
permitted from the e2e harness. Item 1 is the open question; nothing below it can
be scheduled honestly until it is decided.

## How we supersede Zep / Mem0 / Letta / Qdrant

Not by matching feature lists. By owning a combination none of them hold, then
proving it — on a measurement that does not yet exist (item 1).

| property | kern | Zep/Graphiti | Mem0 | Letta | Qdrant |
|---|---|---|---|---|---|
| Per-project self-maintaining graph (per-cwd) | ✅ | ❌ hosted | ❌ | ❌ | ❌ |
| Default recall touches no LLM (sub-ms) | ✅ | ❌ | ❌ | ❌ | n/a |
| Local-first, single binary, no network hop | ✅ | ❌ | ❌ | partial | ❌ |
| Self-forgetting (decay / stigmergy GC / cold spill) | 🟡 items 2, 5, 6 | ❌ | partial | ❌ | ❌ |
| Graph + dense ANN + BM25 + GNN in one process | ✅ | partial | ❌ | ❌ | ❌ |
| Bi-temporal supersede off the recall path | 🟡 items 4, 17 | ✅ | ❌ | ❌ | ❌ |
| Coordinator-free CRDT federation | 🟡 building | ❌ | ❌ | ❌ | ❌ |
| Published eval numbers | ❌ withdrawn | ✅ | ✅ | ✅ | n/a |

Two rows carried an unqualified ✅ in the previous version while this same file
funded the defects that break them. They are 🟡 with the items named — a
scoreboard that disagrees with its own plan is not a scoreboard.

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
3. **One dispatch core.** Every surface goes through `tools::dispatch`, never a
   second copy.
4. **This file is the only plan.** New work goes here, not into a new document.

## Closed and verified — do not re-open

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
  round-trips through `cold_spill`; legacy rows still decode.
- **Cold evictions are counted and surfaced** — was item 6. The 50k FIFO bound
  stays; it is no longer invisible.
- **The detached daemon logs** — was item 10. Per-arg, owner-only, append-only
  `<data_dir>/logs/{hub,daemon}.log` via `src/config/detached_log.rs`, on both the
  hub-first path (`src/hub/node.rs`) and the legacy `spawn_daemon` fallback
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
  in `start_gossip` (`src/commands.rs:900-930`), pulse wired into the maintenance
  tick (`:658`) and the `pulse` MCP tool (`src/mcp/tools_admin.rs:226`),
  `broadcast_q` invoked by `do_resolve` (`src/tick.rs:64`), `handle_question`
  live-dispatched (`src/gossip/handler.rs:41`).
- **`Fetch` is wired** — `wire_fetch` installs the handler at
  `src/commands.rs:894`. Single-id, so it is not anti-entropy (item 36), but it
  is not dead.
- **`union_statements` never existed**; remote heat is no longer pinnable
  (`src/base/merge.rs:20`, applied `:139`).
- **The query cache already matches paraphrases.** `QueryCache::lookup` keys on
  cosine ≥ `theta` (0.97) against the stored query vector
  (`src/retrieval/cache.rs:60-71`); `lookup_text` (`:48`) is a pre-embed fast
  path, not the only key.
- **There are no HNSW tombstones.** See item 27.
- **`Kind` is validated at the wire boundary**, and the premise that it was not
  was wrong on all three counts: `validate_kind` is called
  (`src/mcp/tools_mutate.rs:117`) and rejects the four internal-only kinds;
  `Superseded` is an `EntityStatus`, not an `EntityKind`
  (`src/base/types.rs:19-28`), so it was never claimable; and a forged `Fact` is
  unreachable regardless, since the MCP path runs `clamp_confidence(p.conf,
  AGENT_SOURCE)` capping at `MAX_AI_CONFIDENCE` 0.95 and `kind` is *derived* from
  confidence, which needs 1.0 for `Fact` (`src/base/math.rs:205-210`,
  `src/base/constants.rs:62`). Only the CLI reaches `Fact`, via
  `clamp_confidence(1.0, "user")` (`src/commands/ingest_cmd.rs:47`).
- **`conf` is clamped to [0,1]** — `validate_conf` (`src/base/validate.rs:18`)
  called at `src/mcp/tools_mutate.rs:115`.
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
