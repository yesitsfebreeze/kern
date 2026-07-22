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

### 100. `kern health` reports the serving daemon's degradations — closed 2026-07-22 `[surface]`

**Filed and closed 2026-07-22, found while reading item 28's close.** Eight of
the numbers `kern health` prints are scoped to the process that reads them: the
seven fail-open counters summed into `degraded:` are `AtomicU64` statics —
`query_dim_rejected` (`src/base/search.rs:10`), `below_floor_deliveries`
(`src/retrieval/score.rs:145`), `clock_skew_skips` and `unspilled_drops`
(`src/tick/stigmergy.rs:16`), `ingest_queue_refused` (`src/ingest/worker.rs:81`),
`ingest_dropped_chunks` (`src/ingest/worker.rs:311`), `remote_cap_dropped`
(`src/base/merge.rs:140`) — and `evicted:` reads `Store::cold_evicted`
(`src/base/store.rs:306`), an instance field every `Store::open` zeroes
(`src/base/store.rs:351`). `cmd_health` built all eight from
`graph_health_stats` over a graph *this CLI process* had just opened, and
`load_graph` (`src/commands.rs:281`) runs no search, no scoring, no tick, no
ingest and no merge, so the `if degraded > 0` branch was **unreachable from the
CLI** and `evicted:` was **always 0** however degraded the daemon was. Not
stale-and-sometimes-wrong: structurally zero.

Closed by preferring the wire. `cmd_health` awaits `daemon_health` once, up
front (`src/commands/admin.rs:43`), and hands the response to a new
`degradation_lines` (`:116`) beside `tick_health_lines` (`:169`). `HealthRes`
already carried all eight — `cold_evicted` (`src/trnsprt/src/kern_rpc/dto.rs:42`)
and the seven at `:55`–`:67`, filled from the daemon's own `health_stats`
(`src/rpc/kern_rpc_server.rs:104`) — and all eight are now taken **whole**
(`src/commands/admin.rs:120`), not merged: a daemon is serving the store, so its
counts are the true ones and this process's are noise about a graph nobody
queried. The local read stands only when nothing answers (`:132`), where zero is
honest. The no-daemon output is byte-identical to before, empty graph and
populated.

**The formatter is pure, so the tests are runner-independent.** These are
process statics, so a test asserting on `ingest_queue_refused()` passes under
`cargo nextest` (a fork per test) and fails under `cargo test --workspace
--locked` (one process, which is what CI runs) the moment another test
increments them — the trap item 92 hit. `degradation_lines` reads neither the
statics nor the store, only its two arguments, and its tests
(`src/commands/admin.rs:193`) construct both. The one that carries the decision
is `a_local_count_is_not_printed_over_a_serving_daemons` (`:221`) — local
counters nonzero, daemon healthy — which a `max()` or any additive merge fails.
End to end, `e2e/test_health_surface.py` blinds the CLI's `data_dir` after the
daemon has opened its own, drains three claims the fake LLM refuses to embed
with a 400, and asserts `kern health` prints exactly 3: a count a blinded
process cannot hold and a constant cannot match.

Deciding behavior: verify-before-claiming — a health surface that cannot be
wrong is worth more than another counter on one that can.

### 9. Live writers: `ingest`/`link` still write locally `[surface]`

**Decided 2026-07-21, and implemented for every command the route fits.** The
choice named here was between routing the one-shot writes through
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
putting a trust field on the socket. **Restated 2026-07-22:** that used to read
"an unauthenticated socket"; the socket now authenticates, and the block did not
lift, because the part this half needs was never the missing auth. A shared
secret proves a uid, and the CLI and the agent's proxy are the same uid — so a
`principal` on that frame is still *declared*, and a trust field riding a
declared principal is the same escalation path it always was. Blocked on item
24's residue, not on effort.

That block is a new sequencing edge pointing *down* the file — item 24 sits in
tier 3 and this sits in tier 1 — and the list was not reordered for it. Item 24
does not move up because the trust field is one caller of it, not its severity —
and now that its gate is built, what remains of it is smaller than this half is.

**Closed 2026-07-21: `graviton add`/`remove` and `claim-kind add`/`rm`.** These
were the four shipped subcommands that reached `with_graph`
(`src/commands.rs:462` — load, mutate, `save_graph_unguarded`) with no routing at
all, so beside a running daemon each one wrote the WHOLE kern map back over
everything that daemon had committed since the CLI loaded. They needed no new
tool: the daemon already exposes `graviton` and `claim_kind`
(`src/mcp/tools_admin.rs`) with matching semantics. `graviton_at` and
`claim_kind_at` (`src/commands/admin.rs`) now route first and keep `with_graph`
as the `NoDaemon` fallback, printing through the same four printers so the two
paths cannot drift. The graviton add routes *before* it embeds — the daemon owns
the vector it stores, so embedding locally would be a second call to the same
model for nothing. Not blocked on item 24: neither command asserts trust or mints
a Fact, unlike `ingest`/`link`. Guarded by
`e2e/test_graviton_routing.py`, which blinds the CLI's `data_dir` after the
daemon has opened its store and then makes the daemon flush again — the exact
sequence in which the graviton used to disappear.

**Closed 2026-07-21: `intake drain`.** It had no tool to route to, so it got one
— `intake_drain` (`src/mcp/tools_intake.rs`), one immediate pass of
`ingest::intake::drain_now` inside the daemon, returning `archived`. `drain`
(`src/commands/intake_cmd.rs`) routes first and keeps its in-process pass as
`drain_locally`, the `NoDaemon` fallback; both paths print through the same tail,
so only the archived count crosses the socket. This was a real race, not just
staleness: `drain_once` reads the queue directory and archives each entry, so a
CLI drain beside the daemon's poll loop distilled the same transcript twice and
raced the archive move. **The tradeoff, taken:** it puts a mutation on the
unauthenticated RPC socket. `gc` and `pulse` are already there, and `drain`
carries no caller-supplied content and no trust claim, so unlike `ingest`/`link`
it does not widen item 24's hole in a new way.

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

`ingest` and `link` (blocked on item 24) still write the store directly;
`intake drain` joined the route 2026-07-21 once it was given a tool to route to.
Both are one-shot, so the exposure is a lost write rather than a lost graph —
and both direct-write paths are guarded. `cmd_ingest`
(`src/commands/ingest_cmd.rs`) and `intake drain`'s `flush`
(`src/commands/intake_cmd.rs`) retry through `persist::flush_guarded`;
`cmd_link` joined them 2026-07-21 via `link_and_persist`
(`src/commands/graph_ops.rs`) and `save_graph_guarded`, so a daemon committing
underneath any of them gets a refused flush and a reload rather than a clobber.

**Corrected 2026-07-21 — "one half remains" was wrong, and so was "the remaining
two unlocked callers".** The unguarded entry point is named
`save_graph_unguarded` and carries its precondition, and walking its call sites
turns up **three** classes, not two. The two this item already named stand and
still do not belong to it: `cmd_hub_merge` (`src/commands/admin.rs:915`) writes
a destination graph it holds no lock on, and `maybe_self_heal_store`
(`src/commands.rs:433`) rewrites the store during boot recovery — hub merge
stops both daemons first, self-heal runs before the daemon serves, and neither
has been proven safe.

The third class *was* this item's unblocked half, and it is the one the closure
above discharged. `with_graph` (`src/commands.rs:462`) loads the graph, mutates
it and calls `save_graph_unguarded` (`:465`) holding no lock at all. `cmd_forget`
and `cmd_degrade` reach it safely, because they `route` first and only fall into
it on `NoDaemon` (`src/commands/graph_ops.rs:120`, `:443`). `kern graviton
add`/`remove` and `kern claim-kind add`/`rm` did not route at all, so beside a
running daemon they loaded a snapshot, mutated it, and wrote the *whole graph*
back over whatever the daemon had committed since — a full-graph clobber, not a
lost write, and the exact race the writer lock and the route exist to close.
**Corrected 2026-07-21 — this paragraph said "do not route at all" one commit
after that stopped being true.** All four now route first
(`graviton_at`, `src/commands/admin.rs:358`, route calls `:374` and `:420`;
`claim_kind_at`, `:480`, route calls `:488` and `:506`), keeping `with_graph`
as the `NoDaemon` fallback. Unlike `ingest` and `link` they assert no trust:
`graviton` carries a name, seed text and a mass; `claim_kind` a name and a
description. Neither mints a Fact, so routing them widened item 24's hole no
more than `intake drain` did, and the tools they route to already existed with
matching semantics — `graviton` (`src/mcp/tools_admin.rs:87`, schema `:39`) and
`claim_kind` (`:161`, schema `:52`), both dispatched at `src/mcp.rs:184-185`.

So the item's remainder is **one** half, not two: `ingest`/`link`, blocked on
item 24. `graviton`/`claim_kind` were the unblocked half and are closed. The item
does not close on the read side alone, and it cannot close at all until item 24
decides how trust crosses the socket.

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

### 18. Every read surface enforces the ACL, and a watched file is public by decision — closed 2026-07-22 `[surface]`

**Closed 2026-07-22 by deciding, not by building.** A watched file stays public:
`Acl::default()` names no scope, no user and no group, and `Acl::is_public` —
the single definition of public in the tree — is exactly that emptiness. Both
watcher legs pass it, and `drain_direct_once` carries the payload's own ACL
rather than stamping one, so the decision is made in one place instead of two
that happen to agree.

**Tenant-default lost on the same ground item 20's `source_trust_user` did.**
There is no tenant identity anywhere on the wire: item 24's principal is
*declared*, not proven, and its own residue says same-uid callers are
indistinguishable. Stamping `scope: "tenant"` would name a boundary nothing can
verify — a label that reads as enforcement and is not. Configurable lost because
it does not avoid the decision, it ships a knob plus a default and asks the
question again at the default.

So the deliverable is a pin and a statement rather than code: **a watched file is
readable by every caller, including one naming no principal.** A host that wants
otherwise sets an ACL at ingest, or holds the claim with `review_policy`.

**One gap, recorded rather than papered over.** The test pins the durable leg
only. The RAM leg reaches `Worker::submit`, whose job waits in the channel for a
worker loop the test does not spawn, so a wrong ACL written there is invisible to
it. Both call sites pass `Acl::default()` and cannot drift without someone
editing one — which is precisely the edit this test would miss. Closing an
end-to-end assertion on that leg wants a running worker loop in the fixture.

**Title narrowed 2026-07-22.** It read "a bare `query {id}` still filters
nothing" for a day after that stopped being a defect. The id path runs
`matches_filter` (`src/mcp/tools_query.rs:151-152`), so `query {id, principals:
["bob"]}` on an alice-scoped row answers `thought not found`
(`id_read_withholds_a_scoped_row_from_a_non_member`); a *bare* `query {id}`
filtering nothing is the **decided** empty-principals default, pinned by
`bare_id_read_still_serves_a_scoped_row`, not a gap. What the old title hid is
below: the row was gated and its **edges were not**, on both the id read and the
ranked read.

**Four bullets done 2026-07-21; the item stays open.** `Entity` carries `Acl`
(struct `{scope, users, groups}`, `src/base/types.rs:133-137`) and it is now
written from the caller, not hardcoded — but "enforced in `matches_filter`" is
only worth what the read surfaces that *run* `matches_filter` are worth, and two
of them did not. Both are fixed below; the ones that still do not are listed at
the end, and they are what keeps this item open. What shipped:

- ~~Expose `principals` / `scope` on the MCP `ingest` schema.~~ **Done.**
  `IngestArgs` takes both; `acl_from_args` (`src/mcp/tools_mutate.rs`) builds the
  `Acl` on the same pre-branch line as `valid_until`, so the sync, durable-direct
  and RAM-queue paths all carry it. It rides `ingest::Job::acl`
  (`src/ingest/worker.rs`) into `new_statement_entity`
  (`src/ingest/place.rs`), which no longer hardcodes `Acl::default()`.
  `DirectJob::acl` (`src/ingest/direct.rs`) carries it across the durable hop for
  the same reason `valid_until` is carried: the caller's principal is gone by the
  time the drain runs, so dropping it there would silently republish a scoped
  ingest as public. `Worker::enqueue`/`run` keep their arity and delegate to
  `enqueue_with_acl`/`run_with_acl` with `Acl::default()` — that is what leaves
  the file watcher and the intake drain public, per the deferred decision below.
- ~~Accept `principals` on `query`.~~ **Done.** `QueryArgs.principals`
  (`src/mcp/tools_query.rs`) → `QueryOptions.principals`
  (`src/retrieval/score.rs`), validated by the shared `parse_principals`
  (`src/mcp.rs`) that both surfaces use. A blank entry is a hard error, never a
  silent skip: it would match the empty `Acl::scope` of every public entity, so
  accepting it would turn a caller's typo into an access decision.
- ~~Enforce in `matches_filter`.~~ **Done.** `acl_admits`
  (`src/retrieval/score.rs`) runs first in `matches_filter`. Both tier-wide rules
  are enforced and tested in `matches_filter_is_the_per_entity_predicate`: a
  scoped **Fact** is dropped for a non-member exactly like a scoped Claim
  (GC-immunity is not ACL-immunity), and an **empty `principals` is no filter at
  all**, not public-only. `principals` also makes `QueryOptions::is_active()`
  true, so an ACL-only query takes the pre-filtered ANN path rather than the
  unfiltered seed path — the same predicate either way.
- ~~**Guard the id path.**~~ **Done 2026-07-21.** `src/mcp/tools_query.rs:137-157`
  now builds `QueryOptions` first and runs the resolved row through
  `retrieval::score::matches_filter` (`src/retrieval/score.rs:231`) before it
  renders, so the id read honours every filter the ranked read honours —
  `query {id, kind: "claim"}` on a `Fact` answers `thought not found`
  (`id_filter_tests`, same file). Previously it returned `entity_detail_by_id`
  directly and no filter of any kind ran, which made ACL decorative and put
  `kern get` — routed here by item 9 — behind the same unfiltered path.
  A bare `query {id}` still filters nothing, because `QueryOptions::default()`
  leaves `valid_at`/`as_of` unset; that is what preserves the retired item 91
  `[retrieval]` decision (closed 2026-07-21, not the open item 91 `[ingest]`) to
  *flag* an expired row rather than hide it. The ACL predicate landed on this
  path and the ranked path at once, because both go through `matches_filter`.

**Coverage is unit tests only, and cannot be more.** `principals` is MCP-only —
there is no CLI flag — and `e2e/conftest.py` drives the `kern` binary over
subprocess with no MCP JSON-RPC client, so nothing in `e2e/` can reach this
surface. Reaching it would mean building an MCP stdio driver fixture, which is
larger than the feature. The behaviour is pinned by
`matches_filter_is_the_per_entity_predicate` (`src/retrieval/score.rs`),
`id_filter_tests` (`src/mcp/tools_query.rs`) and `ingest_acl_tests`
(`src/mcp/tools_mutate.rs`), the last of which follows an `ingest` carrying
`principals`/`scope` all the way to the `Acl` on the placed entity.

**Still open, deliberately deferred:** does the file watcher give `Document`
entities a tenant-default ACL, or leave them public? Recommend configurable,
default public-within-tenant, since the tenant boundary is the process.
`src/ingest/file_watcher.rs:113` hardcodes `Acl::default()` — which is that
recommended default — and `Worker::enqueue`'s public delegation keeps it there
until the decision is made.

**The dedup rule is decided, not deferred.** The survivor keeps its own ACL —
an id *is* its content hash, so one text cannot exist under two audiences and no
other answer is available. The hole was never the entity, it was the `Rephrase`
edge: `merge_duplicate` stored the incoming text **verbatim** on the survivor, a
`Reason` carries no ACL of its own, and every surface that renders an entity
renders its edges (`entity_detail` untruncated, the ranked `edges` array,
`resource_thought`, `format_chains`). A scoped ingest landing within
`dedup_threshold` cosine of any public thought therefore published its own text
to everyone. `merge_duplicate` (`src/base/accept.rs`) now takes the incoming
`Acl` and skips the rephrase write when it differs from the survivor's, in either
direction; a support observation is metadata about a statement and still merges,
the wording does not (`ingest::dedup::tests`). The supersede path was already
handled: `src/tick/tasks.rs` carries the old entity's `Acl` into
`build_chunk_entity`, so a rephrase cannot launder a scoped thought into a public
one.

**The `query` tool gated the row and published its neighbours — closed
2026-07-22.** The 2026-07-21 bullet above guarded *the row* an id names and
stopped there, and the same paragraph that enumerated the four surfaces which
render an entity's edges (`entity_detail` untruncated, the ranked `edges` array,
`resource_thought`, `format_chains`) fixed the last two and left the first two.
A `Reason` carries no ACL, but `link` writes its body from
`explain_relationship_prompt` — up to 500 chars of **both** endpoint texts — so
an edge is its endpoints' text under an id that is neither one's. The row
clearing `matches_filter` says nothing about its neighbour, so
`query {id: <public row>, principals: ["bob"]}` served an alice-scoped Fact's
text verbatim through any public neighbour, and the ranked read did the same at
120 chars. Both are the *reads item 9 routes `kern get` to*, which is what made
this an ACL bypass rather than a cosmetic gap.

Fixed at the root rather than in the two branches: the endpoint verdict left
`src/mcp/resources.rs` and became `src/mcp/acl.rs`, one `Endpoint` +
`incident_edge` all four renderings call. It takes the *admission rule* as a
parameter rather than the principals, because the two surfaces disagree about
what "allowed" means and must keep disagreeing — resources can name no principal
so its rule is `Acl::is_public`, while `query` takes the caller's `principals`.
Variants renamed `Public`/`Scoped` → `Admitted`/`Withheld` for the same reason;
`Unresolved` is unchanged and still redacts rather than drops. The `query` side
of the rule is `retrieval::score::acl_admits_entity`, the ACL half of
`matches_filter` lifted out so the edge gate cannot re-derive the
empty-principals default and get it wrong — an edge answers its far endpoint's
**ACL and nothing else**, since `kind` or `since` on the row a caller asked for
says nothing about whether a neighbour's quoted text may be read.
`entity_detail_by_id` — the local `kern get` fallback, which has no principal to
name — passes `QueryOptions::default()` and so renders exactly as before. Pinned
by `edge_acl_tests` (`src/mcp/tools_query.rs`): id read and ranked read each
withhold a scoped **Fact**'s edge text from a non-member and each fails alone
when its own gate is reverted, a member still reads the edge whole, and
`a_bare_id_read_still_renders_the_whole_edge` pins the inert default on the edge
rendering too.

**Two reads returned entity text without passing the predicate**, found by
enumerating the read surfaces rather than trusting the single gate — both now
run `matches_filter`. *Cold-tier backfill* (`src/mcp/tools_query.rs`) pushed
`Store::cold_search` hits straight into the delivered set; that is a raw cosine
scan answering no filter, so spilling an entity was the way around every
predicate the hot path enforces, ACL included. *Path chains* (`format_chains`)
render the text of every entity on a walk and `retrieve` filtered only
`results` — the ACL stopped the row and the chain printed it anyway. A chain
touching a withheld entity is dropped whole: a chain with a hole in it still says
the withheld thought exists and what it connects.

**The resources surface is now default-deny** — the separable half, done
2026-07-21. `src/mcp/resources.rs` can name no principal, so the question it can
answer without item 24 is what such a surface may return *at all*, and the answer
is: only rows carrying no ACL. All three reads enforce it. `kern://local/thoughts`
(`resource_thoughts`, the top 50 by rank, text truncated to 200) skips a
non-public entity *before* it ranks, so a withheld row does not even spend a slot.
`thought://{id}` (`resource_thought`, full text **plus every incident edge's
text**) reads a scoped id back as `thought not found` — the same `None` arm and
the same format string a missing id takes, byte-identical for a given id, and the
file logs nothing, so the surface that withholds the text does not leak the id's
existence. `reason://{id}` (`resource_reason`) previously checked nothing at all;
that was the worst of the three, an unchecked read of scoped entity text through
an id that is not the entity's. "Non-public" is `Acl::is_public()`
(`src/base/types.rs`), the same emptiness test `acl_admits` runs, so this surface
and `matches_filter` cannot drift apart on what "public" means.

**An edge is gated on BOTH its ends, and "did not resolve" is its own answer.**
`explain_relationship_prompt` (`src/base/util.rs:87`) hands the LLM up to 500
chars of both endpoint texts and the reply becomes `reason.text`, so the text
belongs to the endpoints and a public `from` does not make it public — an earlier
cut of this gated `reason://{id}` on `from` alone and still served an edge whose
`to` was scoped, naming that scoped id outright. Both ends are checked now. The
harder half is that `find_entity` (`src/base/search.rs:148`) walks only the
**resident** kern map — `loaded` is `kerns.get`, `all()` is `kerns.values()`,
neither sees `unloaded` or the cold tier — so *an id that does not resolve is not
an id that does not exist*. A GC cold-spill (`src/tick/stigmergy.rs`) or a
kern-cap unload (`GraphGnn::unload`) leaves a scoped row alive in the store with
its ACL intact and invisible here, while the edge quoting it survives untouched:
a kern hosts a reason iff it hosts its `from` (`src/base/reason.rs:78`), so
`move_entity` leaves an incoming edge behind in the *source* kern and
`remove_entity` cascades only within one kern. Treating that as "allowed" is a
fail-open read of scoped text, with no race in it — it is the stable committed
state. So the endpoint verdict has three outcomes (`Endpoint`, now
`src/mcp/acl.rs`), not two: **Withheld** drops the edge, **Admitted** serves it
whole, and **Unresolved** serves the edge
with its `text` withheld. Redaction rather than a drop, because a dangling
endpoint is ordinary here (`Reason::to` is optional in `add_reason`) and dropping
every edge with one would hide a public entity's own structure — that is the line
between default-deny and deny-all. `resource_reason` runs the same rule, except
that `from` fails closed on Unresolved too: it is the entity the edge hangs off,
and one that did not resolve is not one that said the read was allowed.

Pinned by seven tests in `src/mcp/resources.rs`, one per guard — each guard
mutated away fails exactly its own test and no other. The four that predate them
seed `Entity::default()` and are unchanged, the regression guard that default-deny
did not become deny-all. This is deliberately narrower than any principal scheme
item 24 lands — default-deny can only be widened by it, never contradicted.

**Still ungated — this is what keeps the item open.** What is *not* separable is
how a principal arrives — resources get one per read or the server gets a session
one — and that is item 24's residue. **Restated 2026-07-22:** a principal now
does arrive on `kern.sock` (`AuthReq::principal`), but *declared*, not proven,
and nothing reads it; a session principal a caller asserts about itself is not
one an ACL can hold it to. Until then the
surface serves public rows to any client that can open the transport, and a scoped
row to nobody; the ACL is still not a boundary a caller can be *held to*, only one
they cannot get around here. Two residues are deliberate and named rather than
closed. A **cardinality oracle**: `kern://local/health` and `kern://local/kerns`
count every entity and reason, scoped included (`graph_health_stats`,
`src/base/health.rs:48-54`; `k.entities.len()`, `resource_kerns`), so ingesting a
scoped row moves a number. It discloses no id and no text, and it is the same
count the operational `health` tool and CLI report — narrowing it is the separate
question of what an unauthenticated *operational* surface may say, which belongs
with item 24. And an **Unresolved endpoint id is still named** in the edge body:
ids are `content_hash(text)`, so at worst that confirms a guessed text, never
discloses one.

**Gossip egress** replicates a scoped `Entity` to peers with no ACL gate — the
`Acl` does ride the wire and `merge_entity` never imports a remote ACL over a
local one, so neither side can widen the other's, but shipping the row at all is
a trust decision nobody has made. And **`Reason` still has no ACL of its own**:
`link`'s `explain_relationship_prompt` writes a scoped entity's text into an edge
hanging off a public one, and every reader has to re-derive the verdict from the
endpoints. All four renderings now do, through the one `src/mcp/acl.rs` verdict —
which is the cheap fix, not the right one: re-deriving it per read costs a
`find_entity` per edge and fails open on a non-resident endpoint by design.
Storing the verdict on the edge at write time is the real fix and is not this
item.

### 20. Source-trust weighting `[retrieval]`

- ~~**A trust prior in the boost step.**~~ **Done 2026-07-21.** `apply_boosts`
  (`src/retrieval/score.rs:94`) now multiplies the composite by
  `RetrievalConfig::source_trust` (`src/config/retrieval.rs:55`), a map keyed on
  `Source::scheme()`. Empty by default, and an absent key is exactly `1.0`, so an
  unconfigured kern scores bit-identically. Post-fusion, as required — RRF is
  rank-based and a multiplier there would be meaningless.

**What remains is the part the item assumed and the data does not carry: an
`Entity` knows the CHANNEL it arrived on, never its AUTHOR.** `Source` is
`{File, Ticket, Session, Agent, Inline}` — a URI scheme. `kern ingest`, the
human path, writes `Source::Inline` (`src/commands/ingest_cmd.rs:62`), and the
MCP `ingest` tool's default writes `Source::Inline` too
(`src/mcp/tools_mutate.rs:231`). One tag, two trust principals: no weighting
keyed on scheme can separate a person from an agent, so `source_trust_user` and
`source_trust_agent` were not built — they would have been labels for a
distinction nothing records.

Nor does confidence stand in for it. `clamp_confidence`
(`src/base/math.rs:201`) caps a non-`USER_SOURCE` write at `MAX_AI_CONFIDENCE`,
which after `beta_params_from_confidence` (`src/ingest/place.rs:16`) is a 0.667
against 0.650 posterior — a 2.6% edge over an MCP agent, and since item 95 the
same 2.6% over the file watcher, whose `tag` is now its `source` `scheme`
(`src/ingest/file_watcher.rs:85`) instead of a raw `1.0`. So the item's
own headline — a user-authored claim outranking an auto-ingested one at equal
heat — is true by default at last, but only by 2.6%, which is a rounding error
rather than a trust model. `source_trust = { file = 0.8 }` is how to make it
mean something, and it is the channel it penalises, not the author.

The blocker is an author principal on `Entity`, stamped at each write path — a
new field, so a store format bump, and it belongs with whoever holds
`src/base/types.rs`. Deciding behavior: fix-the-root. Until then, do not add
`_user` / `_agent` knobs; they would read as working and weight nothing.

### 21. The review lifecycle has a caller-facing surface: `promote` and `exclude_pending` — closed 2026-07-22 `[surface]`

**Corrected 2026-07-22 — "three of four parts landed" counted a part no caller
can reach.** Two parts landed and are reachable. `ReviewState` is on `Entity`
(`src/base/types.rs:300`) behind a `FORMAT_V7` bump — old stores are rejected
rather than defaulted, per the `FORMAT_V6` precedent, pinned by
`decode_rejects_older_version_bytes`. Source-level policy is
`IngestConfig::review_policy` (`src/config/ingest.rs:17`), keyed on source scheme
with unknown schemes rejected at config load (`:33`), resolved once at the ingest
gate by `review_for` (`src/ingest/worker.rs:57`). Both work.

**The third part was engine-only, and re-verified as such before any code was
written.** `exclude_pending` was already a real `QueryOptions` field
(`src/retrieval/score.rs:54`) and a real predicate (`:242`) that also makes
`is_active()` true (`:69`) so the filter takes the pre-filtered ANN path — but
**nothing outside `src/` could set it to `true`.** It was absent from `QueryArgs`
and from the `query` tool's schema `properties` (both `src/mcp/tools_query.rs`),
and there was no CLI flag; walking every `exclude_pending` in the tree found
exactly one write, in its own unit test (`src/retrieval/score.rs:610`). So the
hold half was as unusable as the release half, for the same reason — the state
was decided entirely inside the process. That is why this was one slice:
`promote` alone would not have made the feature usable, because a host that
promoted a row could still never have filtered it.

**The default is `Active`, deliberately.** Pending-by-default would have made
every existing ingest path silently non-retrievable — a behaviour change
disguised as a schema addition, and one that craters recall rather than failing
loudly. Active-by-default means the feature does nothing until a host opts in,
which is the correct direction for a filter that can hide data.

**Both halves shipped 2026-07-22, as one slice.** The release half is a
`promote` tool (`src/mcp/tools_mutate.rs`) with a dispatch arm in the single
`match name` and a `kern promote <id>` subcommand routing through `route()`,
which is generic on the tool name (`src/commands/route.rs:10`) — so this cost a
dispatch arm and a subcommand, not a new route, exactly as predicted. Both the
tool and the CLI's no-daemon fallback go through one `graph_ops::promote_entity`,
so the routed and local writes cannot disagree about what "reviewed" means. It is
idempotent (`promoted: false` on an already-active row) and loud on an id nothing
resolves — a silent success there would tell a curator a claim was released while
it is still held. The hold half is `exclude_pending` on the `query` schema
(`properties`), on `QueryArgs`, carried by `build_query_options`, and behind a
`kern query --exclude-pending` flag.

**A third unreachability was found while building, one layer below the two this
item recorded.** `[ingest] review_policy` could not be set from a `kern.toml` at
all: `Config::load_with_user` refused the whole `[heat]`/`[ingest]`/`[retrieval]`
tables as preset-managed, so the policy that decides what is held was settable
only from inside the process — the same defect as `exclude_pending`, one level
down, and it made the e2e impossible. Fixed at the root rather than worked
around: what a preset owns is TUNING, and `Preset::apply` writes exactly one key
in that table (`ingest.dedup_threshold`). `[ingest]` now accepts `review_policy`
and nothing else; `[heat]` and `[retrieval]` are untouched, and a tuning knob
smuggled in beside `review_policy` is still refused. Both directions are pinned
by `a_real_kern_toml_can_set_review_policy_and_nothing_else_in_ingest`.

**The tradeoff, taken deliberately rather than sequenced behind 24.** `promote`
lands on the same socket `intake drain` widened — which now authenticates the
connection but still cannot tell one same-uid caller from another — but it is a
wider claim than `drain`: draining asserts no authority, and releasing a held
claim IS a curation-authority decision — anything that can open the path can
release one. Item 24 is in flight and will gate this; **promote's authority rides
on whatever 24 lands**, and that is said on the tool description, on
`cmd_promote`, in `FEATURES.md`'s tool table and in the user-facing
`configure.mdx`. Taken now because the alternative was shipping neither half, and
a host that enabled `review_policy` today would strand every claim it held.

**Coverage.** Unlike item 18's `principals`, this is e2e-measurable, and it is
measured: `e2e/test_review_lifecycle.py` runs the whole loop — policy holds an
`inline` ingest, `query --exclude-pending` misses it, `kern promote` releases it,
the same query returns it — twice, once against a serving daemon blinded the way
`test_graviton_routing` blinds it (so the release provably landed in the daemon's
live graph and survived its persist) and once with nothing serving, for the
`NoDaemon` fallback. `e2e/conftest.py`'s `write_config` grew one `review_policy`
kwarg, emitted only when set so every other test's config text is byte-identical.

Original text, kept for the record: `ReviewState` on `Entity` (added with a store
format-version bump — alpha rejects old stores rather than defaulting them) +
source-level review policy in config + an `exclude_pending` query filter and a
`promote` tool. Lets a host hold auto-distilled claims out of retrieval until a
human curates them. Requires 18's `QueryOptions` work first — review filters are
more `matches_filter` predicates. *(The sentence "No `ReviewState`,
`exclude_pending` or `promote` exists in `src/`" was true when filed and is now
wrong about the first two.)*

### 24. RPC socket authenticates the connection but not the caller — same-uid callers are indistinguishable `[surface]`

`FEATURES.md:677-685`. **Mostly closed 2026-07-22, and deliberately left open —
read the residue at the bottom before citing this as a blocker.** The socket
now authenticates; what it still cannot do is tell one same-uid caller from
another, which is the half items 9 and 18 were waiting on.

**Narrowed 2026-07-22 — "anything that can open the path" was already false on
Unix when this was written.** `harden_socket`
(`src/trnsprt/src/typed/local.rs:355`) sets the socket `0600` on both the fresh
bind and the stale-rebind path, pinned by `a_bound_socket_is_owner_only` and
`a_rebound_stale_socket_is_also_owner_only`, so a foreign uid never reaches it
and the only residue is the sub-ms bind→chmod window item 84 already carries.
No document said so — `FEATURES.md` §13 still read "anything that can open the
path", which is why this is recorded here rather than assumed.

**Built 2026-07-22 — the connection is authenticated.** One `AuthReq` frame
(`src/trnsprt/src/kern_rpc/auth.rs:79`) carrying the graph's `mcp-token` — the
same secret the HTTP surface already demands (`resolve_mcp_token`,
`src/config/serve.rs:64`), never a second one — is compared in constant time
before anything dispatches. The ordering is structural, not remembered:
`serve_authenticated` (`src/rpc/kern_rpc_server.rs:174`) builds the handler
*inside* a closure that only runs after the verdict, so on a refused connection
no handler exists for a method to reach. Every non-match returns `Err`,
including an empty `expected` — a daemon that cannot read its secret serves
nobody rather than everybody — and `run_server` resolves the token before it
binds, so that state is unreachable in practice as well as harmless. Windows
gets the same posture the Unix `0600` states: an owner-only SDDL,
`D:P(A;;GA;;;<user>)`, built from the process token's own SID
(`src/trnsprt/src/typed/local.rs:392`) and passed to *every* pipe instance — <!-- docs-check: anchor-ok -->
the `accept`-created ones too (`:641`), since an instance created without it
would be a hole beside a locked door.

**The tradeoff that was taken, named.** The secret proves *a uid*, not *a
program*. The CLI, the `kern mcp` proxy an agent drives and the hub all run as
the same user and can all read the same file, so no shared secret can separate
them. `principal` is therefore **declared and recorded, never enforced** — the
handler carries it, nothing consults it. This is honest but it is not what item
9 needs to route `ingest`/`link` with their trust intact, nor what item 18 needs
for a principal to survive past the MCP surface. Both must still cite something;
they should now cite the *unproven principal*, not the missing socket auth.

**Why it stays open — four residues, none of them "the gate might not hold".**
The gate holds: making verification always succeed fails the no-token and the
wrong-token tests, and gutting the byte compare fails them too (both mutations
re-run 2026-07-22). What is left is everything the gate does not cover.
1. **Windows is unexecuted.** The descriptor typechecks — `cargo check --target
   x86_64-pc-windows-msvc -p trnsprt` is clean, and a deliberate type error
   inside the `cfg(windows)` module does fail it, so that is not a vacuous
   pass — but no line of it has ever run. The SDDL is unparsed, the token query
   unmade, and there is no Windows test: `bind_tests_unix` is
   `#[cfg(all(test, unix))]` and stays that way. Treat the pipe as *believed*
   owner-only, not *known* to be.
2. **`principal` is unproven**, above.
3. **The socket secret is the HTTP secret, and the socket path is squattable.**
   With no `XDG_RUNTIME_DIR` the endpoint falls back to `/tmp/kern-<tag>-<user>.sock`
   (`Endpoint::scoped`, `src/trnsprt/src/typed/local.rs:44-55`), and `/tmp` is
   sticky-but-writable: another local user can bind that name first. On the
   socket the stolen token buys nothing (the real socket is `0600`), but it is
   the *same* token `mcp_addr` demands, so a socket-side disclosure is an
   HTTP-side compromise. That is the cost of reuse, and reuse was still the
   right call — a second secret is a second thing to mint, rotate and get
   wrong.

   **The disclosure is closed 2026-07-22 — the client now authenticates the
   server, twice.** `require_owned_by_caller`
   (`src/trnsprt/src/typed/local.rs:237`) stats the endpoint and refuses unless
   both the name and what it resolves to are owned by this euid.
   `require_peer_is_caller` (`:283`) then reads `SO_PEERCRED` off the connected
   socket and refuses unless the process serving it is this euid. Both sit in
   `connect_kern` (`:314`), which returns before
   `present_auth` (`src/trnsprt/src/kern_rpc/client_local.rs:44`) writes the
   token — so both are ahead of every byte a client could send, and a check
   after frame 1 would be decoration. It fails closed: any stat error, a
   dangling symlink, and an unreadable peer credential all refuse.
   `Endpoint::hub()` is the same `scoped()` name and reaches the wire through
   the same `connect_kern`, so the hub socket is covered by construction rather
   than by a second check.

   **Why two checks and not one.** The stat is the cheap one and it is not
   sufficient on its own: it describes a *name* at one instant, and the window
   between it and the `connect` is opened by our own daemon rather than by an
   attacker — `Drop for LocalListener` (`src/trnsprt/src/typed/local.rs:654`)
   unlinks the socket on every shutdown and the stale-rebind path unlinks it
   too, so a name that stats as ours can be free a microsecond later and rebound
   by somebody else before the `connect` lands. Waiting for a daemon restart is
   not a privilege an attacker has to earn. `SO_PEERCRED` is the fact the kernel
   recorded when the peer called `listen`, so no rename can move it; the stat is
   kept in front of it only because refusing before opening a connection gives a
   message that names the squatter's uid. **Do not describe the stat alone as
   sufficient** — an earlier draft of this entry did, and it was wrong.

   **Both checks are mutation-tested (re-run 2026-07-22).** Neutering
   `require_owned_by_caller` to `Ok(())` fails 6 of 6 targeted tests, including
   `a_foreign_owned_endpoint_is_refused_before_the_token_is_presented` — the
   ordering assertion, which holds because connecting to a root-owned path
   fails either way but as `UntrustedEndpoint` with the check and `Io` (EACCES)
   without, so it cannot pass for the wrong reason. Neutering the peer uid
   comparison fails `the_peer_check_reads_the_server_uid_and_decides_both_ways`.
   That test injects the expected uid rather than reading `geteuid()`, because
   a socket bound by a second uid is not something a test can create.

   **What closed and what is still owed, in order of how much it matters.**
   - **CLOSED 2026-07-22 — the bind path refuses instead of standing down.**
     The `AddrInUse` arm (`src/trnsprt/src/typed/local.rs:513-540`) now runs
     the same two checks `connect_kern` does, in the same order:
     `require_owned_by_caller` on the name, then `UnixStreamAdapter::connect`
     and the peer check on whatever answered. Either refusal returns the new
     `BindError::Untrusted` (`:335-346`) carrying the foreign uid, never
     `BindOutcome::AlreadyRunning`, and — the half that was not recorded
     before — never reaching the `remove_file` below it. That unlink was the
     wider bug: it ran on a path nobody had verified, and while `/tmp`'s
     sticky bit refuses it for a foreign *socket*, it does not protect a
     *symlink*, so a link this uid owned pointing at a foreign target was
     unlinked and rebound.

     **Fail-closed, traced rather than asserted.** There is exactly one
     `remove_file` in the function (`:536`) and it sits inside the `Err(_)`
     branch of the connect, past the `?` on `require_owned_by_caller`, so it
     is unreachable on a name that has not been proved ours. The three error
     shapes were run: a dangling symlink refuses and the link survives, a
     vanished path refuses (the `Io(NotFound)` the predicate returns for
     absence is mapped to `Untrusted` here, because in *this* arm the kernel
     just said the name was taken), and a plain file this uid owns is still
     reclaimed. **The tradeoff that buys:** a name that disappears between the
     `EADDRINUSE` and the stat — a predecessor exiting mid-race, whose `Drop`
     unlinks — now refuses where it used to rebind. Fail-closed was chosen
     over a retry, and the operator sees the reason because the refusal
     prints; a planned handover does not come through here at all, it comes
     through `adopt_kern_listener`.

     **Reclaiming our own stale socket still works**, and not only per the
     mode tests: verified against a real second process — a same-uid,
     different-exe daemon bound the path, read as `AlreadyRunning` while
     alive (`SO_PEERCRED` proves a uid, not a program), was `SIGKILL`ed, and
     the next bind reclaimed the leftover socket and hardened it to `0600`.
     Pinned in-suite by `a_stale_socket_file_is_removed_and_rebound`,
     `a_bound_socket_is_owner_only` and
     `a_rebound_stale_socket_is_also_owner_only`.

     **The refusal is visible at both call sites**, which is the point of it:
     `run_hub` (`src/hub/serve.rs:305`) already `eprintln!`ed its `Err` arm;
     the daemon's (`src/commands.rs:853`) only `tracing::error!`ed, so a
     refusal there would have printed nothing at the default level while the
     `AlreadyRunning` arm beside it printed to the terminal. It now does both.

     **Mutation-tested 2026-07-22, and both checks are caught.** Neutering
     `require_owned_by_caller` to `let _ =` fails
     `a_symlink_to_a_foreign_target_refuses_the_bind` — the test built on the
     `foreign_path()` helper, which lives in `owner_tests_unix` (not
     `bind_tests_unix`, as an earlier draft of this entry said) and is now
     `pub(super)` so the bind tests share it rather than growing a second
     copy. Neutering the peer check fails
     `a_live_endpoint_served_by_another_uid_refuses_the_bind`. Reverting the
     whole arm to its pre-change body fails both and leaves the other four
     bind tests green, so neither test is a tautology.
   - **The peer check's wiring is covered in the bind arm and not in
     `connect_kern`.** It was recorded here as untestable on one uid; it was
     untested. The arm is reached through `bind_unix(path, expected_peer)`
     (`src/trnsprt/src/typed/local.rs:507`), split out exactly as
     `require_peer_uid` is split out of `require_peer_is_caller` and for the
     same reason, so a test drives the whole arm against a real socket and a
     real `SO_PEERCRED` read with a uid that is deliberately not the server's.
     The two alternatives considered do not bite: a same-uid child with a
     different exe is *correctly* accepted,
     because the check claims a uid and nothing finer (measured above), and an
     abstract-namespace socket or a `socketpair` has no filesystem name, so a
     path bind never returns `EADDRINUSE` for one and the arm is never
     entered. `connect_kern` has no such seam and its `require_peer_is_caller`
     call remains code-review-only — one line, not two.
   - **The live squat end to end is still not executable.** A real foreign
     daemon holding the real path needs a second uid. What is proven is the
     symlink shape, the verdict, and the arm's control flow; what is not is a
     genuine cross-uid squat.
   - **Windows gets no analogue and needs none** — a named pipe has no owning
     uid, and the server side already pins every instance to this process's
     SID, so both checks are `cfg(unix)`.
   - **e2e measures only half of this, and cannot measure the other half.**
     `e2e/conftest.py` sets `XDG_RUNTIME_DIR` per test, so e2e always takes the
     XDG path and never the `/tmp` fallback that is the vulnerable one, and it
     has no second uid with which to create a foreign-owned socket. So
     `e2e/test_daemon_reads.py` and `e2e/test_graviton_routing.py` pin exactly
     one thing — that the checks did not break connecting — and the refusal
     itself is unit-test territory. The bind half added 2026-07-22 is covered no
     better: same `XDG_RUNTIME_DIR`, same single uid, so e2e confirms only that
     the daemon still binds and serves, never that it refuses. Do not read a
     green e2e run as evidence that the squat is covered.
4. ~~**The pre-auth frame is unbounded and untimed.**~~ **Closed 2026-07-22 by
   item 98.** This entry read the defect correctly — per connection rather than
   an accept-loop stall, and a cap belonging in `decode` — and named the one
   thing it could not settle: the number. `verify_auth`
   (`src/trnsprt/src/kern_rpc/auth.rs:98`) now bounds that read at 1 KiB and 5 s
   and lifts both once the frame is in hand, so `call_tool`'s whole documents
   still travel the framing they need
   (`JsonEnvelopeCodec::decode`, `src/trnsprt/src/typed/codec.rs:53`). The
   ranking stands as written: `0600` means the caller was already same-uid, so
   this was a robustness bound, not a boundary failure.

The item's second half is **retired 2026-07-21 — verified
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

`seed_important` (`src/retrieval/seed.rs:127`) iterates `g.all()` ×
`kern.entities`, called unconditionally once per retrieve
(`src/retrieval/query.rs`, in `retrieve_profiled`).

**Narrowed 2026-07-21, after item 26 landed.** The scan is 1.9–3.9× faster and
its share of retrieve is roughly halved, but it is still O(N): what this pass
removed was a parallelism bug, not the linear walk. The index the item asks for
turned out to be blocked, and the blocker is named below so the next attempt
does not rediscover it.

**The old tables are replaced, not amended — both were measured before item 26.**
Re-measured with the same instrument (`tests/seed_scale.rs`, release, 20 reps per
configuration), the PageRank comparison inverts. PageRank is now ~2–3 ms rather
than a flat ~20 ms, so the scan is the larger cost everywhere above 1%
eligibility:

| N=100k | scan | PageRank | scan ÷ PageRank | scan as share of retrieve |
|---|---|---|---|---|
| 1% eligible | 2.37 ms | 2.46 ms | 1.0× | 39.7% |
| 10% eligible | 6.74 ms | 2.93 ms | 2.3× | 60.1% |
| 50% eligible | 18.82 ms | 2.08 ms | 9.0× | 72.1% |
| 100% eligible | 35.69 ms | 2.54 ms | 14.1× | 71.2% |

The superseded row "100k, 1%, 12.3% of retrieve" is not a contradiction: the
scan cost the same, but retrieve no longer carries PageRank's 20 ms, so the same
absolute cost is a far larger share. The old item said the numbers would decide
the ordering; they have.

**Fixed here: the scan was never actually parallel on the ordinary corpus.**
`kerns.par_iter().flat_map_iter(...)` parallelised over *kerns* and walked each
kern's entities on a single thread, so a one-kern graph — what `e2e/` builds and
what a fresh install is — scanned the whole corpus serially however many cores
were free. This item's own "Rayon-parallel" claim was wrong in exactly the case
that matters. The inner walk now splits too
(`src/retrieval/seed.rs:149`), on 8 cores:

| N=100k | before | after | |
|---|---|---|---|
| 1% eligible | 2.37 ms | 1.24 ms | 1.9× |
| 10% eligible | 6.74 ms | 1.73 ms | 3.9× |
| 50% eligible | 18.82 ms | 8.19 ms | 2.3× |
| 100% eligible | 35.69 ms | 12.04 ms | 3.0× |

Recall is exactly unchanged (0.9306 / 0.9722 / 0.9471). The selection is
bit-identical rather than merely equivalent —
`parallel_importance_scan_equals_the_sequential_scan_it_replaces` compares ids,
rank positions and score *bit patterns* against an independently written
sequential gate over a generated three-kern graph. At N=10k the change is inside
the noise floor of a box running three worktrees, and it buys nothing there;
the win is a large-corpus win.

**The index is blocked on the absence of an entity-mutation chokepoint, and
`mutation_epoch` is not one.** `bump_mutation_epoch`
(`src/base/graph.rs:439`) has exactly three callers, all inside `graph.rs`:
`get_mut`, `register`, `deregister`. But `GraphGnn::kerns` and `Kern::entities`
are public fields, and ~20 non-test sites mutate them directly without passing
through any of the three — `merge_remote_entity` (`src/base/merge.rs`) inserts a
fresh Fact, `reembed` replaces every vector through `values_mut`, gossip writes
phantom-kern entities, clustering moves entities between kerns. Each of those
changes an input to the importance gate (`has_vector`, kind, `access_count`)
while leaving the epoch untouched.

Worse, the one mutation that *creates* importance is epoch-silent on purpose:
`commit_access_ids` (`src/retrieval/score.rs:354`) stamps access on every
delivered result and deliberately bypasses `get_mut` so it will not invalidate
the semantic query cache (`src/retrieval/score.rs:320`). An eligible-set index
keyed on the epoch would never see a Claim cross
`important_access_threshold` — stale forever, in the direction that silently
drops seeds and moves recall with no error anywhere.

That is verified rather than argued: installing a `mutation_epoch`-keyed memo
over `seed_important` makes
`an_eligibility_change_is_reflected_with_no_epoch_bump` fail on "crossing the
access threshold makes an entity important on the very next retrieve". The test
is left behind as the guard, so the next attempt at an index fails loudly
instead of shipping the regression quietly.

**What is left is therefore a different question: what makes entity mutation
observable?** Two candidates, neither cheap. Make `Kern::entities` private behind
an accessor that versions its kern — correct by construction, but mechanical
across ~40 call sites in `ingest/`, `tick/`, `gossip/` and `commands/`. Or
hand-maintain the eligible set at each mutation site the way `entity_idx`
already is — the convention the codebase actually uses, and the reason
`merged_remote_entity_is_vector_searchable_without_rebuild` exists — at the price
of a fifth thing every mutation site must remember. Not decided here.

The costs to weigh when it is: an eligible-id set is ~80 B per eligible entity
(a `String` plus its set slot), so ~8 MB at N=100k fully eligible, and it puts
O(1) index maintenance on every write plus a full O(N) rebuild whenever the
freshness signal is lost. An index that slows ingest to speed up query is the
trade, and it only pays where eligibility is low — a corpus where everything is
eligible cannot be helped by an index at all.

### 26. PageRank allocates four N-sized buffers on every query — closed 2026-07-22 `[retrieval]`

**Narrowed 2026-07-21, narrowed again 2026-07-22, closed 2026-07-22.** The flat
per-query cost went first, then the full-reach regression that replaced it, and
last the allocation the title names. Each stage is kept below so the closed parts
are not re-opened and the numbers that are now stale are marked as such.

**Measured before, `tests/seed_scale.rs` in release, default minus
`pagerank_enabled: false`.** At N=100k it cost a flat **~18 ms per query** —
18.8 / 17.9 / 18.1 / 17.1 ms at 1 / 10 / 50 / 100% eligibility. Flat was the
finding: the cost did not shrink when the query filtered hard, because the power
iteration walked the whole adjacency regardless of how few entities survived.

**Measured after, same harness.** 1.7 / 3.1 / 1.9 / 1.3 ms at the same four
points; a filtered retrieve went from 24.0 ms to 7.1 ms end to end. Three
post-change runs put every one of those points in a 1.3–6.0 ms band — the box
has two sibling worktrees building into the same target directory, so ±2 ms is
the noise floor and no single point in that band should be read as exact. The
before numbers do not need the same caveat: they were 17–19 ms at all four
points, which is outside it. The walk is now confined to the teleport support
and what it reaches
(`src/retrieval/pagerank.rs`), which is exact rather than approximate — every
node outside the reached set holds a literal 0.0, so every term the full-width
loop added for it was `+0.0`. `confined_iteration_equals_the_full_width_one_bit_for_bit`
compares the two against each other on bit patterns, not tolerances.

**The item's own proposed closure is unavailable, and this is why.** It read
"the scores depend on the graph, not the query, so persist the vector and
recompute on a tick". They do not: the teleport vector is personalised at the
query's dense and lexical seeds, so a per-graph vector is *global* PageRank —
already weighed and already rejected on the site, "popular entities top every
query, relevant or not". A cache of it would not be a faster version of this
feature, it would be the alternative the feature exists to avoid. Nothing about
cold start or persistence is decided here because there is nothing correct to
persist: personalised PageRank is linear in the teleport vector, so the only
exact cache is one basis vector per seed node, which is O(N) memory apiece and
~75 misses on a cold query. A cache keyed on the seed set instead would report a
large win on any harness that repeats a query — which is the only kind of harness
we have — while doing nothing for real traffic.

**The full-reach regression is closed 2026-07-22, and it did reproduce.** The
cost tracked reach, so a query whose seeds reached the whole graph paid the whole
graph — and paid it through a list where the loops it replaced vectorised over a
slice. Rechecking the old instrument (`cost_against_full_width_by_fanout`) on
this box put out-degree 16 at 31.8 ms against 26.7 ms rather than the 37.7 / 26.3
recorded above; the direction held, the exact 1.4× did not, and that instrument
was never a fair one — it charged its full-width reference a 100k-row sort the
confined path does not pay.

The fair instrument is new: `cost_against_full_width_by_reach` in
`src/retrieval/pagerank.rs`, ignored by default. It confines every edge to a
contiguous block of known size, so the seeds' reach is the block while the graph's
edge count and per-node out-degree do not move with it, and it A/Bs the same
function against itself — same top-k tail, only the loop body differs. N=100k,
75 seeds, 25 iterations, min of 7, the quietest of three runs (a loaded box gives
this a ±20% floor, which is wide enough to hide the whole effect on any single
row — the shape across rows is the finding, not any one of them):

| reached | out-degree 4 | out-degree 8 | out-degree 16 |
|---|---|---|---|
| ~58–60% | 0.90 | 0.82 | 0.88 |
| ~78–80% | 1.08 | 1.03 | 0.92 |
| ~88–90% | 1.13 | 1.06 | 1.06 |
| ~93–95% | 1.17 | 1.17 | 1.11 |
| ~97–100% | 1.22 | 1.16 | 1.29 |

Each cell is confined-only ÷ full-width-once-closed: above 1.00 the confined walk
is the slower of the two.

**The crossover sits at ~80% reach, and reach alone does not decide it.** Below
it the confined walk wins by 0.82–0.92×; from 88% up the full-width loops win by
1.06–1.29×, monotonically in reach, at out-degree 4, 8 and 16 alike. Out-degree
2 never gets past 76% reach on this generator and never crosses; out-degree 1
does not close inside 25 iterations at all, so no switch can fire there and none
needs to.

What ships is `closed && reached * 100 >= n * 90`. 90 rather than 80 because the
two errors are not symmetric — switching too late gives back the 1.1–1.3× band,
switching too early costs up to 1.22× on graphs the confined walk still owns. It
is exact for the same reason the confinement is: every node outside the reached
set holds a literal zero in both vectors, and `x + 0.0 == x` for every
non-negative finite `x`, which is all rank and teleport mass ever are.
`confined_iteration_equals_the_full_width_one_bit_for_bit` now runs its whole
matrix three times — never switch, switch at 90, switch the moment the set closes
— against the one full-width reference, over two graphs placed at 88% and 92%
reach so the threshold is crossed inside the comparison, and it fails if the
matrix walked only one of the two bodies.

**The allocation closed 2026-07-22, and the sparse rank vector was never needed.**
The item said removing the buffers means a sparse representation, and that a hash
map's iteration order would put the `+0.0` argument back in play. That reasoning
had the wrong target. What the buffers cost is the *allocation*, not the *width* —
so the width can stay, dense and ascending and bit-identical, and only the
allocation goes. The four vectors are now lent by the calling thread and handed
back zeroed over the reached set alone, which is the same set the walk already
pays for. No arithmetic moved and no index order moved, so nothing had to be
re-argued: `confined_iteration_equals_the_full_width_one_bit_for_bit` passes
unchanged, and it is a *stronger* test than it was, because its 540 comparisons
now run through one thread's reused buffers — a value left behind by any of them
would surface as a wrong score in the next.

**Measured with an allocator, not a clock** (`floor_by_graph_width_at_fixed_reach`
and `allocation_and_floor_by_reach` in `src/retrieval/pagerank.rs`, both ignored
by default; the counting allocator is `test_support::alloc_probe`). Timing cannot
witness this — 2.5 MB of `calloc` is under the noise of a box with two sibling
worktrees on it — so the gate is the byte count.

| | per-call bytes | largest single block |
|---|---|---|
| before, N=100k @ 1.0% reach | 2,540,344 | 800,000 |
| after, same | 40,344 | 16,384 |

25.40 B/node before, which is exactly the four: 8 + 8 + 8 for `tele`, `rank`,
`next` and 1 for `in_reached`. The largest block before is one N-wide `f64`
vector; after, nothing near it exists. Holding the walk fixed at 976 reached
nodes and moving N alone — 10k, 50k, 200k — the allocation was 300,552 /
1,300,552 / 5,050,552 B before and **50,552 B at all three** after, with the
largest block 80,000 / 400,000 / 1,600,000 B before and 16,384 B throughout.

**What the clock says, which is less than the item claimed.** N=100k at 1.0%
reach, min of 7, four runs a side: **0.310–0.420 ms before, 0.244–0.249 ms
after**. So ~0.065 ms, not the 0.18 ms the item recorded — that 0.18 was the
whole per-query cost at low reach, and the allocation was about a third of it.
At 10% reach it is 5.04–7.66 ms against 4.49–4.64 ms. The direction is
consistent across every paired run; the magnitude is small and should not be
quoted as a headline. The 50 / 90 / 100% rows swing 19–186 ms on this box either
side of the change and decide nothing at all.

**Named tradeoff: the buffers are now resident per thread, not transient per
call.** A thread that has ranked an N=100k graph keeps 2.5 MB until it dies, and
retrievals run under a read lock so concurrent readers each hold a set. Peak RSS
is not worse than before — the old path allocated the same 2.5 MB per concurrent
call — but the steady state is, by thread count. The first call on each thread
still pays the full 2.5 MB, which the bench reports as its `first=` column.

**What this leaves, which is not this item.** The remaining per-call allocation
is proportional to *reach*, not to N: `reached`, `fresh`, `merged` and `scored`
still churn, and at 100% reach a query allocates 4,869,552 B (down from
7,369,552). That is cost proportional to work actually done, which is a different
claim from the one this item made, and it is recorded here rather than opened.
Separately, time at fixed reach still rises with N — 0.19–0.28 / 0.20 / 0.29–0.35
ms at 10k / 50k / 200k after the change, against 0.21–0.30 / 0.25–0.32 /
0.36–0.43 before — so something in the call still tracks graph width. It is not
these buffers; the byte counts above are flat across all three. Not chased.

Item 25's "PageRank ÷ scan" table above is now stale in one direction only: it
is a correct record of what was measured before this change, and the ratios in
it no longer hold. The scan is the larger cost at every eligibility level tested
here, which is the ordering that item said the numbers would decide.

### 27. A GC sweep pays one LMDB commit, not one per victim — closed 2026-07-22 `[lifecycle]`

One item because one sweep paid all of it. The four costs it opened with are
settled — three closed, one withdrawn — and measuring them found a fifth that
actually dominated the sweep, which was none of the four. That fifth closed
2026-07-22 and is the third bullet below.

**Measured 2026-07-21 before touching anything** (`tests/gc_scale.rs`, release,
one sweep per row). `run_gc` returns before eviction once the victim list is
empty, so a sweep over a kern with zero victims *is* the selection scan and
nothing else:

| N entities | victims | whole sweep | selection scan | selection's share |
|---|---|---|---|---|
| 10k | 0 | 0.70 ms | 0.70 ms | 100% |
| 10k | 800 | 4 370 ms | 0.70 ms | 0.02% |
| 100k | 0 | 3.82 ms | 3.82 ms | 100% |
| 100k | 800 | 6 045 ms | 3.82 ms | 0.06% |
| 100k | 80 000 | 278 764 ms | 3.82 ms | 0.001% |

- ~~Victim selection is O(entities) per kern per sweep~~ **Withdrawn 2026-07-21 —
  measured, not fixed.** 3.8 ms at 100k entities against a 6 045 ms sweep. It is
  also linear, never superlinear: one predicate per entity, once per GC interval,
  on a background tick. No index was written and none would help — the predicate
  is decayed heat, a function of `now` and each entity's own `heat_updated_at`
  (`src/tick/stigmergy.rs:49`), so no ordering of entities survives the clock
  advancing.

- ~~The cold tier is a brute-force cosine scan with no index~~ **Closed
  2026-07-21.** It was never the scan that cost: 87–99% of `cold_search` was
  bincode-decoding a whole `Entity` per row to reach its vector. Vectors moved to
  their own LMDB table, so the scan scores off raw floats (`src/base/store.rs:692`)
  and decodes only the k winners (`:709-711`). At the 50k cap, 470 ms → 28 ms per
  call. No index was added: this is on the *recall* path (`src/mcp/tools_query.rs:214`,
  on hot-tier underfill), and an ANN over the cold tier would put a resident index
  back on the tier that exists to not be resident.

- ~~Eviction pays one LMDB commit per victim~~ **Closed 2026-07-22.** It was the
  commit, and only the commit. `cold_spill` and `cold_put_all` encode the same
  rows and issue the same two `put`s per row; they differ only in where
  `write_txn`/`commit` sit, so an A/B over identical batches isolates the
  transaction boundary and nothing else
  (`cold_spill_per_victim_vs_batched`, `tests/gc_scale.rs`, release, dense
  vectors, tier under its cap so no trim pass fires in either column):

  | victims | one commit each | one commit total | per row |
  |---|---|---|---|
  | 100 | 918 ms | 20.5 ms | 9.18 ms → 0.21 ms |
  | 800 | 7 728 ms | 70.2 ms | 9.66 ms → 0.09 ms |
  | 5 000 | 38 053 ms | 284 ms | 7.61 ms → 0.06 ms |
  | 20 000 | 136 012 ms | 1 060 ms | 6.80 ms → 0.05 ms |

  `run_gc` now hands the whole victim list to `cold_put_all` in one transaction
  (`evict_batched`, `src/tick/stigmergy.rs:136`). Whole-sweep effect, both
  columns measured in one sitting on one machine (so these absolutes are ~2.2x
  the 2026-07-21 table's, which is a different host):

  | N | victims | before | after |
  |---|---|---|---|
  | 10k | 80 | 496 ms | 9.4 ms |
  | 10k | 800 | 4 660 ms | 28.1 ms |
  | 10k | 8 000 | 67 811 ms | 250 ms |
  | 100k | 800 | 4 367 ms | 34.9 ms |
  | 100k | 8 000 | 46 884 ms | 222 ms |
  | 100k | 80 000 | 615 919 ms | 2 890 ms |

  The ratio this item opened with has inverted: selection was 0.06% of a
  100k/800 sweep and is now 8.3% of it.

  **What a batched failure means was the real question, and the answer is that
  it means nothing new.** A failed batch falls back to the per-victim loop it
  replaced (`evict_victims`, `src/tick/stigmergy.rs:155`), so the retention
  semantics are unchanged: the row that cannot be spilled stays hot and is
  retried next sweep, every other victim is still collected (`kept`,
  `src/tick/stigmergy.rs:177`). All-or-nothing was the alternative and it was
  rejected: cold GC is the only bound on hot-graph size, so one permanently
  un-encodable row would wedge that bound every hour, forever — a liveness
  failure traded for nothing, since a batch that fails has written nothing and
  loses no data either way. The fallback also absorbs a batch too large for one
  LMDB transaction (`MDB_TXN_FULL`) by finishing the sweep slowly instead of not
  at all.

  Two costs accepted. The batch is a clone of every victim entity before the
  commit — ~80 MB transient at the 80 000-victim row above, of data already
  resident and about to be freed. And one LMDB write transaction is now held for
  the length of a sweep rather than V short ones; at 80 000 victims that is a
  single ~2.9 s hold against 616 s of intermittent holds, so any contending
  writer waits strictly less in total, but a single flush can now block ~2.9 s
  instead of ~7 ms. No deadlock is introduced: nothing inside the transaction
  takes the graph lock, and `run_gc` holds that lock exclusively for the whole
  sweep, so no flusher can hold a graph read lock while waiting on the writer.
  Deciding behavior: name-the-tradeoff.

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

### 28. GNN training is off the tick, linear in edges, and `gnn_train_refused` reaches `kern health` `[lifecycle]` — CLOSED 2026-07-22

~~`TaskKind::GnnPropagate => do_gnn_propagate(...)` runs inline in `process_task`
on the single tick loop, stalling large kerns.~~ **(closed 2026-07-21.)** The
stall was measured before it was changed, by a new instrument in the shape of
`tests/gc_scale.rs` — `tests/gnn_scale.rs`, `#[ignore]`d, release-only. At the
shipped defaults — `min_thoughts` 128 and `train_epochs` 24
(`src/gnn/propagate.rs:16-17`) — over 384-dim vectors, one propagation cost
0.64s at 128 entities, 6.4s
at 1024, 21.6s at 2048 and 79.7s at 4096. Every other tick task at the same
sizes was sub-millisecond: `stigmergy_gc` 0.151ms, `commit_access` 0.002ms,
`idle_sweep` 0.000ms at N=4096. On the real loop (`tick::start`) the recall
path's own heat write-back — `CommitAccess`, enqueued at
`src/mcp/tools_query.rs:200` — landed in 2.2ms with nothing ahead of it and in
**56,787ms** with one propagation ahead of it at N=2048. The premise held.

Training now runs on a dedicated thread (`src/tick/trainer.rs`), and the tick
arm only hands it the kern id (`src/tick.rs:116-121`). Same measurement after:
1.2ms at N=2048, down from 56,787ms. Decisions, all three deliberate:

- **Where.** One `std::thread`, not `spawn_blocking`: that pool is 512 wide, so
  every kern would train at once and each training allocates a dense
  `num_entities^2` adjacency — 134MB at N=4096 alone.
- **Overlap.** The waiting set is keyed by kern id, so a second request for a
  kern already waiting is *coalesced*, not queued (`Submit::Coalesced`,
  `src/tick/trainer.rs:99`) — the waiting job snapshots the graph when it runs,
  so it already covers what the newer request would have seen. Past
  `TRAIN_QUEUE_CAP` distinct kerns (`:16`) the newest is refused and counted
  (`:104`, `:114`), the shape item 30 settled on, and the count reaches MCP health
  as `gnn_train_refused` (`src/mcp.rs:146`).
- **Panic.** Item 2 is closed, so the inline arm was already contained by
  `run_guarded` — moving the work does not improve that, it *relocates* it, and
  a bare thread would have been strictly worse: the first panicking propagation
  would kill the trainer and every later one would silently never run. The
  trainer therefore catches per job (`src/tick/trainer.rs:78`) and records
  through the same `Queue::record_task_panic` the health surfaces already read,
  so `GnnPropagate` keeps being the one task that reports a contained failure.

Moving training off the loop also opened a write-back race the inline version
could not have: an entity superseded *during* training would have been
re-inserted into `gnn_entity_idx` by the apply step, undoing the supersede
removal. `apply_gnn_updates` now re-checks status at write time
(`src/tick/gnn_propagate.rs:166`).

~~Remaining: **the cost itself is untouched.** A propagation still takes 79.7s at
N=4096.~~ **(closed 2026-07-22.)** The adjacency is now stored as its nonzeros
(`normalized_adjacency_sparse`, `src/gnn/graph.rs:178`; `SparseMatrix`,
`src/gnn/sparse.rs:14`) and `try_forward_graph` multiplies that
(`src/gnn/gcn.rs:45`). One propagation, same instrument as before
(`gnn_train_scale`), both arms run back to back in one session so they saw the
same machine: **5.4s → 3.9s at N=1024, 20.5s → 7.4s at 2048, 73.4s → 11.6s at
4096** — 6.3x at the size the item was written around. The resident adjacency
falls from 134 MB to 0.33 MB.

Those were taken under load average ~9 on 8 cores, and the ratio is
load-dependent in a way worth writing down, because the two arms no longer have
the same bottleneck. Dense is bandwidth-bound on a 134 MB matrix and barely
notices contention — its 73.4s here against 79.7s recorded on a quieter machine.
Sparse is CPU-bound on the linear layers and does notice: the same after-numbers
on an idle machine are **1.6s / 3.3s / 6.6s**, i.e. 12.1x at N=4096. The 6.3x is
the honest floor; 12.1x is what an unloaded daemon gets.

The item's own diagnosis was directionally right and specifically wrong, which
is why it was measured first (`gnn_cost_breakdown`, `tests/gnn_scale.rs`). It
blamed *materialising* the dense matrix. Materialising was the **smallest** of
the three dense costs at N=4096: 11.1% of the propagation, against 65.5% for the
multiply and 12.8% for the per-backward `transpose` the item never mentions.
Together 89.4%, so the remedy was right; had anyone optimised only the named
term, 79.7s would have become 71s.

**Bit-identical, and that is the recall gate.** The aggregation is the only
computation that changed, and `sparse_and_dense_products_are_bit_identical`
(`src/gnn/sparse.rs`) asserts over `to_bits()`, not a tolerance, that the sparse
and dense products agree exactly — in both orientations, at widths 1/5/17/384,
on a degree-2 ring, a complete graph and a graph with a zero-degree sink. Two
properties carry it: the entry is the same `1.0 / (sqrt(di) * sqrt(dj))`
expression over a degree the dense form reaches by summing that many exact
`1.0`s, and columns ascend inside a row, so the sparse product visits the same
nonzeros in the same order the dense one does. The terms it skips are exactly
the stored zeros, and `x + 0.0 * b == x` for a `+0.0`-seeded accumulator and
finite `b` — item 26's argument, same shape. Identical inputs through identical
downstream code give identical outputs, so `gnn_vector` is unchanged to the last
bit and ranking cannot move.

The e2e recall harness is green and unchanged (0.9306 / 0.9722 / 0.9471 before
and after) but is **not** evidence here: `test_recall.py` drives the CLI, its
corpus is 36 facts and `min_thoughts` is 128, so no propagation runs in it at
all. *Restated 2026-07-22:* the sentence here used to end "the recall gate the
previous author asked for does not exist at that corpus size", and item 97 built
it — `e2e/test_gnn_recall.py` lowers `min_thoughts` to 4, waits for the daemon's
own `learned propagation applied` line, and scores recall only after one
arrives. So a GNN recall gate now exists and would have covered this change; it
still measures a 36-node graph, so it is a wiring gate, not a scale gate.

~~Also unaddressed, and now the whole of this item: `gnn_train_refused` reaches
MCP health only (`src/mcp.rs:146`) — the RPC `HealthRes` does not carry the
field, so no CLI can see it.~~ **(closed 2026-07-22.)** Corrected before it was
fixed: this said `kern status`, and that is the wrong command.
`src/commands/status.rs:1-6` says what it is for in its own first line — it
describes the *processes* around the graph, `kern health` describes the graph —
so the counter belonged to `cmd_health` (`src/commands/admin.rs:38`). Both
lacked it, so the defect was real either way; only the target moved.

Three edits and no new plumbing. `HealthRes` took a `#[serde(default)]
gnn_train_refused: u64` (`src/trnsprt/src/kern_rpc/dto.rs:71`) — append-only,
the shape every other counter took, and `dto_serde_tests` feeds it a literal
old-daemon payload to prove a new client against an old daemon reads 0 rather
than erroring. `KernRpcHandler::health` (`src/rpc/kern_rpc_server.rs:112`) fills
it from `u64_at("gnn_train_refused")`, reading the *same* `tool_health` JSON the
MCP surface emits so the two cannot drift. And `tick_health_lines`
(`src/commands/admin.rs:179`) folds it into the existing `degraded:` line.

**Why it folds in rather than joining the fail-open line above it.** `cmd_health`
already prints a `degraded:` line for the seven fail-open counters
(`src/commands/admin.rs:156`), and that is where an eighth looks like it belongs.
It cannot go there. That line is built from `graph_health_stats`
(`src/base/health.rs:43`), which the CLI computes *in its own process*, while
`TRAIN_REFUSED` is a global only the daemon ever moves — a CLI reading it
locally sees 0 forever. The only counters a CLI can see are the ones that
crossed the RPC inside `HealthRes`, and the tick line is the only
`HealthRes`-derived degradation line there is. Folding into it also keeps
`a_clean_daemon_prints_no_last_fault_lines` true unedited: a healthy tick still
prints exactly two lines, so a quiet kern does not grow a third that always
reads zero.

Two mutations were run rather than argued. Hardcoding the handler field to `0`
fails exactly `a_refused_gnn_training_reaches_the_rpc_health_surface` and
nothing else — that test spawns a real `Trainer` whose runner blocks on a
channel, submits distinct kern ids until `Submit::Refused`, and asserts the RPC
surface reports the count the trainer actually holds. Gating the new segment on
`task_panics > 0` fails exactly
`a_refused_gnn_training_shows_with_no_other_counter_moving` and — this is the
point — leaves `a_clean_daemon_prints_no_last_fault_lines` green, because an
all-zero `HealthRes` cannot tell a printed zero from a suppressed one.

**The real `Trainer` cost something, recorded because it nearly shipped red.**
`TRAIN_REFUSED` is one global per process, and CI runs `cargo test --workspace`
(`.github/workflows/ci.yml:95`) — one process for the whole suite — where `just
test` runs `cargo nextest`, one process per test. The new test refuses a full
cap's worth of submissions, and the trainer's own
`a_backlog_past_the_cap_is_refused_and_counted_not_grown` asserts its delta is
exactly 1, so the two raced. Measured: **5 red runs in 30 under `cargo test`, 0
in 40 under nextest** — which is precisely why `just test` never saw it, and why
a green `just test` is not evidence about a global. Both tests now hold
`REFUSAL_COUNTER` (`src/tick/trainer.rs:43`) across the window in which they
measure; 40 of 40 green after. The rule this leaves: a test that *moves* a
process-global must serialise against every test that *measures* one, because a
measurement is two reads and the gap between them belongs to whoever else is
running.

Two consequences left deliberately unbuilt. The dense `normalized_adjacency`
(`src/gnn/graph.rs:134`) is no longer on any production path; it is kept as the
reference the equivalence test compares against, because a reference that lives
in the test can drift from the thing that shipped and this one cannot. And the
trainer is one `std::thread` rather than a pool specifically because "each
training allocates a dense `num_entities^2` adjacency — 134MB at N=4096" — that
reason is now false, so the concurrency choice is re-openable on its own merits.

### 29. Spilling all three indexes was measured and refused; it costs 122 MB more `[retrieval]`

~~DiskANN spill is entity-index-only, so the memory ceiling is pushed back, not
removed.~~ **The premise is true and the remedy is refused, measured 2026-07-21.**
The two indexes are exactly where the item said: `rebuild_index` hardcodes
`gnn_entity_idx` and `reason_idx` to `VectorBackend::resident(...)`
(`src/base/graph.rs:289-290`) while only `entity_idx` takes the spill branch
(`:296-297`). What was never checked is whether spilling them would help. It
would not; it costs 122 MB.

The instrument is `tests/spill_memory.rs` — one process per configuration
(glibc does not return HNSW's many ~1.5 KB vector allocations, so a
free-direction reading inside one process is a lie), RSS from
`/proc/self/statm`, and **two readings: cold, and hot after 200 searches.** The
hot one is the honest one, because mmap pages are only resident once touched.
50k entities at dim 384, each with `vector` AND `gnn_vector`
(`src/base/types.rs:303-304`), plus 25k reasons with vectors (`:450`):

| configuration | cold MB | **hot MB** |
| --- | --- | --- |
| kern map alone, no indexes | 260.7 | 260.7 |
| + entity index (resident) | 358.1 | 358.1 |
| + GNN index (resident) | 358.5 | 358.5 |
| + reason index (resident) | 309.6 | 309.6 |
| all three resident — today's default | 510.3 | 510.3 |
| entity spilled — today's `disk_threshold` path | 438.9 | **512.1** |
| all three spilled — what this item asked for | 449.2 | **632.3** |

Three findings, in the order they overturn the item:

1. **Spilling frees nothing under load.** It looks like 71.4 MB (510.3 → 438.9)
   until a query touches the snapshot; then it is 512.1, *above* never spilling.
   The vectors mmap is faulted straight back in, and `DiskIndex` additionally
   keeps `ids: Vec<String>` resident (`src/base/diskann.rs:304`). What spill
   actually changes is the *kind* of memory: ~97 MB of unreclaimable heap becomes
   clean, file-backed, reclaimable page cache. That is worth having under memory
   pressure. It is not a ceiling moving, and `decisions/diskann-spill.mdx:48`
   claiming heap drops "by the full vector set" was corrected in the same change.
2. **Doing what this item asked makes it worse by 122 MB** (632.3 vs 510.3 hot).
   Three `ids` vectors instead of one, three adjacency mmaps walked at open, three
   builds' arenas retained. Priced by the `spilled_all` mode without shipping it.
3. **The largest resident holder is not an index and never was.** The kern map is
   260.7 MB of the 512.1 — 51% — because every vector is stored *twice*: once in
   `Kern::entities`/`reasons`, once in the index that points at it
   (`HnswNode::vec`, `src/base/hnsw.rs:14`). Spill relocates one of the two
   copies. Nothing removes either. Halving that needs shared ownership of the
   vector between the kern map and the index, which is a type change across
   ~20 write sites, not an index-backend swap.

**Closed as written.** No index-spilling code shipped. What the pass did ship is
the defect it found on the way: `build_and_save` was **not reproducible** despite
a seeded RNG and a comment claiming it was — two hashed containers reached the
adjacency, and the `sort_by` that ranks candidates is stable, so every tied
cosine distance broke in per-process hash order. Same corpus, different index,
every process. Both now `collect` into a `BTreeSet` (`src/base/diskann.rs:123`,
`:180`); guarded by
`the_same_corpus_builds_a_byte_identical_index` (22740/76800 adjacency bytes
differ when the first is reverted, 446/76800 when the second is) and by
`tests/spill_transparency.rs`, which also records what spilling costs in recall:
resident 1.0000 vs spilled 0.9940 against brute force, overlap 0.9940. Spill is
therefore **not** answer-preserving — it swaps one approximation for another, and
identical answers were never on offer.

Still open, and belonging to item 83 rather than here: nothing bounds the
resident set, `disk_threshold` defaults to `KERN_CAP_DISABLED`
(`src/config/graph.rs:20`) with no auto-tuning and no signal on approach, and the
double-storage in finding 3 is the actual O(N) term.

### 95. Every ingest entrance now clamps — closed 2026-07-22 `[ingest]`

~~**A raw 1.0 from the watcher.**~~ **Done 2026-07-22.** Confirmed before fixing:
the sink submitted `1.0`, reaching `beta_params_from_confidence` unclamped and
landing on Beta(2,1) = 0.6667 — exactly a human CLI claim's posterior, and above
the 0.6500 (Beta(1.95,1.05)) a deliberate MCP agent assertion gets. A file
appearing on disk outranked an agent that asserted something on purpose.

**The bypass was wider than the title.** `intake.rs`'s `drain_document` minted a
raw `1.0` for a `Source::File` `Document` too, through `run` rather than
`submit`. Clamping inside `Worker::submit` alone — the shape this item proposed —
would have closed one of two live holes and left the other, which is the same
"a convention each caller remembers" failure one method further down.

So the guard went one level lower: `Worker`'s private `job()`
(`src/ingest/worker.rs:40`) is now the ONLY place a `Job` is built — `run_with_acl`
no longer assembles one by hand — and it clamps. Every entrance
(`enqueue`, `enqueue_with_acl`, `submit`, `run`, `run_with_acl`) takes a
`source_tag` it cannot omit, so a future producer is asked "who is asserting
this?" by the compiler rather than by a convention. The clamp takes the
confidence only; `kind` stays the producer's, or a watched file would be
reclassified from `Document` to `Claim`.

**The `tag` is the channel, `source.scheme()`** — `"file"` for the watcher
(`src/ingest/file_watcher.rs:95`) and for the intake drain. Not `USER_SOURCE`:
no human asserted it. Not `AGENT_SOURCE`: an agent's ceiling belongs to a
deliberate assertion by a non-human principal, and a file changing on disk is
not an assertion at all. No new `"watcher"` constant either — `clamp_confidence`
only separates `USER_SOURCE` from everything else, so a new tag would be a
second name for the same 0.95 ceiling, exactly the label-that-weights-nothing
item 20 refused. `scheme()` is already what `RetrievalConfig::source_trust`
keys on, so `source_trust = { file = ... }` is the lever that actually separates
watcher from agent. The two paths that DO know their principal name it
explicitly, because `Source` cannot record an author (item 20's open blocker):
the CLI passes `USER_SOURCE`, MCP and the direct-intake replay pass
`AGENT_SOURCE`.

**Ranking moved for `Document`s, and not at all for the recall corpus.** A
watcher or intake `Document` drops 0.6667 → 0.6500; `e2e` recall is unchanged at
0.9306 / 0.9722 / 0.9471, bit-identical including the worst-probe list, because
that corpus is ingested through `kern ingest` — the one path that still mints
1.0. The recorded baseline therefore stands as measured; nothing to update.

Deciding behavior: fix-the-root.

### 30. The durable backstop landed; what is left is a distill ceiling nobody chose and a queue that does not report its depth `[ingest]`

~~`Worker::enqueue` fires `tokio::spawn(async move { tx.send(job).await })` and
returns immediately. The channel bound is 64; the spawn set is unbounded.~~
**(closed 2026-07-21.)** What it actually did was measured before it was
changed: not a silent drop but unbounded growth. `tokio`'s `send` only errors on
a closed channel, so every detached task *parked* on a full queue holding its
whole text — 500 offered to a stalled worker, 500 accepted, nothing refused, and
that is the failure the new test reproduces when the bound is removed. Now
`QUEUE_CAP` is the whole bound (`src/ingest/worker.rs:79`): `try_send` refuses
the newest job rather than detaching (`:158`), the refusal is counted
(`:163`) and reaches every health surface as `ingest_queue_refused`
(`src/base/health.rs:85`, `src/mcp.rs:145`, `src/commands/admin.rs:163`). The
one producer that must not be refused waits instead — `submit` awaits capacity
(`src/ingest/worker.rs:182`) and the file-watcher sink still calls it
(`src/ingest/file_watcher.rs:129`) — now as the fail-open fallback behind item
30's durable backstop rather than as the only path it has; the MCP RAM-queue
fallback gets a `tool_error` (`src/mcp/tools_mutate.rs:338`). Still distinct
from the *tick* queue, which is bounded at 512 (`FEATURES.md:437-438`).

**Corrected 2026-07-22 — the "no timeout budget" half was never true, and the
grep that established it looked in the wrong files.** The claim was "no
`timeout` in `src/ingest/distill.rs` or `src/ingest/worker.rs`", and there is
none in either — because the bound lives at the client, not at the caller.
`distill` takes an opaque `llm: &dyn Fn(&str) -> String`
(`src/ingest/distill.rs:37`) which is always `Client::complete_func`
(`src/llm.rs:300`), and `complete` posts under `LLM_TIMEOUT` = 600s
(`src/llm.rs:396`, applied at `:267` and `:289` — both the native and the
OpenAI-compat branch), over a client-wide 120s default and a 3s
`connect_timeout` (`:96`, `:99`). So a hung LLM does **not** hold the one
in-flight slot forever; it holds it for at most ten minutes. What is actually
open is that 600s is a ceiling nobody chose for *this* leg and nothing exposes
it: it is a `const`, not config, and a stall that long is indistinguishable at
every health surface from a slow model. Narrower than stated, and a tuning
question rather than a liveness defect.

**The durability half is closed 2026-07-22.** `KernFileWatcherSink::ingest` now
writes a `DirectJob` through `intake_direct` first
(`src/ingest/file_watcher.rs:104`, tmp + rename at `src/ingest/direct.rs:42`) and
falls through to `Worker::submit` (`src/ingest/file_watcher.rs:129`) only when
that write fails — the shape `tool_ingest` already had
(`src/mcp/tools_mutate.rs:300`). It is gated on `intake.enabled` alone
(`src/commands.rs:974`), which is exactly the flag
`spawn_intake` gates on, so the directory is never written unless something
drains it; `drain_direct_once` needs no reason LLM, so the stricter gate
`tool_ingest` uses would have parked nothing on a reason-less host that drains
fine. `notify` installs its watches and reports nothing that happened before
(`FileWatcher::new`, `src/watcher/src/watcher.rs:36`) and there is no startup
scan, so before this a watcher record still in the channel when the daemon died
was lost with nothing to re-offer it.

`DirectJob` grew a `source_tag` (`src/ingest/direct.rs:35`) to make that hop
safe: `drain_direct_once` used to name `AGENT_SOURCE` for every payload it read
(now `:106`), because every payload there was minted by the MCP tool — routing
the watcher through it unchanged would have relabelled `"file"` as `"agent"` and
undone item 95's "the tag is the channel". Old payloads keep the old behaviour
through `#[serde(default)]` (`:38`). The relabel is numerically invisible —
`clamp_confidence` separates `USER_SOURCE` and nothing else — but it is the key
`source_trust` weights on (item 20).

**The fix had a self-referential edge, found and closed in the same change.**
The default watched root is the cwd and the default intake is `.kern/intake`
under it, so parking a record durably wrote a file into the tree that produced
it: the watcher read it back, parked a payload wrapping that payload, and
repeated. Measured against the default config from one seed edit: **283 payloads
in 60 seconds, largest 1.77 MB, versus 0 on the pre-change build** — each one an
embed call and a graph write. `IgnoreRules` only ever hardcoded `.git`
(`src/watcher/src/ignore_rules.rs:60`), so nothing stopped it. Closed by giving
`IgnoreRules` host-supplied denied prefixes (`:45`, matched `:63`) and passing
the resolved `intake.dir` and `data_dir` (`src/commands.rs:967`) — named by the
host, because that crate must not know what kern is. `effective_roots` now pins a
relative root to `cwd` (`src/config/watcher.rs:25`) so event paths and denied
prefixes share one coordinate system. What is *not* closed is that the deny list
is by name rather than by construction — item 99.

The queue-depth half is
**narrowed 2026-07-21** — closing item 8 gave `kern intake` a
`pending=/stuck=/failed=/done=` readout (`src/commands/intake_cmd.rs:43-45`),
so the file-backed queue reports its depth; the in-process `Worker` channel
still does not. "The only LLM call on the path"
(`concepts/acceptance.mdx:7` — the old citation `` `:189-192` `` was past the
end of an 86-line page and quoted a sentence that is nowhere in the tree), and
with the
answer leg removed (2026-07-21) the distill leg is still the only LLM on any
path — no latency work has landed on it.

### 31. Structural debt in the hot types `[retrieval]`

Both routing bullets are retired; what remains is serialization and index shape.

Recorded in `FEATURES.md` gap blocks, planned nowhere:

- ~~Routing does a vector lookup per level, O(depth·log n), and unnamed children
  are unbounded per parent~~ **(retired 2026-07-21 — verified false on both
  counts; the FEATURES gap block it quoted is corrected at `FEATURES.md:126`).**
  ~~Per-parent fan-out is a real cliff and stays on this item's list~~
  **(measured and retired 2026-07-22 — the width is real and unbounded, but it
  costs a linear ~2% at the widths the graph reaches, and the scan the wording
  blames is not where even that goes).** The location holds:
  `route_to_child_id` (`src/base/accept.rs:882`) is a linear scan over the
  parent's loaded named children against each child's stored `graviton_vec`,
  reached from `route_entity` (`src/base/accept.rs:220`) once per accepted
  entity — per distilled claim and per chunk (`src/ingest/place.rs:133`,
  `src/ingest/place.rs:212`) — descending up to `MAX_ACCEPT_DEPTH`, in practice
  two levels. Unnamed children are capped at one per parent on the routing path
  by `get_or_spawn_unnamed_child` (`src/base/accept.rs:644`, guarded by
  `src/base/accept.rs:934`); only tick clustering makes more, one per spawnable
  cluster and deliberately (`src/tick.rs:196`).

  Instrument: `tests/route_fanout.rs`, release, `--ignored`. Fan-out costs
  **0.14-0.18us per child** across runs, of an accept that costs **1.4-2.1ms**
  at 20k entities — 0.5% at width 64, ~5% at 512, ~24% at 4096. The accept is
  dominated by the two HNSW searches it runs (the dedup gate at
  `src/base/accept.rs:39`, the similarity reason at `src/base/accept.rs:316`)
  plus two index inserts, which is why width has to reach the thousands before
  it registers. A named/unnamed A/B over the same walk — identical descent,
  cosine skipped — attributes **-0.009, -0.001 and +0.003us per child** to the
  `graviton_vec` comparison on three runs: zero every time. What the width actually buys is two
  `Vec<String>` clones per descent (`src/base/accept.rs:218`,
  `src/base/accept.rs:683`) and a linear resident-map probe for the generic
  child; the comparison the bullet named is free.

  Nothing caps named children per parent, and unlike the other two claims this
  one survives measurement. Only two things create a child with a routable
  `graviton_vec`: `add_graviton_with_mass` (`src/base/accept.rs:756`),
  human-declared and root-only, and tick naming (`src/tick/tasks.rs:239`), whose
  result `promote_to_root_if_generic` (`src/base/accept.rs:821`) lifts to root.
  Driving that real accept → cluster → name → promote loop, root fan-out tracks
  distinct cohesive topics very nearly 1:1 — 8 topics -> 8 children, 64 -> 55,
  256 -> 191. `GRAVITON_DEDUP_THRESHOLD` collapses only topics whose graviton
  names embed within 0.85 of each other, which is a fact about the corpus, not a
  bound on the structure. Disabling promotion does not shrink the width, it
  relocates it: `generic` then holds all 191 and routing scans them one level
  deeper.

  So the width is real and the cost of it is linear — ~2% of an ingest at 191
  children, and it needs some thousands of distinct cohesive topics before it is
  a fifth. That is a slope, not a cliff, and no optimisation is shipped for it.
  Recording the lever in case the slope ever matters: it is the `Vec<String>`
  clone, not an index over children. The clone in `route_entity` exists only to
  end a borrow and can be replaced by holding `&kern.children` alongside the
  `&GraphGnn` the scan already takes; an index would be write-path work on every
  spawn and every rename, buying back a comparison that measures as free.
- `Entity` is a ~30-field flat struct (serialization cost on every store round
  trip) and `Kern` carries no per-kern stats — mean heat, fill ratio — that
  clustering could reuse (`FEATURES.md:90-92`).
- DiskANN is build-once; the lexical index is RAM-only (`FEATURES.md:251-252`).
- LMDB compaction is manual and offline-only, and is the only way to shrink the
  high-water mark (`FEATURES.md:322-324`).

### 32. Tree depth was an eviction bias in the opposite direction — closed 2026-07-21 `[lifecycle]`

**Closed 2026-07-21. The bias was real; its direction and its severity were
both stated backwards, and the fix the title implies would have made it worse.**

Two corrections to the item as written. The reach is **5 levels, not ~4**:
strength starts at 1.0, halves per level (`PULSE_DECAY`,
`src/base/constants.rs:55`) and the walk returns below 0.05 (`PULSE_THRESHOLD`,
`:56`), so depths 0–4 were all deposited on. And "invisible to any metric that
does not exist yet (item 1)" was stale — item 1 is closed, the harness exists,
and this was measurable the whole time. It is now measured:
`tests/depth_bias.rs` runs the real `pulse` → `commit_access_ids` → `run_gc`
lifecycle over simulated months, two cohorts per depth with identical usage.

The measurement inverted the item. Deep entities were not decaying unfairly —
they were decaying *correctly*. Shallow ones could not be collected at all. The
pulse deposit recurred every 60s, so it never evaporated: equilibrium heat at
depth 4 was 1939 against a `COLD_HEAT_THRESHOLD` of 0.01, and at depth 0 it was
31 031. Anything within 4 levels of the root was permanently exempt from cold
GC, whether or not it had ever been read — which is the vision test "the hot
graph stays bounded" failing, not merely a fairness question. Any recurring
deposit above ~1.6e-7 produces that exemption, so no tuning of the deposit size
could have preserved the pheromone story; propagating *deeper*, which the title
implies, would have extended the exemption to the whole graph.

So the deposit is gone (`src/tick/pulse.rs`), along with
`HeatConfig::deposit_traversal`. Access is the only deposit; the pulse still
fans clustering, GC, reembed and idle-sweep out from the root, so an idle daemon
still maintains itself. Post-fix, every depth 0–7 evicts its unused cohort on
the same day and keeps its used cohort. What this did **not** buy: item 83.
Eviction now fires where it never could, but `max_kerns` and `disk_threshold`
are still `usize::MAX` and there is still no per-kern entity cap, so nothing
bounds the graph *deterministically* — only usage does.

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
`src/commands.rs:1098` and the question path issues it — but it is single-id, not a
catch-up mechanism.) Two pieces adopted on paper and unscheduled: **back-off
pacing** with exponential jitter keyed to a divergence estimate
(`docs/kern/fl-vs-knids-federation.md:163-168`), and **batch-size / push-vs-pull
tuning** at scale (`howto/memory-bank.mdx:149-150`) — the top-32 is hard-coded and
the push-only choice was never revisited.

### 37. Backpressure, divergence metric, and delta write-lock starvation `[federation]`

The only per-origin budget is the `Question` one item 34 records
(`src/gossip/handler.rs:318`, 30/min); the `Delta` path — the one that takes the
write lock — has none. `HealthStats` has no divergence field
(`src/base/health.rs:8-38`). Sharper than previously recorded: the four
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
`docs/kern/pagerank-authority.md:164, :202-235, :274` and was never
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
(`src/retrieval/score.rs:107-117`), not by exclusion. Decide whether `remote-*`
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

- **Tombstone and LWW-history growth is unbounded** (`:278-281`); the note's own
  follow-up was "time-bounded compaction".
- **Vector LWW is coarse across heterogeneous embedding models** (`:284-286`),
  and `docs/kern/fl-vs-knids-federation.md:200-204` explicitly *allows*
  per-node model choice. Item 3 covers the local swap; the federated case — no
  model-identity stamp on the wire — is separate and unfunded.

### 44. Bi-temporal stamps are never federated `[federation]`

`valid_from` / `valid_to` / `invalidated_at` are `#[serde(skip)]`
(`src/base/types.rs:311-316`), so each node re-derives its own `as_of` view and
two *converged* nodes can answer the same point-in-time query differently
(`docs/kern/crdts-federation.md:54-62`). The federated twin of item 4.

### 45. Multicast discovery is unreliable with no health signal `[federation]`

Wireless APs, container bridges and VPN interfaces all break it, with no
fallback and no way to distinguish discovery-failed from no-peers-present
(`concepts/federation.mdx:68-70`).

### 46. One fresh TCP connection per gossip message `[federation]`

`TcpStream::connect` per call at `src/gossip/transport.rs:37` (`send_msg`) and
`:45` (`send_and_receive`). No pooling. Separately, the `trnsprt` client has no
pooling either (`FEATURES.md:976-977`) — that one is not gossip and is not gated
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
(`FEATURES.md:1082-1083`).

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
per-kind (`FEATURES.md:419`).

### 49. The distill prompt is one-shot and global `[ingest]`

One `format!` over the whole conversation, no per-kind branch, no chunking
(`src/ingest/distill.rs:34-53`). The `kind` taxonomy has overlapping categories
(decision/project, fact/code-fact) and label accuracy was measured at ~33% even
at 7B — **that figure came from the deleted harness and is unreproducible; treat
it as a lead, not a number** (item 1's claim standard). Long deltas are not
chunked at all.

### 50. Intake distillation lacks relative-date resolution `[ingest]`

The prompt injects no current date (`src/ingest/distill.rs:43-48`), and
`valid_from` is only requested when the statement states an absolute date — so
dropped text containing "last Tuesday" stores unresolved. The eval path got this
and the product path never did; the eval path is now deleted, so the capability
exists nowhere.

### 90. `DirectJob` carries `valid_until` but drops `valid_from` `[ingest]`

The durable direct intake serializes one bi-temporal stamp and not the other:
`DirectJob` (`src/ingest/direct.rs:11-36`) has a `valid_until` and no
`valid_from`, and `drain_direct_once` overlays only the former onto the drain
loop's `Config`, so `valid_from` is whatever the loop's shared config says —
always `None`. **Not a live loss**: the only producer of `valid_from` is the
distillation path (`src/ingest/intake.rs:193`, from `distill.rs`), which calls the worker
directly and never goes through `direct/`, and the MCP `ingest` schema has no
`valid_from` field to lose. It is a hole that opens the moment either of those
changes — which item 50 would do. (The retired item 89 did not: it gave the
drain loop's shared `Config` a standing `valid_until` and left `valid_from`
alone.) Ranks here, next to 50, for that reason and not for any damage it does
today.

### 51. Require reason text on supersede `[ingest]`

`ReasonKind::Supersedes` edges are minted at `src/base/accept.rs:543` and `:638`
with `fallback_label()` text (`src/base/types.rs:116`), never a caller-supplied
rationale. The *why* is the thing the graph exists to hold.

### 52. A single-line graviton seed still truncates at the embed context window `[ingest]`

**Narrowed 2026-07-21 — the old wording is retired.** It said "acknowledged in
source at `src/mcp/tools_admin.rs:116`" with "chunk + mean-pool" as the unbuilt
upgrade path. Both halves moved in `08c9971`: the acknowledgement comment was
deleted, and chunk + mean-pool **shipped** for the multi-line case —
`seed_examples` (`src/base/accept.rs:717-729`) splits a seed on newlines and
`mean_pool` (`:733`) averages the per-line embeddings, wired at
`src/mcp/tools_admin.rs:119-136`. Line 116 now carries the mean-pool rationale,
i.e. the opposite of what it was cited for.

What is left is the case `seed_examples` deliberately does not split: a seed
whose `lines` len is under 2 is embedded whole (`src/base/accept.rs:724-726` —
spelled in full because the nearest preceding path is `tools_admin.rs`, which is
455 lines long, and a bare `:NNN` continues the wrong file), so one long
paragraph still goes to the model as a single call and truncates past its
context window with no signal. Chunking *that* wants a length-based split, not a
newline one, and is still blocked on a real document long enough to truncate.

### 53. Clustering is vector-only `[lifecycle]`

No semantic or structural features (`FEATURES.md:502`), and naming plus
enrich are a cold LLM call per kern. The adopted-but-unbuilt upgrade is
thought-level PageRank feeding the split heuristic — high-rank nodes become
gravitons, bridge nodes become sub-kerns
(`docs/kern/pagerank-authority.md:258-263`, `decisions/pagerank-authority.mdx:120-121`).
Graph structure informs ranking today and never informs the tree shape that
routing depends on.

### 54. GC has no convergence gate `[lifecycle]`

The adopted loop-closing design gated forgetting on convergence — `G ≥ 0.6`
**and** heat below floor for `forget_ttl`
(`docs/kern/stigmergy-self-improving.md:210, :236`). Shipped GC has no gate at all.
Depends on item 62 (the convergence metric) existing.

### 55. Two freshness signals, different half-lives, neither ever tuned `[retrieval]`

A 24-hour one for ranking (`qbst_recency_half_life_secs`,
`src/config/retrieval.rs:31`, defaulted from `QBST_RECENCY_HALF_LIFE`,
`src/base/constants.rs:12`) and the retention one on `HeatConfig`. The offline
NDCG sweep meant to tune either was never run
(`decisions/stigmergy-over-gardening.mdx:117`). Third input nobody reconciled:
`docs/kern/stigmergy-self-improving.md:160-170` derives a 1–2 day half-life.

**Restated 2026-07-21 — the old "7-day retention" wording was stale.** The 7 days
at `src/base/heat.rs:17` is the struct default and is never what runs:
`Config::load` applies the preset unconditionally (`src/config/mod.rs:104`,
`:151`) and `Preset::apply` is the only writer of `heat.half_life_secs`
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

There is no `Contradicts` reason kind (`src/base/types.rs:90-99`) and no `stance`
parameter on the ingest schema (`src/mcp/tools_mutate.rs:19-33`);
`observe_contradict` (`src/base/types.rs:434`) has exactly one caller, GNN
alignment (`src/tick/gnn_propagate.rs:163`). Observer-reputation weighting is
also unbuilt.

### 57. No evidence decay `[lifecycle]`

`conf_alpha` and `conf_beta` only grow — the sole zeroing is the remote strip
(`src/base/merge.rs:28-29`) — so stale consensus takes proportionally many new
observations to unseat. Tick-based γ damping is an open design
(`decisions/bayesian-confidence.mdx:137`).

### 58. Supersede chains are unbounded while contested `[lifecycle]`

No `ReasonKind::Edit` rationale edge (`src/base/types.rs:90-99`) and no producer
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
tool exposes the supersede chain beyond `include_history` (`FEATURES.md:179-180`).
Two open questions beside it, from `docs/kern/bayesian-belief.md:159-162`: should
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

### 94. A near-duplicate's alternate wording is indexed and findable — closed 2026-07-22 `[retrieval]`

The premise held. Confirmed by measurement before any fix: a dedup of
`"alpha beta gamma"` and `"alpha beta gamma, restated"` leaves
`rephrases=[("alpha beta gamma, restated", 0)]` on the survivor —
stored, `vector.len() == 0` — with `reason_idx` empty and the lexical index
answering `[]` for `restated` while it answers the survivor for `alpha`. The
failing query that proves it is real: over a 21-entity corpus where 20 fillers
sit nearer the query vector, `velocipede outbuilding` — the merged-away
document's own words — returned twenty fillers and never the survivor.

**The fix the item proposed would have been wrong.** `LexicalIndex` is keyed by
entity id and `inner_insert` removes before it inserts, so "one `lex.insert` of
the rephrase text against the survivor's id" would have *replaced* the
survivor's own posting with the alternate's. Shipped instead: one lexical
document per entity, `entity_document` (`src/base/lexical.rs:15`), being the
entity's statements followed by every `Rephrase` text hanging off it. One
document per id is also the answer to "does the entity appear twice" — BM25
cannot return one id twice, so no dedup logic is needed at the seed layer.

Lexical only; **no dense vector for the alternate**. `merge_duplicate` is a pure
graph function reached from both gates and neither has an embedder in hand, so a
vector would make dedup an I/O operation on the write path, and item 83 already
names vectors as the largest single holder. It would also buy nothing on the
default path: only `Mode::Reason` seeds off `reason_idx` (`seed_by_reason`);
`Mode::Hybrid` never reads it. And the dense gap is the smaller one by
construction — the two texts merged *because* their vectors were within
`INGEST_DEDUP_THRESHOLD`, so the survivor's own vector already stands in for the
alternate. The gap was purely lexical: exact rare terms.

Lifecycle, so the wording cannot outlive its survivor: `reindex_entity`
(`src/base/lexical.rs:34`) re-derives the document from the graph and is called
at every site that mints or drops a `Rephrase` — `merge_duplicate`
(`src/base/accept.rs:199`), the supersede path that consumes one
(`src/tick/tasks.rs:209`), and `degrade_entity_reasons`
(`src/commands/graph_ops.rs:491`). GC needed nothing: `remove_entity` already
calls `lex.remove(id)`, and the wording lives in the survivor's own document.
`rebuild_from_graph` uses the same builder, or the posting would survive exactly
until the next reload.

Cost, named: the survivor's `doc_len` grows, so BM25 length normalization
(`b=0.75`) dilutes the primary wording's own terms a little. That is the trade
for the alternate being reachable at all.

**What it does not buy, and why the recall number did not move.** Recall is
unchanged at 0.9306 / 0.9722 / 0.9471 because the e2e corpus has no
near-duplicate pair, so no `Rephrase` is ever minted and every lexical document
is byte-identical to what it was. The corpus was deliberately left alone so the
baseline stays comparable. More importantly, a CLI-level probe cannot show this
today: `cmd_search` (`src/commands/query.rs:101`) is pure vector and never reads
the lexical index at all, and `kern query` runs `fuse_hybrid_seeds`, which
rescores every fused seed by `cosine(qvec, entity.vector)` — so a lexical-only
hit is re-ranked by exactly the signal that failed to find it and is cut
whenever the candidate pool overflows the delivery cap. The fix makes the
wording a *candidate* where it was not one; turning that into a delivered rank
is item 61's question, not this one.

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
(`docs/kern/stigmergy-self-improving.md:271`). Item 54 depends on this.

### 64. Normalize and re-found the scoring stack `[retrieval]`

Three that must be judged together, since each moves the others:
min-max normalize `apply_boosts`, which is purely additive and unnormalized today
(`score * confidence + boost + fact_bonus`, `src/retrieval/score.rs:90-102`); swap
the hand-rolled stemmer (`src/base/lexical.rs:244`, no stopword list, no
`rust-stemmers` in `Cargo.toml`) for `rust-stemmers` 1.2.0 + stopwords, which
needs a BM25 rebuild; and validate-or-remove GNN reranking, whose only expression
is the 0.6 blend in item 61.

### 65. Rank on the lower confidence bound `[retrieval]`

`p − k·√var` instead of the mean (`docs/kern/bayesian-belief.md:149`) — a
one-line ranking change that makes a single-observation claim stop outranking a
well-evidenced one at equal mean.

### 66. RRF weights and mode blends are configurable but never auto-tuned `[retrieval]`

Was two ceilings; the rerank half left with the rerank stage itself
(2026-07-21). What remains: RRF weights plus mode blends are configurable but
never auto-tuned (`FEATURES.md:216`).

### 67. Binary quantization stays non-user-selectable `[retrieval]`

Its recall floor is too low without a rescoring pass; deliberately excluded from
`parse` (`src/quant.rs:20-21`). Beside it: no int4 path and the quantization
scale is fixed at encode time (`FEATURES.md:271`).

### 69. Speculative decode for the distill leg `[ingest]`

qwen3.5:0.8b draft → 4b generator. With the answer leg gone (2026-07-21) the
only LLM latency that matters is distillation throughput; no `draft` or
`speculative` anywhere in `src/llm.rs`. Latency is the one axis item 1 does not
gate — the e2e harness can still judge this.

---

# Tier 9 — process, packaging, and things that rot unnoticed

Last because none of it affects a running kern. First within its tier because a
contract nobody enforces is a contract nobody has.

### 93. Line anchors cannot survive a merge, and `docs-check` cannot see it `[process]`

Every citation of the form `` `FEATURES.md:420-421` `` is a bet that nothing is
ever inserted above line 408. `FEATURES.md` only grows, so the bet loses on
every merge that appends — and when two branches each append and then combine,
it loses twice over. Four times on 2026-07-21: four anchors, then four more,
then twenty-seven, then fifteen. `scripts/docs_check.py` was green through all
of it, correctly — it verifies the line *exists*, and it always does. <!-- docs-check: anchor-ok -->

The repeated hand re-pointing is not the fix. It is a tax paid on every merge,
by whoever remembers, and the first time nobody remembers the docs quietly start
lying again. Two candidate closures:

- **Symbolic anchors.** Cite a heading or a distinctive phrase
  (`` `FEATURES.md#12-mcp-surface` ``) and let the checker resolve it. Immune to
  insertion; breaks loudly on rename, which is the right failure. **Still open** —
  this is the better answer and the larger change, and nothing below replaces it.
- **Content-checked anchors.** ~~Keep line numbers but have `docs_check.py`
  verify the target still relates to the citing sentence.~~ **Landed 2026-07-21.**

**What landed.** `scripts/docs_check.py` now compares the content words of the
citing block — the whole bullet or paragraph, since the docs wrap at eighty
columns — against the content words of the cited line(s), and nominates an anchor
whose target shares too few. Tokens are lowercased, split on `_` and on the
camelCase boundary, kept at four characters or more, and filtered through a small
stopword set. The bar depends on what is cited: two shared words for prose
targets, and for code targets only total silence counts, because prose citing
code shares almost nothing *by design* — the sentence explains, the line
implements. On the tree as it stands a two-word bar everywhere nominates 117 of
654 anchors and is unusable; the split bar nominates 39.

**It nominates, it does not gate.** Nominations print under their own heading and
leave the exit code alone; `python3 scripts/docs_check.py` still exits 0 with 38
standing. `--strict-anchors` makes them fatal, for a CI that has decided to trust
them — not yet. A nomination a human has adjudicated is silenced in place with a
trailing `docs-check: anchor-ok` comment, the same idiom as the historical page
marker, and the marker counts only outside backticks so a page may quote it. The
paragraph above is the first user of that escape: its `FEATURES.md:420-421` is an
illustration of the disease, not a citation, so it is acquitted rather than fixed
— and so is this sentence, which repeats it. <!-- docs-check: anchor-ok -->

**Measured false-positive rate: 13 of 39, about 33%.** Adjudicated one at a time
against the real tree, not estimated. The 26 true ones are the expected shape —
`FEATURES.md` citing `classify_prompt` at a `return Vec::new();`,
`bayesian-belief.md` citing `conf_alpha`/`conf_beta` at a `ChunkPart` struct,
`README.md` citing the cold-tier drop counter at a closing brace; five of the six
anchors in one `crdts-federation.md` status block are wrong. The 13 false ones
share one weakness: the anchor is right but the target's distinguishing word is
under four characters (`acl`, `rrf`, `run_hub`) or is a stem the prose inflects
(`fn stem` against "stemmer"). Two more are self-inflicted — this item quoting
`FEATURES.md:420-421` as an example rather than as a citation — and are acquitted
in place, leaving **38 standing, 12 of them false, 32%**. <!-- docs-check: anchor-ok -->

**One measured non-win, recorded rather than buried.** Splitting the camelCase
boundary looks like a strict improvement and is not: it silenced two false
positives and one *true* one. `bayesian-belief.md:16` cites `src/base/types.rs:66-75`
for "the seven kinds that exist" while 66-75 is `EntityStatus` and `ReasonKind`
starts at 76 — a real breakage that now matches on a stray "entity" and no longer
nominates. Precision moved 64% → 67% and recall moved 27 → 26. It is kept because
prose and code should tokenise alike, not because the numbers earned it.

A third of the output being wrong is why `--strict-anchors` is opt-in, and it is
the number to beat before CI adopts it: stemming and a three-character floor with
a longer stopword list are the obvious next moves, and symbolic anchors retire the
question entirely.

**Second pass, 2026-07-21 — the nominations were worked, and both next moves
landed.** All 38 were opened at the cited line and adjudicated one at a time: 27
real drift, 11 false. Every one of the 27 was re-pointed and the new target read
back before it was written down — 29 anchors moved in all, counting the two bare
`` `:533` ``/`` `:398` `` continuations the regex never sees. Three of the false
ones were acquitted in place; the other eight the tokeniser can now see for
itself. `tokens()` keeps three characters instead of four, runs a light suffix
stripper with consonant-undoubling so `stemmer` and `stemmers` both reach
`fn stem`, and carries a stopword list grown to match — articles and pronouns are
back in play at three characters, and Rust's boilerplate (`let`, `pub`, `self`,
`new`) is there because a match on it says only that the target is Rust.

**Measured on the real tree, before and after, against a hand-adjudicated truth
set.** Precision 27/38 = **71.1% → 26/29 = 89.7%**. False-positive rate **28.9% →
10.3%**. Recall, over the 28 breakages known to exist, **27/28 = 96.4% → 26/28 =
92.9%** — *it went down*. The three-character floor that acquits `acl`, `rrf` and
`hub` also acquits `gpu`, and `gpu` plus `kern` is enough to reach the two-word
prose bar and silence a genuinely under-covering anchor (this file citing
`FEATURES.md:598` for three claims that span 589-590). That is the same trade the
camelCase note above records, paid a second time, and it is recorded here for the
same reason. A prose bar of 1 instead of 2 measures 96.3% precision at unchanged
recall and was **declined**: the truth set holds only five prose-to-prose anchors,
which is not evidence, it is a sample small enough to fit anything.

**What remains.** Three false positives are structural, not tokenisation, and
carry the marker rather than a fix: `FEATURES.md:206` cites `if scored.len() < k {`
for cold backfill and shares nothing with it because the line is a bound check
whose meaning lives in the lines around it; this file's two retired notes cite a
`README.md` table row and a `FEATURES.md` bullet for the single word each was
retired over (`move`, `acl`), one word short of the prose bar. Two of the three
would need a *block*-shaped target, not a better tokeniser. And the checker still
cannot see the `bayesian-belief.md:16` class at all — a wrong range that overlaps
the right one by a word — which is why that one was fixed by hand here rather than
by the tool. So `--strict-anchors` exits 0 on this tree, but **10.3% is not near
zero**: adopting it in CI buys a gate that will demand a human acquittal on about
one nomination in ten and will still go quiet on an off-by-a-range anchor.
Symbolic anchors remain the better and larger answer, and nothing here replaces
them.

**Third pass, 2026-07-22 — the checker was measuring 63% of the anchors and
reporting on all of them.** Both earlier passes tuned the *content* rule and
neither asked what the scanner could see. `REF` demands a literal `src/` prefix,
so two forms were invisible to it: a bare `` `:NNN` `` continuation, and a bare
`` `place.rs:112` ``. The docs use both constantly — a bullet names
`` `src/base/store.rs:624` `` once and then cites `` `:636` ``, `` `:649` `` and
`` `:684` `` rather than repeating the path — and across the scanned pages they
were **245 of 664 line anchors, 37%**, carrying no existence check and no
content check at all. The `store.rs` anchor there is an illustration of the
form, not a citation of the function, so it is acquitted in place rather than
re-pointed — the same verdict the `FEATURES.md` example above carries, and the
reason the doubly-backticked escape covers only the two forms added here.
<!-- docs-check: anchor-ok -->
The second pass felt this without naming it: it re-pointed "29 anchors in
all, counting the two bare `` `:533` ``/`` `:398` `` continuations the regex never
sees", by hand, because the tool could not.

Both now resolve against the last file cited before them, which is how a reader
resolves them, and the scope resets at every heading — a section is where a
reader stops carrying context forward. A bare name with no antecedent falls back
to a unique match under `src/`; `types.rs` is four files and `graph.rs` is three,
so an ambiguous one resolves to nothing and is reported rather than guessed. A
doubly-backticked span is a quotation of the form, not a use of it, so the
paragraph above displays `` `:533` `` without citing it. References checked, on
the tree as this pass found it: **834 -> 1008**; it reads higher now only
because this entry cites five things of its own. The remainder of the 245 live
in `CHANGELOG.md`, which carries the historical marker and is skipped whole.

**It found a dead reference on its first run, and the shape of it is the
argument.** This file cited `Drop for LocalListener` at `` `:654` `` under a
paragraph whose last named file was `client_local.rs` — 146 lines long. The
line number was right and the *file* was wrong: `Drop for LocalListener` is
`src/trnsprt/src/typed/local.rs:654`. That is a failure mode a spelled-out path
does not have, and it is the one a continuation adds: existence stops being a
property of the anchor and becomes a property of the anchor plus everything
above it, so an edit that inserts an unrelated citation silently re-points every
continuation under it. Fixed here by spelling the path out, which is the only
fix that survives the next insertion.

**18 nominations, adjudicated one at a time against the real tree: 15 true, 3
false — 83.3% precision on a population that had never been checked.** Five of
the 15 are the wrong-file class the dead reference belongs to
(`` `:507` `` reaching `src/commands.rs` for a `bind_unix` that lives in
`src/trnsprt/`, `` `:129` `` reaching `direct.rs` for a `Worker::submit` in
`file_watcher.rs`, and the two Windows/PQ notes at `` `:136-137` `` and
`` `:132-134` `` that say "doc-only" while landing in `src/base/graph.rs`);
the other ten are ordinary rot, including
`FEATURES.md:55` citing `Acl { scope, users, groups }` at a `_ => None,` thirteen
lines above the struct. The three false ones are the known weaknesses, not new
ones: `gc` is two characters and falls under the token floor, so the `gc` row
citing `fn tool_gc` shares nothing; and two are historical quotations of an
anchor being *discussed*. **They are reported, not fixed** — every one is a
`[surface]`, `[retrieval]`, `[lifecycle]` or `[federation]` claim, and this pass
owns `[process]`.

The remaining blind spot is unchanged in kind and now smaller in reach: symbolic
anchors are still the answer, and a continuation is the strongest argument yet
for them, because it is a line number that does not even name its own file.

**Verified independently 2026-07-22, against the commit rather than a working
tree.** The third pass reconciled, implemented and recorded in one go, so no
adversary had read it. Re-run: `just docs-check` green with its selftest,
`just check` green, `just test` 39 passed and 4 skipped with both recall floors
printed and unmoved. The 834 -> 1008 comparison reproduces — the prior script
over the prior tree reports 834 and nominates nothing, this one reports 1008 and
the single dead reference. The four-loops-into-one-sweep refactor is
verdict-identical: with the two new patterns neutered, the sweep prints
byte-for-byte what the prior build printed, so it moved no failure and no
nomination.

Two corrections to the paragraphs above. **The wrong-file class is four, not
five** — the two anchors reaching the wrong `src/` file and the two doc-only
notes; the fifth candidate is the retired `` `:189-192` `` note, counted among
the false ones above, which is the same class seen from the other side. And
**`--strict-anchors` no longer exits 0 on this tree**: 18 nominations stand, so
it exits 1. The second-pass sentence saying otherwise was true when written and
is superseded now.

**Re-adjudication does not reproduce 83.3%, and the gap is a definition rather
than a mistake.** Twelve of the 18 are rot by any reading. The three in dispute
are the wrong-file class, and for those the anchor a human resolves is
*correct*: `` `:507` ``, under a paragraph about `bind_unix`, means
`src/trnsprt/src/typed/local.rs`, which is where `bind_unix` is, and `` `:129` ``,
under one about `Worker::submit`, means `src/ingest/file_watcher.rs`, which is
where the `submit` call is. Both hit the named symbol exactly. So the count is
15 if the question is "is this anchor under-specified" and 12 — 66.7% — if it is
"is the docs' information wrong". Either is defensible; the record should not
read as though only one measurement exists.

The distinction is load-bearing because the wrong-file class is the only one
that can turn the run *red*. It did exactly that once already, on this file's
own `Drop for LocalListener` note, and the fix was to edit the doc. Whenever an
inherited file is shorter than its inherited line, a page no human considers
wrong fails the check.

**One residual this pass opens and does not close.** Both new forms are matched
inside fenced code blocks, and inside any backticked span that merely looks like
a line number — a port, a ratio, a `sed` address. Either is a failure and not a
nomination once the number runs past the inherited file. There are none in the
tree today. The doubly-backticked escape covers prose and not fences; skipping
fenced blocks is the fix when one appears.

Neither is a defect in a running kern, which is why this sits in tier 9 — but it
is the reason every reconcile pass so far has spent most of its effort
re-pointing citations instead of checking claims.

### 98. The pre-auth frame is capped and deadlined — closed 2026-07-22 `[surface]`

Both halves were real and both are shut. 978 unit tests, e2e untouched.

**The size half was real by a different mechanism than this item named, and the
difference decided where the fix had to go.** There is no length field on this
wire to reject: `JsonEnvelopeCodec` is newline-delimited
(`src/trnsprt/src/typed/codec.rs:53`), so nothing declares a size and
`FramedRead` simply reserves, reads and doubles its `BytesMut` for as long as
`decode` returns `Ok(None)` — which it does until a `\n` arrives
(`tokio-util-0.7.18/src/codec/framed_impl.rs:218`, `state.buffer.reserve(1)`
inside the read loop). So the allocation is not requested, it is accreted, and a
cap enforced *after* `channel.recv()` returns would never run at all, because on
this input `recv()` never returns. The cap therefore lives in the decoder, where
the buffer is, and measures an **incomplete** line: the only shape an endless
frame ever has.

Measured before it was fixed: a peer that writes 16 MiB with no newline had all
16777216 bytes taken into the daemon's buffer, and was then refused at EOF. The
refusal was already there. Only the memory was not bounded — which is why the
test asserts on bytes taken and not on the verdict
(`an_endless_pre_auth_frame_is_refused_without_being_buffered`).

**The patience half was real as written**, though "occupies its accept slot" is
not what happens: `serve_kern_rpc_loop` spawns per connection and keeps
accepting. What a silent peer holds is a task, an fd and a growing buffer, for a
session item 24 guarantees will never be authorised. Confirmed by removing the
deadline and watching a 20 s outer timeout fire instead of the inner one.

**The numbers.** `AUTH_FRAME_MAX = 1024` — a minted token is 64 hex characters
(`mint_token`), so a real frame is 110 bytes; 1 KiB is nine times that and an
order of magnitude under `FramedRead`'s own 8 KiB starting buffer, so the
refusal lands on the first decode. `AUTH_DEADLINE = 5s` — every real client
writes the token in the same breath as the connect, a microsecond conversation
over a local socket. Both are lifted the instant the frame is in hand, so the
authenticated path keeps the unbounded framing an ingest payload needs, and both
are therefore strictly tighter than anything past the gate.

**On timeout the connection is closed without a word**, which is the one place
this path departs from "one message for every refusal". A peer that ran out the
clock never spoke, so there is no misconfigured client to inform — and a reply
would be a free liveness probe that also names the deadline. An oversized frame
*does* get the standard refusal, deliberately: it did speak, and a distinct
answer would tell a caller which limit it hit.

### 96. A shared `target-dir` can report green on stale code `[process]`

The parallel-cycle worktrees all point `build.target-dir` at the main
checkout's `target/`, so one warm 11 GB cache serves every tree instead of each
paying a cold build. The cost was named when it was adopted — cargo takes an
exclusive lock, so concurrent builds serialize — but a second cost was not:
**under concurrent access a run can execute a lib-test binary that predates the
edit under test.** Observed 2026-07-21: a cycle saw `873 passed` with its own
three new tests absent from the run, and `touch src/lib.rs` changed the count to
a self-consistent 867.

**Corrected 2026-07-21, later the same day: it IS cross-worktree contamination,
and the first version of this item said the opposite.** That earlier paragraph
argued from the wrong artifact class. Each tree does get its own
`kern-<hash>` *lib-test binary* — four hashes for four trees — and that was
checked and is true. But a workspace sub-crate is a different artifact: `trnsprt`
has the same package name, version and relative path in every tree, so its
fingerprint matches across them and cargo reuses whichever build got there first.

Two independent observations, both loud:

- `cycle/3` failed to compile with `struct HealthRes has no field named
  ingest_queue_refused` — a field that existed **only** in `cycle/2`'s source
  tree. `cycle/3` was linking a sibling's `trnsprt`.
- `cycle/2` watched `cargo build` report `Fresh trnsprt` against a stale
  `libtrnsprt-*.rmeta` with **hard-link count 3** while its own `dto.rs` change
  sat on disk, in a tree that had just compiled cleanly under `cargo nextest`.
  Two rmetas carrying the field existed under different hashes (check and test
  profiles); the dev-profile one kept losing to sibling builds with a newer
  mtime. `touch src/trnsprt/src/kern_rpc/dto.rs` before each build was the
  workaround it used for every number it reported.

So there are two failure modes, not one:

1. **Stale lib-test binary** — an aggregate count that looks green while the
   binary predates the edit. Fails *quietly*. **An aggregate pass count is not
   evidence that a new test ran.** The discipline that catches it: run the new
   tests BY NAME (`cargo nextest run --workspace -E 'test(<name>)'`) and read
   each result. A filter naming ten tests and printing ten results cannot be
   satisfied by a binary containing none of them.
2. **Foreign sub-crate rlib/rmeta** — one tree links another's `trnsprt`. Fails
   *loudly* here, because the two sources disagreed about a struct field. That
   is luck, not safety: a change that stayed type-compatible would have linked
   silently and produced a green run against a sibling's code.

**Decided: each worktree gets its own `target-dir`.** Three trees were flipped
the moment the second observation landed, and both stalled builds recovered
immediately. It costs a cold build per tree and roughly 33 GB against an 11 GB
shared cache — a straightforward trade once the alternative is "verification
results that may describe another branch's code". The staleness-guard
alternative (a `just` recipe failing when the binary is older than the newest
source) is now moot for the same reason: it would have caught mode 1 and been
blind to mode 2.

**The correct action is to delete the file, not to maintain a path.** Cargo's
default `target-dir` is already `<workspace-root>/target`, which in a worktree
is that worktree's own directory — isolation is what you get by doing nothing.
Confirmed on a tree with no `.cargo/` at all: it resolves to
`kern-cycles/2/target` unprompted. So the whole defect was introduced by adding
a `.cargo/config.toml` pointing at the main checkout, and the fix is to stop
writing that file when a worktree is created rather than to write it with a
different path. A per-tree path works but is a second thing to keep correct, and
the launch step that wrote the shared one is exactly the kind of step that gets
copied forward unexamined.

What survives: the by-name discipline. It is cheap, it catches mode 1
independently of the build layout, and it is what confirmed both cycles that
found this.

### 97. The e2e harness runs the GNN and gates on it — closed 2026-07-22 `[eval]`

**Closed. The premise was right and understated: there were two independent
reasons no propagation ran, and only one of them was the corpus.**
`DEFAULT_MIN_THOUGHTS` is 128 (`src/gnn/propagate.rs:16`) and the recall corpus
is 36 facts (`e2e/test_recall.py:199`) — but `test_recall.py` drives the CLI,
and **the CLI has no tick loop at all**. `do_gnn_propagate` is reachable only
from `tick::start` (spawned by `store::Registry::open`, i.e. a daemon or `kern
mcp`) and from `tick_sync`, whose one caller is a unit test. So the propagation
was not merely skipped at the threshold there; it was never called.

Both halves were measured through the real binary, with the daemon's own
propagation entry temporarily instrumented. At 36 facts under a daemon:
`entities=36 min=128`, entered and returned, nothing applied. **And at 150
facts: still nothing** — the boot cluster pass split the root into 36 + 114 and
neither part reached 128. That is structural, not incidental: `do_cluster`
enqueues `GnnPropagate` only `if did_structural_work`, and structural work is
the same act that moves entities *out* of the kern about to be propagated.

**So the closure the item favoured — grow the corpus — does not work at the
size it implies.** 150 facts leave the largest kern at 114, and the ingest cost
is already superlinear through the CLI: 1.9 s for 36 facts, 54.7 s for 150, each
`kern ingest` loading and re-saving the whole graph. Reaching a *post-split* kern
of 128 would cost minutes per e2e run and would depend on clustering behaviour
nobody has pinned — a gate that silently stops running the day that behaviour
shifts, which is the failure being fixed here, not a fix for it.

**What shipped is the other closure, plus the liveness assertion that makes it
worth having.** `e2e/test_gnn_recall.py` writes an e2e-only `[gnn]
min_thoughts = 4` and `[tick] interval_secs = 0` (boot pass only, so the
embeddings are not still moving under the probes), starts a daemon with
`RUST_LOG=kern.gnn=info`, and **waits for the propagation to report itself**
before scoring anything. `do_gnn_propagate` now logs `learned propagation
applied` with a `nodes` count on success — failure was already loud, success was
silent, and `gnn_vector` is not persisted, so nothing on disk could have
answered "did it run". The test fails if no propagation arrives in 60 s, fails
if it covered fewer than 30 nodes, and only then scores recall.

Proof it is not another vacuous gate. With entity `i`'s propagated embedding
deliberately written to entity `i+1`: **`cargo nextest run --workspace` 972
passed, `e2e/test_recall.py` passed printing its usual 0.9306 / 0.9722 / 0.9471,
and the new test failed 3 of 3 runs** (recall@1 0.7917 / 0.7222 / 0.7361 against
a 0.85 floor). Every existing gate was blind to a GNN wiring bug; this one is
not.

What is left, and it is what the item warned about: **the gate measures a
36-node graph.** It cannot catch a regression that only appears at scale, and
`tests/gnn_scale.rs` — the only thing that runs at 128+ — is `#[ignore]`d and
asserts nothing about ranking. Also unchanged: the propagation is stochastic
(unseeded weight init and negative-edge sampling), so its floors are looser than
the CLI corpus's and were set below the worst of 8 runs rather than from one.

### 92. Tests that race a backward-stepping `CLOCK_REALTIME` — closed 2026-07-22 `[eval]`

**Closed. Both tests were already fixed when this item last said one was not,
and the constant it recorded is wrong by 2.4x.** The mechanism holds:
`CLOCK_REALTIME` does step backwards here while `CLOCK_MONOTONIC` runs straight.
What it does not do is step "roughly 2.8 s every 30 s".

Measured two ways sharing no code path. A sampler reading both clocks every
50 ms and reporting every change in `realtime - monotonic`: over 300.009 s of
monotonic, **9 backward steps, mean -1.243 s, mean period 32.25 s**, 11.185 s
of realtime lost. A shell cross-check of `/proc/uptime` against `date(2)` over
120 s: 3.730 s lost, which is three steps of 1.243 s to a millisecond, reached
by arithmetic nothing in the sampler touches. A second 240 s window caught the
informative outlier: one period stretched to 47.45 s and its step grew with it,
to -1.816 s.

So **the rate is the invariant, not the step**. `1.243/32.25` is 3.85% and
`1.816/47.45` is 3.83%: realtime runs ~3.8% slow and the sync repays the entire
accrued drift in one jump whenever it fires. That reframes the item. A margin is not
unsafe because a fixed 2.8 s might land inside it; it is unsafe because the loss
scales with how long you wait, and a delayed sync bunches it.

It also explains the reproduction failures better than the old entry did. The
pre-fix shape — `time.sleep(RETENTION + 2)`, a 2 s margin over a 5 s retention —
was run 12 times interleaved with `test_recall.py` against today's rate and
passed **12 of 12**. One 1.24 s step cannot eat 2 s, and a 7 s window cannot
hold two steps that are 32 s apart. The six clean runs the original entry filed
as puzzling were the *expected* outcome; the two observed failures needed a
stretched sync interval, which the 47.45 s sample proves does occur.

Which means waiting for the flake is not a test. Reproducing it needs the step
**constructed**: an `LD_PRELOAD` shim over `clock_gettime` subtracting
`STEP × floor(monotonic / PERIOD)` from `CLOCK_REALTIME` alone. The offset is a
pure function of the monotonic clock, which is boot-relative, so the pytest
driver and every `kern` subprocess it spawns see one consistent warped realtime
while `CLOCK_MONOTONIC` stays untouched. At 2.8 s every 5 s the pre-fix shape
fails **5 of 5** on its original message — "an expired fact was still
delivered" — and the shipped shape passes **5 of 5** on that same clock, whole
file included.

**Both fixes are in and verified.** `src/ingest/intake.rs` waits on the wall
clock, restarts its marker on a backward step, and caps on the monotonic clock.
`e2e/test_retention.py` waits to an *absolute* realtime target — which needs no
restart, since only realtime reaching the target can end the wait — and then
polls for the drop, because a step landing between the wait and the query can
put a passed deadline back in the future. It has done both since 2026-07-21
17:57 — seven hours before the 2026-07-22 00:37 commit that updated this item to
say it wanted the treatment. That sentence was written from the item rather than
from the file, which is the same reading-the-record-instead-of-the-thing mistake
that put 2.8 s in it.

An injected instant was considered and **declined**, on two counts. `kern query`
has no `--valid-at`; only the MCP surface does (`src/mcp/tools_query.rs`), so
the e2e harness cannot reach one without adding a CLI flag for a test's sake.
And it would measure the wrong path: `drop_expired` (`src/retrieval/score.rs`)
returns early whenever `valid_at` or `as_of` is set, leaving the work to
`matches_filter`, so an injected instant exercises the filtered reader while
production expiry rides the unconditional pass. Trading the real path for a
mockable one is no fix for a flake that never lived on the real path.

Nothing else here shares the defect. The only other test that sleeps and reads a
clock is `tick_head_of_line_delay` (`tests/gnn_scale.rs`), which measures with
`Instant`, and `e2e/conftest.py`'s `wait_until` is monotonic on both sides.

What stands is why this was written down at all: **the next person to see it red
will assume it is this flake and wave a real regression through.** An
intermittent failure nobody has recorded is indistinguishable from a regression
nobody has noticed — and one recorded with a wrong constant is worse, because it
reads as adjudicated.

### 70. The oracle pre-commit hook is untracked and has no installer `[process]`

`ORACLE.md` rule 1 is enforced by `.git/hooks/pre-commit`, which lives only in
`.git/` and is created by nothing in the repo — no `justfile` recipe, no
`install.sh` step, no `.pi/update.sh` line. A fresh clone has **zero enforcement
of the ruling every commit is supposed to answer to**, and nothing announces it.
The hook itself calls this out as a "per-clone install product"; the install half
does not exist. Wanted: track it under `scripts/` and install it via
`core.hooksPath` or a `just` recipe run by `.pi/update.sh`.

### 75. Crash consistency on the DiskANN path `[store]`

**Half of this was verified false 2026-07-21 and the residual risk is narrower
than stated.** The item was written off `docs/kern/diskann-disk-index.md:142-143`
("no WAL and no atomic-rename-per-segment") and never checked against the build.
Per-segment atomic rename *does* exist: `atomic_write`
(`src/base/diskann.rs:293-297`) writes `<path>.tmp` then `std::fs::rename`, and
`build_and_save` uses it for all three segments — meta (`:272`), vectors
(`:280`), graph (`:289`). `DiskIndex::open` (`:310-355`) then rejects a divergent
set rather than reading it: ids-length vs count, entry point in range, both mmap
lengths against the meta's `count × dim × 4` / `count × r × 4`, and every
adjacency slot either `SENTINEL` or a valid node id — the last one specifically
so a beam walk cannot slice the vector mmap out of bounds.

What is actually left is **cross-segment** atomicity, and it is a real hole: three
independent renames mean a crash between them leaves meta from build N+1 beside
vectors from build N. That survives the length checks whenever the two builds
have the same `count`, `dim` and `r` — the common case, since a rebuild usually
changes vectors and not shape. There is also no `sync_all` before the rename, so
rename ordering is atomic while the *data* behind it need not be durable. A
corrupt index is not fatal — `build_entity_disk_snapshot`
(`src/base/graph.rs:351-366`) logs and falls back to the in-RAM index — so this
is silent staleness, not a crash. Wanted: one rename that publishes all three
(a versioned directory, or a manifest naming the build the three files belong
to), and an fsync before it.

Beside it, both unverified against source and still doc-only: mmap file-locking
and flush semantics differ on Windows
(`docs/kern/diskann-disk-index.md:149-150`), and PQ codebook training/drift has
no retrain trigger — "a bad codebook silently degrades recall" (`:145-148`) —
which lands in item 1's lap the moment PQ is promoted out of the non-goals.

### 76. The watchdog force-exit skips the final guarded flush `[store]`

Confirmed against source 2026-07-21, and it is not a doc claim: `spawn_watchdog`
(`src/commands.rs:929-968`) beats a counter once a second from the async runtime
and force-exits `std::process::exit(101)` (`:959`) after `STALL_LIMIT * CHECK_SECS`
= 30s of no progress. `process::exit` runs no destructor and no `Drop`, so it
skips the guarded shutdown flush the ordinary path takes — the `shutdown` notify
at `src/commands.rs:891` unwinds into the guarded persist closure `save_fn`
(`:632`, called at `:897`),
which is the thing that "never overwrites a grown disk". Nothing on the watchdog
path writes anything, and the exit line does not say so.

The stall it fires on is named as "graph deadlock or worker starvation", which is
precisely the state where the in-memory graph is ahead of disk and unreachable.
Combined with item 10 the default posture can lose up to a tick interval of
writes with no log. The awkward part is that a stalled runtime is exactly when a
flush may itself block, so "flush before exiting" is not free — wanted is a
bounded attempt (flush on the watchdog's own thread with a hard deadline, exit
either way) plus a line saying which of the two happened.

### 77. Hash composition is an unguarded breaking change `[store]`

"Changing how a hash input is composed is a breaking change to every existing
graph" (`concepts/graph.mdx:86-88`), and source confirms the exposure. The hash
itself is pinned — `content_hash` (`src/base/util.rs:3`) is sha256-to-lowercase-hex
and `util.rs:155` asserts length, alphabet and determinism. What is unpinned is
every *composition* feeding it, and each one is a different format string: entity
ids are `content_hash(text)` bare (`src/ingest/place.rs`,
`src/ingest/file_watcher.rs:182`, `src/ingest/direct.rs:44`,
`src/ingest/worker.rs:148`), `Source::source_id` is
`scheme \x00 object \x00 section` (`src/base/types.rs:270-282`), child and named
ids are `parent_id + nonce` and `parent_id + name + nonce`
(`types.rs:515-516`, `:520-521`), and the HNSW canon has its own
(`src/base/hnsw.rs:525`).

The bare-text composition is load-bearing beyond identity: the gossip import
guard re-derives `content_hash(&e.text()) == e.id` to refuse a forged remote
statement (`src/gossip/handler.rs:536`, and the note at `:521` names every minting
site it depends on). So changing that composition does not merely orphan old ids —
it makes every existing remote statement fail its receipt.

Repo law 1 guards bincode schema round trips; nothing guards or versions any of
these, and there is no migration path. Wanted: golden-vector tests pinning each
composed input string — not the digest of a value, the *shape* of what is hashed —
same standing as the bincode guard.

### 78. A non-local LLM URL egresses everything, silently `[surface]`

"The full text of everything kern captures transits that provider"
(`concepts/security.mdx:81-96`) when a non-local endpoint is configured — no
redaction, no allowlist, no warning at config load, no egress log. For a project
whose first claim is "local-first, zero egress", the one setting that voids it is
unremarked.

### 79. `validate_fact_source` is dead code `[surface]`

Called **once** (corrected 2026-07-21 — it was twice; the second site left with
the ingest `kind` arg in `216730d`), with the literal `AGENT_SOURCE`
(`src/mcp/tools_mutate.rs:161`), and it accepts `USER_SOURCE` / `AGENT_SOURCE`
(`src/base/validate.rs:21`), so it can never fail. Decision:
thread a real auth identity (item 18/24), or delete. Delete is correct for a
single local daemon and needs only sign-off.

### 81. `resources/list` and `prompts/list` return `-32601` on the proxy path `[surface]`

`ProxyServer` implements `tools_list` / `call_tool` / `extra_capabilities` only
(`src/commands/mcp_cmd.rs:301`, `:317`, `:355`) with no `handle_method` override,
so the trait default returns `None` (`src/trnsprt/src/server.rs:21`). Meanwhile
`extra_capabilities` advertises `{"resources": {}, "prompts": {}}`
(`src/commands/mcp_cmd.rs:358` — spelled in full because the nearest preceding
path is `server.rs`, and a bare `:NNN` continues the wrong file) to
match standalone, which *does* serve them (`src/mcp.rs:213-222`, advertised `:160`). Advertised on
the normal path, non-functional there. Either forward them or stop advertising.

### 82. Standalone `kern mcp` runs no gossip `[surface]`

**Corrected:** the previous version said "no maintenance tick and no gossip". The
tick *is* started (`src/commands/mcp_cmd.rs:473-484`); only gossip is absent
(`broadcast_q: None` at `:479`, `broadcast_pulse: None` at `:493`). A graph
served that way decays, clusters and GCs normally, and simply does not federate.

### 83. Nothing bounds memory deterministically: eviction and spill are both disarmed `[lifecycle]`

**Retitled 2026-07-21 — the old title named a knob that does not exist.**
`KERN_CAP_DISABLED` (`src/base/constants.rs:30`) is a *kern-eviction* sentinel,
not a per-kern entity cap; its own comment says so. It defaults both `max_kerns`
and `disk_threshold` to `usize::MAX` (`src/config/graph.rs:18,20`), and those are
the two things it disarms: `enforce_kern_cap` (`src/base/graph.rs:216`) never
unloads a kern, and the DiskANN spill branch (`src/base/graph.rs:296`) never
fires. A per-kern *entity* cap for local kerns does not exist at all — the only
one in the tree is `GOSSIP_REMOTE_KERN_ENTITY_CAP` for `remote-*`. Wanted,
unchanged: a safe cap plus an escalation policy. The comment's "currently unsafe"
is a real reason nothing is set — eviction drops unpersisted `children` pushes
(`src/config/graph.rs:16-17`).

**The double-storage half shipped 2026-07-21; the bounding half is untouched.**
Item 29 finding 3 assigned it here: every vector was resident twice. Verified
before building rather than assumed — `index_kern_into` handed the index
`t.vector.clone()` (`src/base/graph.rs:34`, `gnn_vector` `:38`, reasons `:46`)
and `HnswNode` stored that clone verbatim, because the shipped default is
`QuantizationMode::None` (`src/quant.rs:8-9`; int8 is opt-in through
`kern compress`, and under it the node's float vector was already empty, so this
buys nothing there). The two copies were the same floats, not a normalised one
and a raw one — recall is unmoved to four decimals across the change, which is
the check that would have caught it had they differed. `Entity::vector`,
`Entity::gnn_vector` and `Reason::vector` are now `Embedding`
(`src/base/types.rs:609`) and every index holds the map's own allocation.

Measured with `tests/spill_memory.rs` in `resident` mode — 50k entities at dim
384 each carrying `vector` AND `gnn_vector`, plus 25k reasons, one process per
reading, ten interleaved before/after pairs in release:

| | hot RSS | index walk, median |
| --- | --- | --- |
| before | 510.2 MB | 190 µs |
| after | **324.6 MB** | 211 µs |

**−185.6 MB, −36.4%**, with 0.5 MB of spread across ten runs. The three
`*_only` rows are the control — they still copy, and the type now makes that
copy visible as a `to_vec` (`src/base/graph.rs:336` does the same for the
DiskANN build input) — and they moved ≤1.3 MB, which is the `Arc` header on
125k allocations.

The cost is query latency, and naming it is the point: **+17 µs median on a
~190 µs index walk (+9%), slower in 11 of 15 paired runs.** It is not the atomic
refcount, which is never touched on the read path, and not the indirection —
`Arc<[f32]>` derefs exactly as `Vec<f32>` does. The likely mechanism is
locality: the index's vectors used to be allocated together during its build and
now point into the scattered kern map. That mechanism is unproven, and the
measurement cannot settle it — the host carried load average 4-6 from two
sibling cycles throughout and the before-arm spread was ±60 µs, the same order
as the effect. What is safe to claim is that it is not a step change.

Still open here, and the reason this item does not close: **nothing bounds the
resident set.** Halving the O(N) term moves the ceiling; it does not install
one. Also deliberately unclaimed, so that the number above measures one change:
`do_reembed` seeds `vector` and `gnn_vector` from the same embed
(`src/tick/tasks.rs:545-546`) and they could share a third allocation until GNN
propagation overwrites one — another 76.8 MB at this corpus size.

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
  (`FEATURES.md:641-642`).
- The LLM client is Ollama-centric with no retry/backoff policy object
  (`FEATURES.md:919-920`).
- ~~Watcher `.gitignore` parsing is approximate; no rename tracking~~ **(retired
  2026-07-21 — verified false on both counts).** `IgnoreRules` builds a real
  `Gitignore` through ripgrep's `ignore` crate
  (`src/watcher/src/ignore_rules.rs:3`), i.e. the full spec, and
  `WatchKind::Renamed {from, to}` carries both endpoints
  (`src/watcher/src/event.rs:9`). What survives is narrower and still open:
  **a rename is not re-keyed in the graph.** `build_record`
  (`src/watcher/src/pipeline.rs:48`) ingests `to` and discards `from`, so the
  renamed file lands as a new `Document` and the old one is neither moved nor
  removed. It duplicates only when the rename *also* edits the file — ids are
  `content_hash(text)`, so an untouched move re-resolves to the same id, while
  `external_id` is the path (`src/ingest/file_watcher.rs:186`), so a
  move-plus-edit gets a new id under a new external id and supersede never
  fires. It sits in this tier and not in tier 1 because the watcher is **off by
  default** — `WatcherConfig::enabled` is `false` unless a `kern.toml` sets it
  (`src/config/watcher.rs:14-16`) — so it is not a default-path defect
  (`FEATURES.md:1085-1088`).
- `unnamed` lists only; there is no `promote` (`FEATURES.md:818`).
- GNN has no GPU path, weights are per-kern rather than shared, and the objective
  is link-prediction only (`FEATURES.md:593-594`).
- Under WSL2 NAT a loopback Ollama URL must be hand-pinned; kern neither rewrites
  nor warns (`FEATURES.md:1130-1132`).
- RPC socket bind→chmod race — sub-millisecond, umask default — recorded as an
  accepted risk where it happens (`harden_socket`,
  `src/trnsprt/src/typed/local.rs:348-358`); revisit only if the umask
  alternative stops being worse. **Corrected 2026-07-22:** this cited
  `concepts/security.mdx:40-43`, which is the API-key-vs-redirected-endpoint
  rule and says nothing about the socket; that page states the `0600` mode at
  `concepts/security.mdx:16` and does not record the race at all.

### 101. Every anchor the widened checker exposed now points where its sentence says — closed 2026-07-22 `[process]`

`docs-check` learned to resolve bare continuation refs — `` `:239` `` following a
`` `place.rs:112` `` — and immediately nominated a list that was always wrong and
always invisible. The count is a measurement of the old blind spot, not of new
damage: nothing broke, the checker started looking. The merge that filed this
counted 18; the list read off this branch is **20**. The discrepancy was not
chased — the list is what the checker prints, not what the filing remembered.

Each of the 20 was adjudicated by reading the citing sentence, opening the cited
line, finding where the described thing actually lives, and reading the new
target back. The list resolved into four classes, not the two the filing guessed:

- **Stale line numbers**, the ordinary rot — 11 of them. The `move` row's
  `` `tools_mutate.rs:491` `` was three lines short of `tool_move`; the `Acl`
  struct had moved 13 lines; `mcp_cmd.rs`'s two `None`s had drifted 18.
  Mechanical, one target read at a time.
- **A bare ref continuing the wrong file** — 4, plus a fifth wearing a
  quotation's clothes. Item 52 documented the hazard and it is exactly as
  predicted: `bind_unix`'s `` `:507` `` bound to `src/commands.rs` and meant
  `src/trnsprt/src/typed/local.rs`; `Worker::submit`'s `` `:129` `` bound to
  `src/ingest/direct.rs` and meant `src/ingest/file_watcher.rs` — the *number*
  was right, only the file was wrong, which is the quietest failure of the set;
  and item 75's two doc-only leads bound to `src/base/graph.rs` while meaning
  `docs/kern/diskann-disk-index.md`. None of these can be fixed by adjusting a
  number. Each citation is now spelled in full at the point the file changes,
  and only the continuations that follow it are left bare.
- **A quotation the checker read as a citation** — 2, both in this item's own
  first draft, which named the broken anchors in single backticks and so made
  fresh copies of them. The fifth counted above — item 30's note on the dead
  `concepts/acceptance.mdx` citation — is the same defect wearing a wrong-file
  binding as well. The repo already has the fix: a doubly-backticked span is an
  illustration and is blanked before scanning. The lesson is narrow and worth
  keeping: **a page describing a broken citation must display it, not make it.**
  Writing this entry reproduced the fault twice before the checker caught it.
- **False positives** — 2, and both are the same shape. The `gc` row cited
  `tool_gc` correctly, and `README.md:399` pins the version correctly; in both
  the only distinguishing token is under the three-character floor (`gc`) or is
  digits the tokeniser discards (`1.1.0`). Neither was acquitted with
  `anchor-ok`. The `gc` anchor was widened by two lines to reach the `reaped` /
  `before` / `after` binding the row actually describes, and the `README` line
  now names the anti-entropy pointer that shares it — both true of the target,
  and both leave the anchor checked rather than silenced.

**Two out-of-band repoints came with them**, found by reading targets rather than
by nomination: `Entity::acl` was cited into `src/base/types.rs` at `` `:296` ``
(an unrelated field) and `start_gossip`'s builders into `src/commands.rs` at
`` `:1046-1127` `` (an unrelated span). Both are wrong the same way and neither
was nominated, because a long enough citing block shares words with almost
anything. That is the residual: the content check bites where the sentence is
short.

11 + 5 + 2 + 2 = 20. So the false-positive rate on the widened checker's first
real list is 2 in 20, and both are floor artefacts rather than judgement errors
— which is the number `--strict-anchors` has to be adopted against.

Note the shape of the win: the checker's own improvement is what made its
previous coverage claim false. Every "no anchors nominated" before this widening
meant "none among the 63% I could parse".

### 99. The watcher's off-limits set is a list of names, not an invariant `[ingest]`

Item 30's durable backstop put a kern-written file inside the default watched
root and the watcher ate it — 283 payloads from one seed edit before the fix. The
fix works and is measured: `IgnoreRules::with_denied`
(`src/watcher/src/ignore_rules.rs:45`) takes the resolved `intake.dir` and
`data_dir` from `spawn_file_watcher` (`src/commands.rs:967`). But it closes that
loop by *enumerating* the two directories that were writing, not by making the
class impossible. Anything kern writes under a watched root in future is
ingestible again unless someone remembers to add it, and there is no test that
fails when they forget — the regression e2e
(`e2e/test_file_watcher_durability.py`) pins the two dirs that exist today.

Two shapes worth weighing, and neither is obviously right. **One state root**:
declare that everything kern writes lives under a single prefix and deny that
prefix, which is nearly true already (`.kern/` holds config, intake and data) but
false the moment `data_dir` is pointed outside it — which is a supported config
and the reason the deny list is computed rather than hardcoded. **Deny at the
writer**: have the code that opens a path for writing register it, so the deny
list is derived from what the process actually holds rather than from a list
maintained by hand. The second is correct and costs a registry the daemon does
not have.

Ranks here and not in tier 1: the two paths that matter today *are* denied, the
failure mode is amplification rather than a wrong answer, and the whole watcher
is off unless a `kern.toml` enables it (`src/config/watcher.rs:8`). <!-- docs-check: anchor-ok -->
It is a latent correctness hole, not a live one.

Deciding behavior: fix-the-root — the enumeration is the symptom-level fix, kept
because the root fix is a registry and the loop was live.

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
- (retired 2026-07-21 — the id path stopped bypassing filters and the page was
  corrected in the same change) the `id` path in `query` bypassed every filter
  (item 18); `howto/mcp.mdx:50` now says every filter applies to an `id` read.
- (retired 2026-07-21 — the tables were filled in) the `move` MCP tool is listed
  in `README.md:352` and `FEATURES.md:619`, and the site's count is now thirteen
  (`howto/mcp.mdx:5, :75`), so the "Eleven tools" note is retired with it.
  <!-- docs-check: anchor-ok -->
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
  forbids no longer survive. `FEATURES.md:201` keeps only the `+7% p50` latency
  half and says the retrieval-quality half is withdrawn;
  `docs/kern/diskann-disk-index.md:26` says the note "previously published" the
  `recall@10 ≥ 0.90` figure; this file's own "recall@10 A/B" citation was struck
  earlier.
- (retired 2026-07-21 — `README.md` and `VISION.md` were corrected) neither
  opens on "takes in durable facts from your sessions" or "learns on its own"
  any more; `VISION.md:52` now *states* there is no recorded baseline instead of
  gating claims on one; `README.md:393-394` says the Question and Pulse senders and
  the fetch RPC are live, and `:399` pins the version at `1.1.0` beside the
  anti-entropy pointer, matching `FEATURES.md`.
- (retired 2026-07-21 — all three fixed) `FEATURES.md:54` now lists `Entity`'s
  `acl` <!-- docs-check: anchor-ok -->; the retired query-cache finding is gone from the file entirely; and
  `:634-635` marks prompts and resources "served on the standalone path only",
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

- **The second dedup gate no longer lies about what it did** — was item 91
  `[ingest]` (the third item to carry that number), closed 2026-07-21. Both
  halves shipped in `src/ingest/place.rs`. `place_document` returns
  `Some(result.entity_id)` instead of an unconditional `Some(doc_id)`, so
  `finalize_doc_identity` — which infers dedup from `surviving_id != content_id`
  and was **correct all along, merely fed a lying id** — now reports `Deduped`
  where it reported `Committed`. And both `lex.insert` calls are gated on
  `!result.deduped`. Gating the index is the right half rather than reindexing
  the discarded id under the survivor, because the discarded id names nothing:
  `seed_lexical` (`src/retrieval/seed.rs:96`) does not filter by graph presence,
  so the ghost reached `fuse::rrf`, was rescored to `0.0` by
  `find_entity_ref_in_graph`'s `unwrap_or(0.0)` (`src/retrieval/query.rs:118-120`)
  and *displaced* a live seed. Removing it is a strict win; carrying the wording
  onto the survivor is a separate, larger fix, filed as item 94.
  Proven by reverting each half separately: restoring `Some(doc_id.to_string())`
  fails `a_second_gate_dedup_reports_deduped_and_the_surviving_id` on
  `left: 0302188…` / `right: 64989cc…` and leaves the chunk test green;
  ungating the two `lex.insert`s fails
  `place_chunks_second_gate_keeps_the_discarded_id_out_of_the_lexical_index`
  alone. `place_document` also stopped cloning the whole `Entity` under the
  write guard — `tid`/`joined` are read off it before the lock, mirroring
  `place_chunks`; nothing in that hoist reads guard-held state.
- **Deleting a source cascades into the graph** — was item 19, closed
  2026-07-21. `forget_by_source(scheme, object_id, force)`
  (`src/commands/graph_ops.rs`) resolves every entity whose `Source` matches the
  pair across all resident kerns and cascades through the existing
  `forget_entity`, so edge removal has one implementation. The key is
  deliberately `(scheme, object_id)` and **not** `source_id`, which hashes the
  section too — keying on that would forget one chunk of a document and leave
  the rest. Reachable as the MCP tool `forget_by_source` and as
  `kern forget --source <scheme>://<object_id> [--force]`, which routes through
  the daemon with the local path as the `NoDaemon` fallback (item 9's contract);
  `a_routed_forget_by_source_mutates_the_serving_daemons_graph` and an e2e that
  blinds the CLI's `data_dir` prove the write lands in the daemon's live graph
  and not in a stale on-disk copy.
  **The guard was in two places, not one.** `remove_entity`
  (`src/base/reason.rs`) carries its own local-Fact immunity check, so a `force`
  that lifted only `forget_entity`'s outer guard would have counted and reported
  `removed_entities: 1` while removing nothing — success printed over a silent
  refusal. Both take `force`; every other caller, GC included
  (`src/tick/stigmergy.rs`), passes `false`, and that is the only bypass of the
  Fact guard in the tree. The response carries a third field, `kept_facts`,
  beyond the two the item asked for: without it a source made only of local
  Facts answers `removed_entities: 0`, which is indistinguishable from "that
  source was never ingested" — the refusal has to be observable or `--force` is
  undiscoverable and an incomplete deletion reads as a complete one.
- **Retention reaches the id read surface** — was item 91 `[retrieval]` (the
  second item to carry that number; the `[ingest]` one is still open), closed
  2026-07-21. Every claim in the item was re-verified against source first and
  all of them held. The item prescribed "one filter at one call site"; that was
  **not** what shipped, and the reason is the decision the item deferred. An
  explicit id names one row. The ranked path answers "what is true now", so it
  drops; `kern get <id>` answers "what is this row", and replying
  `thought not found` for a row that is on disk — and that GC never collects,
  since a non-superseded `Fact` is immune (`is_cold_victim`,
  `src/tick/stigmergy.rs:35-46`) — is a false statement the caller has no way to
  falsify. So the id path **serves and flags**: `entity_detail`
  (`src/mcp/tools_query.rs:382`) emits `expired` and `valid_until` whenever a
  retention is set, and `kern get` prints an `Expired:` line
  (`src/commands/graph_ops.rs:67`). Filtering lost because the surface item 9
  deliberately widened — prefix plus cold-tier fallback — would have been
  silently narrowed by it to nothing a caller could distinguish from a typo.
  Proven by revert: dropping the two lines in `entity_detail` fails
  `graph_ops::tests::the_id_path_flags_an_expired_thought_instead_of_hiding_it`
  on `left: Null, right: Bool(true)` and fails
  `e2e/test_retention.py::test_an_expired_fact_is_served_by_id_but_flagged` on
  a real `kern get` printing the expired fact with no marker. The bi-temporal
  escape is now pinned at the call site too, not only on the predicate:
  `retrieve_drops_an_expired_claim_from_the_default_path`
  (`src/retrieval/query.rs:609`) runs the same corpus twice and asserts an
  `as_of` query still returns the since-expired claim; neutering the early
  return in `drop_expired` fails that half alone. What this did **not** buy is
  item 18's fourth bullet — that bullet wants ACL enforcement on the id path,
  and an ACL denial cannot be expressed as a flag on the row it is denying, so
  it still needs its own guard.
- **A deduped ingest carries its retention** — was item 88, closed 2026-07-21.
  There were two dedup gates and both swallowed it; both now funnel through one
  site. `accept::merge_duplicate` takes the incoming `valid_until` and calls
  `accept::merge_valid_until`, so the `find_duplicate` gate in `place.rs` and
  `commit_entity`'s `dup` branch reach the same rule, and the fresh-placement
  path calls it directly after accept. The rule is `resolve_valid_until` — `min`
  with `None` as +∞, decided over LWW because a TTL is a ceiling and `min`
  converges under the arbitrary replay order federation produces where
  last-writer does not. Accepted cost, stated in the tool schema: ingest can
  shorten a deadline and never lengthen one. The orphan delta the item predicted
  was real and is gone — the pre-accept stamp in `place.rs` minted a
  `PendingDelta` for an id gate 2 then discarded; the stamp moved *after* accept,
  onto the id that entered the graph. Proven by reverting each half separately:
  neutering gate 1 fails three `place.rs` tests and leaves the gate-2 test
  green, neutering gate 2 fails only the gate-2 test, and removing the
  `push_delta` fails all three delta assertions including the fresh-placement
  one. `e2e/test_retention.py::test_a_deduped_ingest_still_applies_its_retention`
  fails loudly with the merge neutered. What this did *not* buy was item 91
  `[ingest]` — not the `[retrieval]` 91 listed above: the same gate still
  reported `committed` on a dedup and still left the discarded id in the
  lexical index. That closed separately, later the same day.
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
  that stays. What the item did *not* buy became items 88, 89 and 90; 88 — the
  dedup branch swallowing a retention — and 89 — the other two entrances, and
  the missing config key — both closed the same day, and what is still open is
  that `DirectJob` drops `valid_from` (90).
- **Retention reaches all four entrances, from a key a `kern.toml` can set** —
  was item 89, closed 2026-07-21. The `.txt` distillation path and the
  file-watcher sink had no caller to pass a flag: `drain_entry` cloned the
  queue's `Config` and overwrote only `valid_from`, and `KernFileWatcherSink`
  handed the worker `IngestRunConfig::default()` outright. Both now carry one,
  through `Config::with_retention` onto the existing
  `valid_until_from_retention` — still one duration→instant conversion, so the
  four entrances cannot drift. **The key could not go where this item said.** It
  named `IngestConfig`, but `load_with_user` refuses a user-written `[heat]`,
  `[ingest]` or `[retrieval]` section outright, so the half the item itself
  called load-bearing would have shipped unsettable; it went to `[intake]` and
  `[watcher]`, the sections that name the sources, and
  `a_real_kern_toml_can_set_per_source_retention` loads a real file rather than
  trusting the struct. Resolved per drain pass and per watched record, never at
  daemon start — hoisting it back above the intake loop collapses two
  transcripts queued two seconds apart onto one deadline and fails
  `the_poll_loop_resolves_its_deadline_per_pass_not_once_at_startup`. **Two
  things this did not buy.** The file-watcher half is unit-covered only —
  `WatcherConfig::enabled` is `false` by default and nothing in `e2e/` starts a
  watcher, so `the_sink_stamps_the_configured_retention_on_what_it_ingests` is
  the whole proof for that entrance. And a durable `direct/` job still cannot
  inherit a standing policy: `drain_direct_once` overlays `job.valid_until` over
  the loop's config, and an absent flag and `--retention-secs 0` are both `None`
  on the wire, so per-call wins with no way to defer. Deliberate — the flag is
  the more specific statement — but it is a seam, not an absence.
- **The intake is visible and drivable** — was item 8, closed 2026-07-21.
  `kern intake` (alias `kern intake status`) prints pending with age, the last
  error for anything stuck, quarantined `failed/` entries and the `done` count;
  `kern intake drain` forces one pass, sharing `drain_once` with the daemon's
  loop so the two can never diverge (routed through the daemon since 2026-07-21;
  in-process when none is serving, so the CLI still works with no daemon).
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
  in `start_gossip` (`src/commands.rs:1105-1136`), pulse wired into the maintenance
  tick (`:760`) and the `pulse` MCP tool (`src/mcp/tools_admin.rs:218`),
  `broadcast_q` invoked by `do_resolve` (`src/tick/tasks.rs:386`), `handle_question`
  live-dispatched (`src/gossip/handler.rs:44`).
- **`Fetch` is wired** — `wire_fetch` installs the handler at
  `src/commands.rs:1098`. Single-id, so it is not anti-entropy (item 36), but it
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
  `clamp_confidence(1.0, "user")` (`src/commands/ingest_cmd.rs:59`).
- **`conf` is clamped to [0,1]** — `validate_conf` (`src/base/validate.rs:14`)
  called at `src/mcp/tools_mutate.rs:159`.
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
  to overlap. Nothing to kill. (`KernRpc` mirroring the MCP tool list 1:1 was
  carried as item 24's second half; that half is retired 2026-07-21 — verified
  false on this same reading.)
