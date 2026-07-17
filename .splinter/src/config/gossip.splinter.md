# splinter: src/config/gossip.rs

[gossip] is opt-in federation over TCP + LAN multicast discovery, OFF by default — a lone daemon opens no socket and never announces itself. Field semantics:
- enabled: master switch; false runs no node, listener, or discovery.
- addr: TCP bind for the gossip listener; :0 picks an ephemeral port.
- discovery: LAN multicast — advertise, and auto-add same-network-id peers.
- network_id: discovery id shared by daemons that should pool. Unset announces the graph's per-daemon UUID, so independent daemons never auto-pair.
- discovery_port: UDP port for discovery announce/listen.
- peers: seed peers (host:port) dialled on startup, in addition to discovery.
effective_network_id wire-format hazard kept in source: a ':' in the id corrupts the kern:<id>:<addr> announce format, so an invalid id falls back to the generated one.
