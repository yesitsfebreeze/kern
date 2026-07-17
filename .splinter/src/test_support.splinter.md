# src/test_support.rs — commentary

Consolidation origin: `entity_vec` replaced the local `fn ent(id, vector)` fixtures that several `base`/`retrieval`/`tick` test modules each open-coded; `edge` did the same for `reason`/`pagerank` test modules; `spawn_http` replaced the per-module `serve(app)` boilerplate that hand-rolled the same bind-127.0.0.1:0 → local_addr → spawn → format URL dance.
