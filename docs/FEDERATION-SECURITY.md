# Federation security — operator guide

> **Scope.** This document describes the trust model of the gossip federation
> **as implemented today**, for an operator deciding whether and how to enable
> it. This file describes the code that actually runs.

## TL;DR

- **Gossip is off by default** (`[gossip] enabled = false`). Single-node kern
  never opens a network port.
- When enabled, federation is **unauthenticated and unencrypted**. Treat the
  gossip network as a **trusted LAN segment only** — equivalent in trust to an
  NFS export or an internal Redis with no auth.
- The blast radius of a malicious peer is **bounded but non-zero**: it can
  inject entities (with attacker-chosen metadata) into a quarantined remote
  namespace and pin their heat high; it cannot overwrite, delete, or downgrade
  your local thoughts, and it cannot raise the confidence of anything you
  already hold.

## What enabling gossip does

With `enabled = true`, a node:

1. Binds a TCP listener (default `0.0.0.0:7400`) for gossip messages.
2. Broadcasts a UDP discovery announce (default port `7475`) of the form
   `<network_id>:<tcp_addr>`, and listens for peers announcing the same
   `network_id`.
3. Heartbeats peers, broadcasts its root scope and hottest entity bodies, and
   merges inbound entity bodies from peers into a phantom
   `remote-<sender network_id>-<kern_id>` kern via the content-addressed CRDT
   (each sender's data stays in its own remote namespace).

## Trust model: what is and isn't protected

### Protected / bounded

- **Default-off.** No attack surface unless you opt in.
- **Namespacing by `network_id`.** Multicast discovery only auto-pairs nodes
  announcing the same `network_id`, and inbound data from a different
  `network_id` never mixes with your own — it lands in that network's separate
  `remote-*` namespace. This separates co-located deployments.
- **Local data is never overwritten by peers.** Remote entities land in a
  separate `remote-*` phantom kern. Merge is a content-addressed union (ids are
  `sha256` of content), so a peer cannot mutate or delete an id you already
  hold — only contribute to the remote namespace.
- **Remote-kern entity cap.** Each `remote-*` phantom kern holds at most a
  bounded number of entities (50k); once at cap, new remote ids are dropped,
  limiting flood amplification.
- **Confidence is never imported.** The CRDT join deliberately excludes
  confidence (`conf_alpha`/`conf_beta`) from remote merges — a peer inflating a
  claim's confidence cannot raise it on your replica.
- **CRDT delta clamping.** Inbound counter deltas are rejected above a hard
  ceiling, so a peer cannot pin a counter slot arbitrarily high in one message.
- **Seen-set loop suppression** with a TTL and a hard count ceiling, so
  replayed/looping messages are dropped and the set can't grow without bound.
- **Poison-tolerant handlers.** A panic processing one message no longer
  poisons shared locks or crashes the daemon (it degrades to a logged warning).

### NOT protected — assume a peer on the network can do these

- **No encryption.** Transport is raw TCP; discovery is plaintext UDP. Anyone
  who can sniff the segment sees all federated knowledge in cleartext.
- **`network_id` is not a secret.** It is broadcast in the clear in every
  discovery announce. It is a *grouping* key, not an access credential — anyone
  on the segment can read it and join that network.
- **No peer authentication, no payload signatures.** A node cannot prove which
  peer authored an entity. Signed payloads are a known future effort (see the
  comment at `gossip/handler.rs` `handle_entity_sync`); until then the id cap +
  remote-namespace scoping are the accepted bound.
- **Remote metadata is attacker-chosen at insert time.** A brand-new remote
  entity lands with whatever confidence, heat, and status its sender stamped on
  it, and **heat joins by `max`** on later merges. Do not treat a remote
  entity's confidence or heat as a trust signal — it reflects its most
  optimistic (or malicious) source, not consensus.
- **Content is accepted on cap, not verified.** Entity bodies are accepted up to
  the cap without semantic verification (an intentional, documented decision —
  see the EntitySync content-verification note in the git history).

## Deployment guidance

- **Only enable on a network segment where you trust every host.** Home/lab
  LAN, a private VPC subnet, or a WireGuard mesh — not a coffee-shop Wi-Fi, not
  a shared office VLAN, not the public internet.
- **Do not bind to a public interface.** The default `0.0.0.0:7400` listens on
  all interfaces. On a multi-homed host, set `addr` to a specific private
  interface, and firewall the gossip TCP port and the UDP discovery port to the
  trusted segment.
- **Use a distinct `network_id` per logical deployment** so unrelated kern
  fleets on the same segment do not merge.
- **If you need confidentiality or peer authentication today, provide it at the
  network layer** (run gossip only inside a WireGuard/VPN mesh). The protocol
  itself provides neither.
- **Keep it off if you don't need multi-node memory.** Single-node kern is the
  default and has no network exposure.

## Reporting

Report security issues in the federation path privately to the maintainers
before public disclosure.
