# src/base/descriptors.rs — commentary

- Module placement: descriptors are *behaviour* (a builder + a registration helper), not pure data, so they live in this module rather than in `constants.rs`.

Second-pass migration:
- Module `//!`: compressed to the mapping contract (SOURCE_* kind → distiller extraction hint). The descriptor strings themselves are the documentation of what each hint asks for.
- `default_descriptors`: doc deleted as a name restatement.
- `register_default_descriptors`: "so the call is idempotent" dropped — the return value (count newly inserted) and the `Entry::Vacant` guard say it; the idempotency itself is locked by `register_into_empty_map_inserts_all_defaults`.
- test `default_descriptors_have_no_colliding_keys`: kept the collision trap in 2 lines. Full history — the count is 15 because there are 15 distinct source kinds; a count below 15 means two `SOURCE_*` consts resolved to the same string and one silently overwrote the other in the map. That actually happened (the AGENT_SOURCE/SOURCE_AGENT duplicate regression), which is why the test asserts an exact count rather than a lower bound, and why `constants.rs` carries a matching do-NOT-duplicate note next to `AGENT_SOURCE`.
- test `register_into_empty_map_inserts_all_defaults`: the "Idempotent: a second registration inserts nothing new" label deleted — `assert_eq!(register_default_descriptors(&mut m), 0)` is self-evident.
