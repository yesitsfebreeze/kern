# src/gossip/ledger.rs — commentary

- `Index`: the `by_expiry` BTreeMap mirror replaced an O(n) min-scan for soonest-expiry eviction (now O(log n)).
- `Index::insert`: regression fixed here — the old evict-then-insert path ran eviction while at cap even when the put was an overwrite, so re-putting "a" could evict "b"; the expiry-index path removes the prior entry for the key first. Pinned by test `overwriting_a_key_does_not_evict_another_entry`.
- `live_addr`: pulled out as a free fn so TTL semantics are unit-testable with an injected `now` instead of waiting on a wall-clock TTL.


Second-pass migration:
- `Index` doc compressed. Details moved here: soonest-expiring entry is `by_expiry`'s first element (O(log n) eviction); the `(Instant, String)` composite key makes eviction order total and deterministic when two entries share an Instant; the lockstep invariant means `by_expiry` never holds a stale `(expiry, key)` for a removed/overwritten entry.
- `Ledger` doc compressed. TTLs: `entities` uses `LEDGER_THOUGHT_TTL`, `routing` uses `LEDGER_ROUTING_TTL`. All entries are advisory; a lookup past the TTL returns `None` and the stale entry is swept lazily on the next capacity eviction, not eagerly.
- `put_*` / `lookup_*` / `lookup` / `live_addr` docs deleted (restated the code; eviction behavior lives on `Index::insert`).
