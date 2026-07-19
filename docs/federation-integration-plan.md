# Federation integration plan — making gossip usable

> **What this is.** An audit of every claim in `docs/federation-roadmap.md`
> against the actual codebase at v1.0.0, followed by a concrete integration
> plan for turning federation from "trusted-LAN-only, half the wire dead" into
> something correct and safe to deploy. Every claim below was verified against
> real source — file paths, line numbers, and grep evidence are the record.
>
> **Companion docs.** The design study is `docs/kern/crdts-federation.md`.
> The operator trust guide is `docs/FEDERATION-SECURITY.md`. The gap list is
> `docs/federation-roadmap.md` (F0–F4). This doc is the plan to close the gap.

---

## Audit: roadmap claims vs actual source

### F0 — Live delta senders

| Roadmap claim | Verified | Evidence |
| --- | --- | --- |
| `Delta`, `Question`, `Pulse` have handlers but no sender | ✓ | `splinter_grep_files "GossipKind::(Delta\|Question\|Pulse)"` → only `handler.rs:new_handler` match arms + test constructors. No production emitter. |
| `start_heartbeat` broadcasts `PeerExchange` | ✓ | `src/gossip/node.rs` `start_heartbeat` — `GossipKind::PeerExchange` on `GOSSIP_HEARTBEAT_INTERVAL` |
| `start_announce` broadcasts `Sphere` | ✓ | `src/gossip/handler.rs:50` `start_announce` — `GossipKind::Sphere` on heartbeat interval |
| `start_entity_sync` broadcasts hottest 32 entities | ✓ | `src/gossip/handler.rs:79` — sorts by `cmp_rank(heat)`, `truncate(32)`, `GossipKind::EntitySync` |
| `commit_access` bumps `access_count` but emits no `Delta` | ✓ | `src/retrieval/score.rs:186` `stamp_access` — `e.access_count.increment(replica, 1)`, no gossip call. `commit_access_ids_with_half_life` same — no broadcast. |
| Local pulse does not fan out to peers | ✓ | `tick::pulse::pulse` called from `commands.rs:spawn_maintenance_tick` and `mcp/tools_admin.rs:Server.tool_pulse` — neither emits `GossipKind::Pulse`. Only `handler.rs` receives. |
| `handle_question` searches local vectors, replies with `Sphere` | ✓ | `src/gossip/handler.rs:176` `handle_question` — `search_all_unlocked`, reply is `GossipKind::Sphere` with `answer-` id convention |
| `CrdtDeltaPayload.value` is the sender's absolute slot total | ✓ | `src/gossip/types.rs` comment + `crdt.rs` `GCounter::merge` is per-slot max — a delta-since-last would be lost |
| `CrdtTarget` only has `ThoughtAccessCount` and `ReasonTraversalCount` | ✓ | `src/gossip/types.rs` — `CrdtTarget` enum, two variants |
| `CrdtDeltaPayload` has no Lamport field | ✓ | `src/gossip/types.rs` — struct has `kern_id, object_id, target, replica, value` only |

**Verdict: roadmap F0 is accurate.** The G-Counter convergence the CRDT layer
was built for never happens because no sender emits deltas after
`stamp_access` increments the counter.

### F1 — Anti-entropy

| Roadmap claim | Verified | Evidence |
| --- | --- | --- |
| `Fetch` works for a single thought by id | ✓ | `src/gossip/node.rs` `fetch_thought` — `FetchPayload { resource: "thought", id: entity_id }`, request/reply via `send_and_receive` |
| No bulk "give me everything in kern X" pull | ✓ | `handle_fetch` calls `fetch_handler(resource, id)` — single resource, single id. No wildcard, no batch. |
| No `AntiEntropy` wire kind | ✓ | `GossipKind` enum in `types.rs` has 7 variants: Sphere=0, Question=1, Pulse=2, PeerExchange=3, Fetch=4, Delta=5, EntitySync=6. No `AntiEntropy`. |
| CRDT plan names Stage 3 anti-entropy as not built | ✓ | `docs/kern/crdts-federation.md` §5.1 names `AntiEntropy = 6` as a new variant; §6 Stage 3 "anti-entropy (2 days)" is listed as future work |

**Verdict: roadmap F1 is accurate.** A rejoining node has no catch-up path
beyond single-thought fetch.

### F2 — Full CRDT fields

| Roadmap claim | Verified | Evidence |
| --- | --- | --- |
| `crdt.rs` ships only `GCounter` and `PnCounter` | ✓ | `src/crdt.rs` — two structs, no OR-Set, no LWW-Register |
| `merge_entity` does not merge `statements` | ✓ | `src/base/merge.rs:35` `merge_entity` — no mention of `statements`. `Entity.statements` is `Vec<String>` at `types.rs:254`; never unioned. |
| `valid_until` uses `join_min_time` (LWW by wall clock) | ✓ | `src/base/merge.rs:60` — `join_min_time(&mut local.valid_until, remote.valid_until)`. `join_min_time` compares `SystemTime` directly. |
| `Reason.score` has "no merge rule" | **✗ partially** | `src/base/merge.rs:65` `merge_reason` — `if remote.score > local.score { local.score = remote.score }` — this IS a max-join merge rule. The roadmap says "no merge rule" but there is one; it's the **wrong** rule. See correction below. |
| `refine_edges` sets `r.score = clamped` directly | ✓ | `src/retrieval/answer.rs:360` — `r.score = clamped`, no Lamport, no CRDT |
| Confidence is never imported from remote | ✓ | `src/base/merge.rs:38` comment + test `merge_does_not_import_remote_confidence` — `conf_alpha`/`conf_beta`/`unlinked_count` excluded from merge |

**Correction — F2 `Reason.score`:** The roadmap says "no merge rule" but
`merge_reason` does a max-join (`remote.score > local.score`). The real
problem is deeper than the roadmap states: **max-join is the wrong merge
semantics for `Reason.score`** because score is *not monotonic*.

- `refine_edges` can set score to any value the LLM returns (up or down).
- `degrade_entity_reasons` (`src/commands/graph_ops.rs:236`) *lowers* scores
  (`r.score -= decay`) and removes edges below `DEGRADE_MIN_THRESHOLD`.
- Under max-join, a deliberate degrade (lowering) is **irreversibly lost** on
  sync — the peer's stale higher score wins and overwrites the decay.

So the roadmap's conclusion ("one is lost on sync") is correct, but the
mechanism is "wrong merge rule" not "no merge rule." The fix is the same
the roadmap proposes: LWW-Register with `(lamport, producer_id)` tiebreak,
which handles both raises and lowers.

**Verdict: roadmap F2 is accurate in conclusion, wrong on one detail.** The
`statements` gap and `valid_until` wall-clock LWW are exactly as described.
`Reason.score` needs LWW-Register, not because there's no merge rule, but
because max-join is semantically wrong for a non-monotonic field.

### F3 — Transport security

| Roadmap claim | Verified | Evidence |
| --- | --- | --- |
| Transport is raw TCP | ✓ | `src/gossip/transport.rs` — `TcpStream::connect`, `TcpListener::bind`, no TLS layer |
| `network_id` broadcast in cleartext over UDP | ✓ | `src/gossip/discovery.rs:21` — `format!("{ANNOUNCE_PREFIX}{}:{}", node.network_id, node.addr())` sent via `UdpSocket::send_to` to multicast group |
| No payload signatures | ✓ | `GossipMessage` struct in `types.rs` — `kind, id, origin, payload`. No signature field. |
| No peer authentication | ✓ | `node.rs:handle_conn` accepts any `TcpStream`, `handle_peer_exchange` adds any `msg.origin` as peer. No cert, no handshake. |
| Signed payloads are "a known future effort" | ✓ | `handler.rs:130` comment on `handle_entity_sync`: "Content↔id binding is NOT verified — network scoping + GOSSIP_REMOTE_KERN_ENTITY_CAP bound it." |
| rustls is a transitive dep via reqwest | ✓ | `Cargo.toml` — `reqwest = { features = ["rustls", ...] }`. But `rustls`/`tokio-rustls` are NOT direct deps; would need adding. |

**Verdict: roadmap F3 is accurate.** Gossip is unauthenticated, unencrypted,
and `network_id` is a broadcast grouping key, not a secret.

### F4 — Backpressure and divergence bounds

| Roadmap claim | Verified | Evidence |
| --- | --- | --- |
| 50k entity cap per remote namespace | ✓ | `constants.rs` — `GOSSIP_REMOTE_KERN_ENTITY_CAP = 50_000`. Enforced in `merge.rs:merge_remote_entity`. |
| `GOSSIP_CRDT_DELTA_MAX` clamp on inbound counters | ✓ | `constants.rs` — `GOSSIP_CRDT_DELTA_MAX = 1_000_000`. Enforced in `handler.rs:validated_delta_value`. |
| Seen-set with TTL and hard count ceiling | ✓ | `constants.rs` — `GOSSIP_SEEN_SET_CAP = 10_000`, `GOSSIP_SEEN_TTL = 60s`. `seen.rs` enforces both. |
| No per-peer inbound rate limit | ✓ | `src/gossip/ledger.rs` — `Ledger` has `entities` and `routing` indexes with TTL+cap. No message-rate tracking. |
| No divergence alarm | ✓ | `src/base/health.rs` `HealthStats` — `kerns, entities, reasons, unnamed, gravitons`. No federation divergence counter. `wire.rs` `HealthResponse` — no federation field. |
| Heat joins by `max` | ✓ | `merge.rs:41` — `if remote.heat > local.heat { local.heat = remote.heat; }` |

**Verdict: roadmap F4 is accurate.** Existing defenses are bounds (caps,
clamps, TTL), not flow control. A flood can fill the remote namespace to cap
and pin heat; there is no rate limit and no operator visibility into
divergence.

---

## Integration plan

Ordered by dependency: F0 and F2 are the correctness core (can ship
together). F1 depends on F0's delta senders to know what's missing. F3 is
orthogonal (can start in parallel). F4 is hardening on top of F0+F1.

### Phase 1 — Correctness core (F0 + F2)

These are the workstreams that make federation *correct*. Without them,
gossip broadcasts full entity bodies every 30s but never propagates
incremental counter state, and concurrent writes on `statements`/
`valid_until`/`Reason.score` silently lose data.

#### 1a. Lamport clock + `CrdtDeltaPayload` extension (F2 prerequisite)

**Why first:** F0's delta sender and F2's LWW-Register both need a Lamport
clock on the wire. Build it once.

**Change:**

1. Add `AtomicU64` Lamport clock to `Node` (`src/gossip/node.rs`).
   - `bump()` on every local mutation, `observe(remote)` on every incoming
     delta (`max(local, remote) + 1`).
2. Extend `CrdtDeltaPayload` (`src/gossip/types.rs`):

   ```rust
   pub struct CrdtDeltaPayload {
       pub kern_id: String,
       pub object_id: String,
       pub target: CrdtTarget,
       pub replica: String,
       pub value: u64,
       pub lamport: u64,       // NEW
       pub producer: String,   // NEW: tiebreak for LWW
   }
   ```

   - This is a wire-format change. Bump `wire::VERSION` or gate by feature
     flag. The existing `repr(u8)` `GossipKind` is unchanged (additive).
3. Add `LWW-Register` and `OR-Set` primitives to `src/crdt.rs`:
   - `LwwRegister<T>` with `(lamport, producer_id)` tiebreak.
   - `OrSet<K>` with `(key, tag)` add-remove semantics.
   - Both `merge(&mut self, &other) -> bool`, same convention as `GCounter`.

#### 1b. Delta sender on counter increment (F0.1)

**Change:**

1. Wire a `broadcast_delta` closure into `Deps` (`src/gossip/handler.rs`),
   same pattern as the existing `save` closure.
2. After `stamp_access` (`src/retrieval/score.rs:186`) increments
   `access_count`, call the closure with a `CrdtDeltaPayload` for
   `(object_id, CrdtTarget::ThoughtAccessCount, replica, value)`.
   - `value` = the sender's absolute slot total (per the existing comment).
   - Throttle: one delta per object per heartbeat, coalesced. Add a
     `pending_deltas: Mutex<HashMap<String, CrdtDeltaPayload>>` to `Node`
     that drains on the heartbeat tick.
3. Same for `Reason.traversal_count` — find every increment site (the
   traversal path) and emit `CrdtTarget::ReasonTraversalCount`.

**Done when:** Two nodes, A and B, same `network_id`. Query on A bumps
`access_count` for entity `e`. Within one heartbeat, B's `e.access_count`
reflects A's increment. No full entity-body broadcast needed.

#### 1c. Pulse sender (F0.2)

**Change:**

1. In `commands.rs:spawn_maintenance_tick` and
   `mcp/tools_admin.rs:Server.tool_pulse`, after the local
   `tick::pulse::pulse` call, broadcast a `GossipKind::Pulse` carrying
   `(kern_id, strength)` to peers via `node.broadcast`.
   - This requires `Node` to be accessible from these call sites. Thread it
     through the same `Deps`/closure pattern, or store `Node` on the
     `SharedGraph`/daemon context.

**Done when:** A local pulse on node A causes node B to pulse its
corresponding kern. Clustering federates — each node clusters not only its
own graph but receives peer pulses.

#### 1d. Question sender (F0.3)

**Change:**

1. In the query path (where a local `query` returns nothing hot enough),
   emit a `GossipKind::Question` carrying the reason vector.
   - The `handle_question` receiver is already built (`handler.rs:176`).
   - Needs the query miss-detection hook: after `search_all_unlocked`
     returns below `QUESTION_RESOLVE_THRESHOLD`, broadcast.

**Done when:** A query on A that misses locally triggers a `Question` to
peers. B has a hit and replies with an `answer-` Sphere. A resolves the
question edge.

#### 1e. OR-Set for `statements` (F2.1)

**Change:**

1. Replace `Entity.statements: Vec<String>` with `OrSet<String>` (or add
   an OR-Set shadow field).
   - Keyed by `(text_hash, replica_id, lamport)` per the CRDT plan §3.
2. `merge_entity` (`src/base/merge.rs:35`) unions the OR-Set instead of
   ignoring `statements`.
3. The `Entity.text()` reader (`types.rs:302`) materializes from the
   OR-Set — join the live statements.
4. Tombstone growth: bounded by the existing cold-tier/decay path. Note as
   follow-up: OR-Set compaction using `valid_until`.

**Done when:** Two replicas concurrently dedup the same incoming text. After
sync, both agree on a single `statements` set (no lost appends), regardless
of sync order.

#### 1f. LWW-Register for `valid_until` and `Reason.score` (F2.2)

**Change:**

1. `valid_until`: replace `join_min_time` in `merge_entity` with
   `LwwRegister` merge using `(lamport, producer_id)` tiebreak.
2. `Reason.score`: replace the max-join in `merge_reason` with
   `LwwRegister` merge. This is the **critical fix** — max-join is
   semantically wrong because `degrade_entity_reasons` lowers scores and
   `refine_edges` sets arbitrary values. LWW-Register with Lamport ensures
   the *newest* write wins, not the *highest*.
3. `refine_edges` (`answer.rs:360`) and `degrade_entity_reasons`
   (`graph_ops.rs:255`) must stamp `(lamport, producer_id)` on every write.

**Done when:** Two replicas concurrently refine the same reason edge and one
degrades it. After sync, both agree on a single `Reason.score`
deterministically (newest by Lamport wins, not highest by value).

### Phase 2 — Catch-up (F1)

Depends on F0 (delta senders) to know what state a rejoining node missed.

#### 2a. `AntiEntropy` wire variant + bulk pull

**Change:**

1. Add `GossipKind::AntiEntropy = 7` to the enum (`types.rs`). Additive,
   no renumber.
2. Add `AntiEntropyPayload`:

   ```rust
   pub struct AntiEntropyPayload {
       pub kern_id: String,
       pub watermark: Watermark,  // vector clock or seen-set snapshot
   }
   ```

3. Responder (`handler.rs` new `handle_anti_entropy`): diffs its kern
   against the watermark, streams missing `EntitySyncPayload` chunks back
   (reuse `merge_remote_entity`, capped at `GOSSIP_REMOTE_KERN_ENTITY_CAP`).
4. Trigger on rejoin: when a peer becomes reachable again (heartbeat
   succeeds after failure), initiate an anti-entropy round.
5. Periodic with exponential backoff if divergence persists.

**Done when:** Node A ingests 1000 entities. B is offline, comes back.
Within N seconds B's `remote-<A's network>-<A's kern>` namespace contains
all 1000, verified by a query on B returning all of A's content.

### Phase 3 — Transport security (F3)

Orthogonal to Phase 1+2. Can start in parallel once the wire-format changes
in 1a are settled (signatures ride on the same envelope).

#### 3a. mTLS on the TCP gossip port

**Change:**

1. Add direct deps: `tokio-rustls`, `rustls` (pem), `rcgen` (dev/test cert
   generation).
2. `transport.rs`: wrap `TcpListener`/`TcpStream` in
   `tokio_rustls::TlsAcceptor` / `TlsConnector`.
3. `GossipConfig` (`src/config/gossip.rs`): add `tls_cert`, `tls_key`,
   `tls_ca` (peer cert chain to trust). When set, gossip requires mTLS.
4. `Node::listen` and `send_msg` / `send_and_receive` negotiate TLS. A peer
   without a valid cert is refused at the handshake.

#### 3b. Payload signatures

**Change:**

1. Add `signature: Vec<u8>` to `GossipMessage` (`types.rs`).
2. Each node holds a keypair (derive from cert or separate ed25519 key).
3. Sign `(id, origin, payload)` on send; verify on receive in
   `handle_conn` / `new_handler` before dispatch.
4. `handle_entity_sync` (and all handlers) verify before merging. This
   closes the "attacker-chosen metadata at insert time" gap — a remote
   entity's producer becomes provable.

#### 3c. `network_id` as shared secret

**Change:**

1. Stop broadcasting `network_id` in cleartext (`discovery.rs`).
2. Either derive `network_id` from the mTLS cert chain (cert SAN matches a
   configured network name), or require manual `peers` config and disable
   multicast discovery for secured deployments.
3. `GossipConfig.discovery = false` when TLS is enabled, or gate discovery
   behind a `discovery_secret` that must match.

#### 3d. Update `FEDERATION-SECURITY.md`

Shrink the "NOT protected" section to what remains (metadata traffic
analysis) once TLS + signatures + cert auth land.

**Done when:** Two nodes federate over an untrusted network with mTLS +
signed payloads. A third node with a self-signed cert is refused. An
attacker on the segment cannot read, inject, or impersonate.

### Phase 4 — Backpressure hardening (F4)

Depends on F0 (delta senders) and F1 (anti-entropy) to have meaningful
divergence to measure.

#### 4a. Per-peer inbound rate limit

**Change:**

1. Track messages/sec per peer in the `Ledger` (`src/gossip/ledger.rs`) —
   add a `rate: RwLock<HashMap<String, PeerRate>>` tracking a sliding
   window.
2. Above a configurable ceiling (`GOSSIP_PEER_RATE_LIMIT`), drop further
   messages from that peer for a backoff window and log.
3. `handle_conn` checks the rate limiter before dispatch.

#### 4b. Divergence metric in `health`

**Change:**

1. On each anti-entropy round (F1), record entities-pulled vs. already-had.
2. Add `federation_divergence: i64` to `HealthStats` (`health.rs`) and
   `HealthResponse` (`wire.rs`).
3. Surface via the `health` MCP tool so an operator sees a stuck pair.

#### 4c. Heat floor for remote entities

**Change:**

1. Cap remote heat at ingest to the sender's claimed value (already
   clamped by `GOSSIP_CRDT_DELTA_MAX` on counters, but heat is `f64` and
   unclamped).
2. Consider decaying remote heat independently — a remote entity should
   not exceed the local heat of the hottest local peer unless accessed
   locally. Local access raises it from the floor; no remote-only heat
   inflation.

**Done when:** A peer floods 100k entities/sec. The receiver drops above the
rate limit without unbounded memory growth. The remote namespace stays
under cap. `health` reports the divergence. No panic, no lock poisoning.

---

## Sequencing and dependencies

```
Phase 1a (Lamport + wire) ──┬── Phase 1b (Delta sender)
                            ├── Phase 1c (Pulse sender)
                            ├── Phase 1d (Question sender)
                            ├── Phase 1e (OR-Set statements)
                            └── Phase 1f (LWW-Register score/valid_until)

Phase 1 ───────────────────── Phase 2 (Anti-entropy)

Phase 3 (Transport security) ── parallel, needs 1a wire envelope settled

Phase 4 (Backpressure) ── needs Phase 1b + Phase 2 for divergence signal
```

**Minimum viable federation:** Phase 1 (correctness) + Phase 2 (catch-up).
This makes federation *correct* — counters converge, no silent overwrites,
rejoining nodes catch up. It is still unauthenticated/unencrypted, so
trusted-LAN-only.

**Production-safe federation:** add Phase 3 (transport security). This is
the gate for any deployment outside a WireGuard mesh / trusted LAN.

**Hardened federation:** add Phase 4 (backpressure). Survives floods and
gives operators visibility.

---

## Wire-format impact

| Change | Breaking? | Mitigation |
| --- | --- | --- |
| `CrdtDeltaPayload` gains `lamport` + `producer` | Yes — struct layout change | Bump `wire::VERSION`, or gate by feature flag, or make fields `#[serde(default)]` for back-compat |
| `GossipKind::AntiEntropy = 7` | No — additive variant | `repr(u8)` append, old nodes ignore unknown kinds |
| `GossipMessage.signature` | Yes — new field | `#[serde(default)]` on the signature field; unsigned nodes still interop in a mixed fleet (but cannot verify) |
| `Entity.statements` → OR-Set | Yes — type change | Persist migration (bincode version bump). One-shot `kern migrate`. |

The content-addressed id contract (`sha256` of content) is preserved
throughout — it is what makes the union-merge conflict-free and must not
change.

---

## What is already done (don't rebuild)

- **GCounter / PnCounter** (`src/crdt.rs`) — correct, tested, merge is
  per-slot max + idempotent + commutative.
- **Content-addressed entity-body merge** (`merge.rs:merge_remote_entity`) —
  hijack protection (id owned by another kern → reject), cap enforcement,
  index-on-insert. This is the foundation anti-entropy (F1) will reuse.
- **Seen-set loop suppression** (`seen.rs`) — TTL + hard cap, tested under
  flood. Keep as-is.
- **Delta value clamping** (`handler.rs:validated_delta_value`) — rejects
  empty ids, zero, and values over `GOSSIP_CRDT_DELTA_MAX`. Keep.
- **Confidence isolation** (`merge.rs:38`) — `conf_alpha`/`conf_beta`/
  `unlinked_count` never imported from remote. This is correct and must
  survive every change above. Trust is replica-local.
- **Heat/status/superseded_by joins** (`merge.rs:merge_entity`) — heat
  max-join, status lattice, `superseded_by` lexicographic max. These are
  monotone and correct as-is (F4c addresses heat inflation, not the join
  itself).
- **Phantom kern namespacing** (`handler.rs:inject_remote_scope`) —
  `remote-<network_id>-<kern_id>` isolation. Keep.

---

## Open questions for `ROADMAP.md`

1. **Does `Reason.score` max-join get replaced by LWW-Register, or is
   there an argument for keeping max-join for monotonic trust signaling?**
   The `degrade` path (lowering) means max-join is wrong, but changing it
   is a semantic shift — a peer that degrades an edge will see its degrade
   propagate. This is the correct behavior but should be a deliberate
   decision.
2. **Anti-entropy watermark shape:** vector clock (per-replica Lamport
   frontier) or seen-set snapshot (content-hash bloom filter)? The CRDT
   plan §5.1 names both; the choice trades bandwidth for simplicity.
3. **TLS cert authority:** operator-provided PKI (each node gets a cert
   from the operator's CA), or self-signed with a trust-on-first-use pin?
   The threat model differs: TOFU is vulnerable to first-contact MITM.
4. **Does `network_id` derive from the cert, or stay config-owned?** If
   cert-derived, a node cannot join a different network without a new
   cert. If config-owned, `network_id` remains a grouping key and the
   cert provides transport auth but not network membership.
