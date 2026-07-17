# src/gossip/node.rs — commentary

- `peer_count`: exists for the peer-exchange loop's capacity break, which only needs the length — `peer_list().len()` would clone the whole `Vec<String>` (every address) per iteration.


Second-pass migration:
- `peer_count` inline doc ("without cloning the list") deleted — rationale already recorded above.
