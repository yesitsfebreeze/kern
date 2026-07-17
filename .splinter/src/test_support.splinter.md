# src/test_support.rs — commentary

Consolidation origin: `entity_vec` replaced the local `fn ent(id, vector)` fixtures that several `base`/`retrieval`/`tick` test modules each open-coded; `edge` did the same for `reason`/`pagerank` test modules; `spawn_http` replaced the per-module `serve(app)` boilerplate that hand-rolled the same bind-127.0.0.1:0 → local_addr → spawn → format URL dance.
## Design context (moved from source doc comments)

- Module: shared test-only helpers, compiled only under `#[cfg(test)]`.
- `edge(from, to)` builds a default `Reason` edge `from -> to` with id `"{from}->{to}"`.
- `spawn_http(app)`: binds an axum app to an ephemeral localhost port; returns its base URL and the task handle. Dropping the handle detaches — the stub keeps serving.
