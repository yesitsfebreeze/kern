# src/gossip/handler.rs — commentary

- `handle_crdt_delta`: the persist-on-change call exists because an earlier version merged counters in memory only, violating the `Deps.save` contract (federation mutations must survive restart); a regression test pins it.
- `handle_entity_sync` threat model (see also `base::merge::merge_remote_entity`): a remote peer cannot hijack a local-origin or other-network entity — the merge is scoped to that peer's own `remote-{net}-{kern}` phantom kern and rejects ids owned elsewhere — and cannot grow the graph unboundedly (`GOSSIP_REMOTE_KERN_ENTITY_CAP`). Content↔id binding is unverifiable here without the original creating text or a signature: the entity id is the sha256 of the original text, but `ingest::dedup` refines `statements` in place afterwards, so the id is not re-derivable from the transmitted body. Robust fix is signed gossip payloads (federation-auth effort, tracked with the CRDT ownership-auth item); until then scope + cap are the accepted bound.
- `resolve_question_from_peer`: reason, owning kern, and local network id are pulled under one read acquisition on purpose (previously locked once for `find_reason` and again for `network_id`).
- test `peer_exchange_caps_at_max_peers`: the handler loop breaks on `peer_count()` — a cheap length read that replaced a per-iteration `peer_list().clone()`.


Second-pass migration:
- `Deps.save` doc compressed; the callers are the federation mutations: remote scope inject, CRDT counter merges, question resolution, entity sync.
- `start_announce` / `start_entity_sync` docs compressed. Both loops tick every `GOSSIP_HEARTBEAT_INTERVAL` and run until the node's stop signal fires. Announce: without it a node only ever receives. Entity sync: sends the 32 hottest local entities (heat-sorted, truncated); receivers merge via `base::merge::merge_remote_entity` into per-network phantom kerns.
- `validated_delta_value` doc compressed. Rationale: because `value` is an absolute slot total merged via GCounter per-slot max, delivery in any order and with duplication converges (commutative + idempotent). Empty replica/object ids and zero are dropped as no-ops.
- `handle_entity_sync` doc compressed; full threat model already recorded above. Also: ignores own-network echoes and empty network ids; persists only when the merge changed the graph.
- `handle_crdt_delta` persist comment compressed; full rule: the save closure read-locks the graph, so calling it while the write guard is held deadlocks — drop the guard first, and skip persist when the merge was a no-op (idempotent re-delta).
