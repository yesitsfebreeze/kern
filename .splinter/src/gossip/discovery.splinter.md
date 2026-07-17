# src/gossip/discovery.rs — commentary

- `start_listen`: `socket.recv_from` is awaited directly inside the `select!` on purpose — an earlier version used `set_nonblocking` + sleep-drain polling, and a blocking recv would pin a worker thread off the async executor.


Second-pass migration:
- `start_broadcast` doc compressed: sends to `GOSSIP_DISCOVERY_MULTICAST:port` from an ephemeral UDP socket; the spawned task runs until the node's stop signal fires (same for `start_listen`).
- `parse_announce` doc compressed: because the split is at the first ':' and ids are colon-free, operator-configured ids of any length work, not just the generated 36-char UUID form.
