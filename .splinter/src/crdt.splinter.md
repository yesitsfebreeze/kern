Second-pass migration:
- Module doc compressed. CRDT properties moved here: GCounter/PnCounter are conflict-free, commutative, idempotent, monotone primitives — merge is per-slot max, so replicas converge to the same value regardless of gossip delivery order or duplication.
- `merge_is_commutative_and_order_independent`: deleted narration; contract is three absolute-total deltas across two replicas applied in two different orders (one duplicated) must converge to identical state.
## Design context (moved from source doc comments)

- Module: grow-only (`GCounter`) and positive-negative (`PnCounter`) CRDT counters backing the per-replica access/traversal counts merged by `base::merge`.
- Test helper `slot(replica, value)` builds a single-replica absolute-value slot — the shape inbound CRDT deltas merge as.
