# Federation roadmap — what to build to make it useful

> **What this is.** A concrete, code-grounded list of what must be added or
> changed in kern to make gossip federation actually deployable — not the
> design study (`docs/kern/crdts-federation.md`) and not the operator trust
> guide (`docs/FEDERATION-SECURITY.md`). This names the gap between the code
> that runs today and federation that is safe to turn on outside a trusted LAN.
>
> **Status of federation today.** Gossip is off by default. With it on, a node
> heartbeats peers, broadcasts its root scope and hottest entity bodies, and
> merges inbound entity bodies into a quarantined `remote-<network_id>-<kern_id>`
> namespace. That is the *entire* working surface. Everything else in the wire
> enum is handled on receive but has **no live sender**.

## Verified against code

Read of `src/gossip/`, `src/crdt.rs`, `src/base/merge.rs`, `src/base/types.rs`,
`src/wire.rs`, `docs/FEDERATION-SECURITY.md`, `docs/kern/crdts-federation.md`
at v1.0.0.

### What sends today

Only three message kinds have a sender. The heartbeat loop
(`src/gossip/node.rs` `start_heartbeat`) broadcasts `PeerExchange`. The two
announce loops (`src/gossip/handler.rs` `start_announce`, `start_entity_sync`)
broadcast `Sphere` and `EntitySync` on the heartbeat interval, each carrying the
node's hottest 32 entities (sorted by heat, truncated).

### What receives but never sends

`src/gossip/handler.rs` `new_handler` dispatches all seven kinds, but
`GossipKind::Question`, `GossipKind::Delta`, and `GossipKind::Pulse` have **no
emitter anywhere in the tree** (grep across `src/` finds only the handler-side
match arms and the type defs). They are dead on the send side — a handler
exists, nobody calls it.

| Kind | Handler | Sender | Status |
|------|---------|--------|--------|
| `Sphere` | `handle_sphere` / `handle_answer` | `start_announce` | **live** |
| `EntitySync` | `handle_entity_sync` | `start_entity_sync` | **live** |
| `PeerExchange` | `handle_peer_exchange` | `start_heartbeat` | **live** |
| `Fetch` | (`node.rs` `handle_fetch`, not `handler.rs`) | `fetch_thought` (request/reply) | **live, single-thought only** |
| `Question` | `handle_question` | none | **dead (receive-only)** |
| `Delta` | `handle_crdt_delta` | none | **dead (receive-only)** |
| `Pulse` | `handle_pulse` | none | **dead (receive-only)** |

### What the CRDT layer actually is

`src/crdt.rs` ships `GCounter` and `PnCounter`. That is the full primitive set.
The CRDT plan in `docs/kern/crdts-federation.md` names OR-Set and LWW-Register
as needed for `statements`, `valid_until`, and `Reason.score` — **none of those
exist.** `src/base/merge.rs` `merge_entity` does ad-hoc max/min joins on
timestamps and a status lattice; it is hand-rolled last-writer-wins, not a
formal CRDT, and concurrent writes on `statements` / `Reason.score` can silently
overwrite.

The `CrdtDelta` wire payload (`src/gossip/types.rs` `CrdtDeltaPayload`) carries
only `ThoughtAccessCount` and `ReasonTraversalCount` (both G-Counter). No delta
type for any other field. And even this one has no sender.

### Confidence is deliberately not federated

`src/base/merge.rs` `merge_entity` carries a load-bearing comment:
`conf_alpha`/`conf_beta`/`unlinked_count` are **never imported from remote** — a
max-join on confidence would be an irreversible poisoning pin. This is correct
and must survive every change below. Trust is replica-local.

---

## Workstreams (ordered)

### F0 — Live delta senders  *(the dead wire comes alive)*

**Problem.** `Delta`, `Question`, `Pulse` are receive-only. The heartbeat
broadcasts full entity bodies every interval; there is no incremental state
propagation. Two replicas answering the same query each bump `access_count`
locally; without a `Delta` sender the G-Counter increments never reach the
peer, so the G-Counter convergence the CRDT layer was built for never happens.

**Build.**
1. **`Delta` sender on every counter increment.** After
   `retrieval::score::commit_access` bumps `access_count`, emit a
   `GossipKind::Delta` / `CrdtDeltaPayload` for that `(object_id, target,
   replica, value)`. Same for `Reason.traversal_count` after a traversal.
   `value` is the sender's **absolute** slot total (per the existing comment
   in `types.rs` — a delta-since-last would be lost under the receiver's
   max-merge). Throttle: one delta per object per heartbeat, coalesced.
2. **`Pulse` sender.** `tick::pulse::pulse` is already called on receipt of a
   remote `Sphere`/`Pulse`; add a sender so a local pulse fans out to peers.
   This is what makes clustering federate — without it, each node clusters
   only its own graph.
3. **`Question` sender.** The `handle_question` path searches local vectors and
   replies with a `Sphere` (the `answer-` id convention). It needs a sender: on
   a local `query` that returns nothing hot enough, emit a `Question` carrying
   the reason vector. Today this is dead code — the handler is ready, the
   caller is missing.

**Done when.** Two nodes, A and B, on the same `network_id`. A query on A bumps
`access_count` for entity `e`. Within one heartbeat, B's `e.access_count`
reflects A's increment (verified by reading B's store). No full entity-body
broadcast was needed for that increment.

---

### F1 — Anti-entropy  *(a rejoining node catches up)*

**Problem.** A node that was partitioned and rejoins has no way to pull the
state it missed. `Fetch` works for a **single** thought by id
(`src/gossip/node.rs` `fetch_thought` — request/reply), but there is no bulk
"give me everything in kern X" pull. The CRDT plan calls this Stage 3
(`docs/kern/crdts-federation.md` §5.1, `AntiEntropy` kind) and notes it was
**not built.**

**Build.**
1. Add a `GossipKind::AntiEntropy` variant (or reuse `Fetch` with a wildcard
   `id`) carrying a `kern_id` + a vector clock / seen-set watermark of what the
   requester already has.
2. The responder diffs its kern against the watermark and streams the missing
   `EntitySyncPayload` chunks back (reuse the existing entity-body merge path,
   capped at the 50k remote-namespace ceiling already enforced).
3. Run anti-entropy on rejoin (a peer becomes reachable again) and
   periodically with exponential backoff if divergence persists.

**Done when.** Node A ingests 1000 entities, node B is offline. B comes back.
Within N seconds B's `remote-<A's network>-<A's kern>` namespace contains all
1000, verified by a query on B that returns all of A's content. No manual
resync step.

---

### F2 — Full CRDT fields, not just counters  *(no silent overwrites)*

**Problem.** Only `access_count` and `traversal_count` are G-Counters. Three
fields still use ad-hoc last-writer-wins and can lose concurrent writes:

- **`Entity.statements`** (`Vec<String>`, append-on-dedup). `src/base/merge.rs`
  does not merge `statements` at all today — two replicas that dedup the same
  incoming text concurrently each append; on entity-body sync the receiver's
  `statements` is whatever the sender shipped, not a union. Concurrent adds
  can be lost.
- **`Entity.valid_until`** — `join_min_time` in `merge.rs` is LWW by wall
  clock. Two producers racing to set validity on the same entity silently
  overwrite; min-join is not a correctness guarantee under skew.
- **`Reason.score`** — `refine_edges` sets `r.score = clamped` directly; there
  is no Lamport tiebreak, no merge rule. Two replicas refining the same edge
  produce two writes and one is lost on sync.

**Build (per `docs/kern/crdts-federation.md` §3, Stage 2).**
1. **OR-Set** primitive in `src/crdt.rs` for `statements` (keyed by
   `(text_hash, replica_id, lamport)`). `merge_entity` unions instead of
   overwriting. Tombstone growth is bounded by the existing cold-tier/decay
   path (note as a follow-up: OR-Set compaction using `valid_until`).
2. **LWW-Register** primitive with a `(lamport, producer_id)` tiebreak, not
   wall clock. Apply to `valid_until` and `Reason.score`.
3. **Lamport clock** — one `AtomicU64` per node, bumped on every local
   mutation and on every incoming delta (`max(local, remote) + 1`). Travels in
   every `CrdtDeltaPayload` (extend the struct) and is the tiebreak in all LWW
   registers. This replaces the implicit wall-clock LWW that clock skew can
   break.

**Done when.** Two replicas concurrently ingest the same content (same
`content_hash`) and concurrently refine the same reason edge. After sync, both
replicas agree on a single `statements` set (no lost appends) and a single
`Reason.score` (no lost rating), deterministically (same result regardless of
sync order). Verified by a partition-then-rejoin test.

---

### F3 — Transport security  *(safe off the trusted LAN)*

**Problem.** `docs/FEDERATION-SECURITY.md` is explicit: gossip is
**unauthenticated and unencrypted.** `network_id` is broadcast in cleartext
over UDP discovery. Transport is raw TCP. Anyone on the segment can sniff all
federated knowledge, and a peer cannot prove which node authored an entity.
Signed payloads are "a known future effort" with a code pointer at
`gossip/handler.rs` `handle_entity_sync`.

**Build.**
1. **TLS on the TCP gossip port.** rustls (already a workspace transitive dep
   via reqwest's `rustls` feature) for the listener and the dialer. mTLS so
   every peer presents a cert — this gives peer authentication, which is the
   thing that is completely missing today.
2. **Payload signatures.** Each node holds a keypair; every `GossipMessage`
   carries a signature over `(id, origin, payload)`. `handle_entity_sync` (and
   every handler) verifies before merging. This closes the "attacker-chosen
   metadata at insert time" gap the security doc names — a remote entity's
   producer becomes provable, not just claimed.
3. **`network_id` as a shared secret, not a broadcast.** Stop announcing it in
   cleartext on UDP discovery. Either derive it from the mTLS cert chain, or
   require manual `peers` config (the reliable path today) and drop multicast
   discovery for secured deployments.
4. **Document the threat model change.** Update `FEDERATION-SECURITY.md` so
   the "NOT protected" section shrinks to what remains (e.g. metadata traffic
   analysis) rather than "no auth, no encryption, no signatures."

**Done when.** Two nodes federate over an untrusted network with gossip bound
to a public interface, mTLS handshake required, payloads signed. An attacker
on the segment cannot read federated content (TLS), cannot inject entities
(signature verification rejects), and cannot impersonate a peer (cert
rejection). A test with a third node presenting a self-signed cert is refused.

---

### F4 — Backpressure and divergence bounds  *(survive a flood)*

**Problem.** The defenses today are bounds, not flow control: 50k entity cap
per remote namespace, `GOSSIP_CRDT_DELTA_MAX` clamp on inbound counters, a
seen-set with a TTL and a hard count ceiling. A malicious or buggy peer can
fill the remote namespace to cap and pin heat high (heat joins by `max`). There
is no rate limit on inbound, no per-peer quota, no divergence alarm.

**Build.**
1. **Per-peer inbound rate limit.** Track messages/sec per peer in the
   `Ledger` (`src/gossip/ledger.rs`); above a configurable ceiling, drop
   further messages from that peer for a backoff window and log.
2. **Divergence metric.** On each anti-entropy round (F1), record how many
   entities the pull delivered vs. already had. Expose via `health` as a
   federation divergence counter so an operator sees a stuck pair.
3. **Heat floor for remote entities.** Remote heat joins by `max`, but a
   remote entity should never exceed the local heat of the hottest local peer
   unless it's actually accessed locally. Consider decaying remote heat
   independently, or capping remote heat at ingest to the sender's claimed
   value (already clamped) and letting local access raise it from there.

**Done when.** A peer floods 100k entities/sec; the receiver drops above the
rate limit without unbounded memory growth, the remote namespace stays under
cap, and `health` reports the divergence. No panic, no lock poisoning (the
existing poison-tolerant handler guarantee holds).

---

## Priority summary

| Priority | Workstream | What it fixes |
|----------|-----------|---------------|
| F0 | Live delta senders | dead wire comes alive; G-Counter convergence actually happens |
| F1 | Anti-entropy | a rejoining node catches up instead of staying divergent forever |
| F2 | Full CRDT fields | concurrent `statements`/`valid_until`/`Reason.score` writes stop silently overwriting |
| F3 | Transport security | federation becomes safe off a trusted LAN/WireGuard mesh |
| F4 | Backpressure & divergence | a flood or a buggy peer can't exhaust the receiver |

## The one decision that gates production

**F3 (transport security) gates any deployment outside a fully trusted
network.** Without it, federation is documented as "trusted LAN only —
equivalent to an NFS export with no auth." If the product needs federation
across a VPC, a WAN, or any segment that is not physically private, F3 is
first. If federation only ever runs inside a WireGuard mesh the operator
controls, F3 can defer and **F0 + F1 + F2 are the real work** — those are what
make federation *correct*, F3 makes it *safe*.

Everything above is additive to the existing wire enum and the existing merge
path; none requires a breaking protocol bump except where noted (the
`Lamport` field on `CrdtDeltaPayload`). Keep the content-addressed id
contract — ids are `sha256` of content, which is what makes the union-merge
conflict-free, and every change here must preserve it.

<recap>
goal: Write a markdown doc in ~/dev/agentic/kern/docs/ describing what Kern needs to make federation production-ready.
current: Investigated federation code (gossip handlers, senders, CRDT layer, merge, security doc) and confirmed all gaps against real code; wrote the doc.
next: Verify the doc reads cleanly and is accurate, then goal_complete.
</recap>