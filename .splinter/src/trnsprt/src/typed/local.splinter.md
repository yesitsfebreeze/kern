# src/trnsprt/src/typed/local.rs — commentary

- `Endpoint::kern`: why per-cwd, not per-user — a per-user endpoint would let the first daemon win for the whole user, so a second project would silently attach to the first project's graph (cross-project memory contamination). Per-cwd scoping also lets multiple daemons coexist on one host (e.g. local federation testing). The old module doc claimed `$XDG_RUNTIME_DIR/kern.sock` / `\\.\pipe\kern-<USERNAME>` endpoints; that predated the cwd tag and was deleted as stale.
- `LocalAdapter`: exists so both the server (accept) and client (connect) paths return a single concrete type that `Channel::new` — generic over `A: Adapter` — can consume directly.
- `a_second_bind_of_the_same_pipe_reports_already_running` (test): relies on `first_pipe_instance(true)` making the OS reject a second owner of the same pipe name while the first instance is alive.

Second-pass migration: `Endpoint::kern` doc compressed to one line (per-cwd rationale already recorded above). `cwd_tag` doc trimmed to the FNV-1a/stable-across-processes trap. `bind_kern_listener` / `LocalListener` docs compressed to the Unix-vs-Windows split.
