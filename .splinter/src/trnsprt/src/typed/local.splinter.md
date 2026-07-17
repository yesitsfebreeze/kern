# src/trnsprt/src/typed/local.rs — commentary

- `Endpoint::kern`: why per-cwd, not per-user — a per-user endpoint would let the first daemon win for the whole user, so a second project would silently attach to the first project's graph (cross-project memory contamination). Per-cwd scoping also lets multiple daemons coexist on one host (e.g. local federation testing). The old module doc claimed `$XDG_RUNTIME_DIR/kern.sock` / `\\.\pipe\kern-<USERNAME>` endpoints; that predated the cwd tag and was deleted as stale.
- `LocalAdapter`: exists so both the server (accept) and client (connect) paths return a single concrete type that `Channel::new` — generic over `A: Adapter` — can consume directly.
- `a_second_bind_of_the_same_pipe_reports_already_running` (test): relies on `first_pipe_instance(true)` making the OS reject a second owner of the same pipe name while the first instance is alive.

Second-pass migration: `Endpoint::kern` doc compressed to one line (per-cwd rationale already recorded above). `cwd_tag` doc trimmed to the FNV-1a/stable-across-processes trap. `bind_kern_listener` / `LocalListener` docs compressed to the Unix-vs-Windows split.
Design notes (moved from source comments during comment sweep):
- Local-socket transport for the kern singleton daemon: per-cwd endpoint resolution, Unix/named-pipe Adapter impls, singleton-aware bind/accept.
- Endpoint = platform-specific endpoint location for a singleton local daemon. Endpoint::kern() = per-cwd endpoint; the cwd hash is folded into the socket/pipe name so each project gets its own daemon. Endpoint::display() = human-readable identifier for logs/error messages.
- cwd_tag (KEPT tightened in source): FNV-1a over the canonical cwd — MUST stay stable across processes (unlike DefaultHasher's randomized state) so daemon and clients always agree.
- LocalAdapter = platform-tagged local-socket adapter. connect_kern connects to a kern singleton at an endpoint and returns a LocalAdapter ready to wrap in a Channel with any codec.
- BindOutcome::Bound = endpoint bound; caller now owns the singleton and may LocalListener::accept. BindOutcome::AlreadyRunning = another live daemon already owns the endpoint; caller should exit 0.
- bind_kern_listener is singleton-aware: on AddrInUse it probes whether a live daemon owns the socket (a successful connect -> AlreadyRunning); a stale socket file is removed and rebound once. Windows uses first_pipe_instance(true) to let the OS enforce uniqueness. Magic os errors handled: 5 = ERROR_ACCESS_DENIED, 231 = ERROR_PIPE_BUSY.
- LocalListener = unified local-socket listener: Unix drives a UnixListener; Windows holds the current NamedPipeServer and re-creates one per accept. KEPT in source: pre-create the next instance so a subsequent accept doesn't race a fast reconnect; and Drop does best-effort socket-file cleanup so the next daemon doesn't trip the stale-sock probe.
- test a_stale_socket_file_is_removed_and_rebound: binds then drops a raw UnixListener WITHOUT LocalListener's Drop, leaving the socket file on disk with nothing listening (a stale endpoint).
