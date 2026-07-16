#!/usr/bin/env python3
"""Deterministic generator for scaled retrieval-bench traces.

The bench embedder (src/bench_support/embed.rs) is a hash stub: cosine
similarity tracks *token overlap*, not meaning. Ground truth is therefore made
retrievable by construction — every doc carries globally unique "anchor" tokens,
and each query reuses its target's anchors plus a few of its topic words. Docs in
different topics share almost no tokens, so a target's anchors dominate both the
dense and the BM25 lexical leg and rank it in the top-10. Near-duplicate clusters
share a cluster anchor, giving multi-relevant queries (2-4 expected ids).

Same args -> byte-identical output (single seeded random.Random stream, stable
iteration order, json.dumps with fixed separators).

Usage:
    gen_trace.py --docs N --queries M --seed S --name NAME --out PATH
"""

import argparse
import json
import random

# Topic-specific vocabulary pools. Docs draw filler words from one topic's pool,
# so same-topic docs share partial vocabulary (realistic distractors) while
# cross-topic docs are near-orthogonal.
TOPICS = {
    "db": [
        "btree", "page", "split", "compaction", "sstable", "wal", "fsync",
        "durability", "mvcc", "snapshot", "isolation", "vacuum", "tuple",
        "index", "heap", "planner", "cardinality", "histogram", "join",
        "partition", "checkpoint", "replication", "pagination", "keyset",
        "offset", "bloom", "lookup", "amplification", "cluster", "primary",
        "secondary", "columnar", "vectorized", "deadlock", "lock", "commit",
        "rollback", "buffer", "pool", "flush",
    ],
    "dist": [
        "raft", "paxos", "consensus", "quorum", "leader", "election", "vote",
        "gossip", "membership", "epidemic", "vector", "clock", "causal",
        "crdt", "merge", "commutative", "idempotent", "hashing", "ring",
        "replica", "handoff", "fencing", "token", "lease", "linearizable",
        "saga", "compensating", "partition", "split", "brain", "skew",
        "hybrid", "backpressure", "quorumread", "repair", "reconfiguration",
        "joint", "sloppy", "virtual", "node",
    ],
    "rs": [
        "borrow", "checker", "lifetime", "reference", "aliased", "mutable",
        "send", "sync", "thread", "pin", "future", "async", "monomorphization",
        "generic", "vtable", "dispatch", "trait", "object", "drop", "refcell",
        "unsafe", "miri", "arc", "atomic", "phantom", "niche", "slice",
        "bounds", "elided", "orphan", "coherence", "polonius", "datalog",
        "leak", "cycle", "sharing", "cacheline", "padding", "pipeline",
        "misprediction",
    ],
    "net": [
        "congestion", "slowstart", "quic", "stream", "udp", "handshake",
        "bgp", "hijack", "rpki", "nagle", "ack", "segment", "dns", "resolver",
        "ttl", "anycast", "prefix", "mtu", "fragmentation", "syn", "flood",
        "cookie", "backlog", "dpdk", "userspace", "poll", "ecn", "bbr",
        "bandwidth", "reuseport", "listener", "gro", "coalesce", "cdn",
        "websocket", "headofline", "blocking", "nat", "punching", "bucket",
    ],
    "ml": [
        "gradient", "allreduce", "tensor", "parallel", "shard", "precision",
        "bfloat", "loss", "scaling", "checkpoint", "activation", "kvcache",
        "paged", "attention", "dataloader", "prefetch", "pinned", "quantize",
        "int", "speculative", "decoding", "draft", "embedding", "vectorstore",
        "hnsw", "recall", "flash", "softmax", "sram", "batching", "utilization",
        "latency", "registry", "lineage", "straggler", "pipeline", "microbatch",
        "bubble", "nvlink", "interconnect",
    ],
    "bt": [
        "incremental", "artifact", "cache", "hermetic", "action", "worker",
        "farm", "reproducible", "toolchain", "timestamp", "lto", "thin",
        "monorepo", "graph", "lockfile", "transitive", "ccache", "sccache",
        "preprocessed", "layer", "docker", "sysroot", "sandbox", "network",
        "quarantine", "flaky", "distributed", "translation", "confusion",
        "scoped", "registry", "provenance", "attestation", "debounce", "watch",
        "sharding", "runner", "container", "hash", "staleness",
    ],
    "sec": [
        "injection", "parameterized", "timing", "sidechannel", "constanttime",
        "jwt", "hmac", "argon", "bcrypt", "memoryhard", "pinning", "transparency",
        "ssrf", "metadata", "rowlevel", "tenancy", "spectre", "speculative",
        "vault", "lease", "prototype", "pollution", "mtls", "mesh", "padding",
        "oracle", "aead", "credential", "stuffing", "seccomp", "syscall",
        "privilege", "blastradius", "csp", "xss", "supplychain", "signature",
        "oidc", "audience", "escape",
    ],
    "obs": [
        "tracing", "traceid", "span", "causality", "cardinality", "series",
        "sampling", "head", "tail", "histogram", "bucket", "native", "red",
        "use", "utilization", "saturation", "structured", "log", "aggregate",
        "symptom", "slo", "errorbudget", "burnrate", "exemplar", "percentile",
        "quantile", "ebpf", "probe", "scrape", "pull", "push", "skew", "ntp",
        "attribute", "resource", "synthetic", "journey", "deploy", "marker",
        "regression",
    ],
    "os": [
        "scheduler", "preempt", "context", "switch", "pagefault", "tlb",
        "virtual", "physical", "frame", "copyonwrite", "fork", "syscall",
        "interrupt", "softirq", "cgroup", "namespace", "futex", "spinlock",
        "rcu", "epoll", "iouring", "mmap", "slab", "allocator", "numa",
        "affinity", "priority", "inversion", "deadline", "cfs", "nice",
        "swap", "reclaim", "oomkiller", "dirty", "writeback", "journaling",
        "inode", "dentry", "vfs",
    ],
    "gfx": [
        "rasterize", "shader", "vertex", "fragment", "pipeline", "framebuffer",
        "depth", "stencil", "blend", "texture", "mipmap", "sampler", "vulkan",
        "descriptor", "swapchain", "barrier", "occlusion", "culling", "frustum",
        "raytracing", "bvh", "acceleration", "denoise", "gbuffer", "deferred",
        "tonemap", "hdr", "bloom", "antialias", "msaa", "tessellation", "compute",
        "workgroup", "atomic", "warp", "occupancy", "bandwidth", "coalesce",
        "instancing", "batch",
    ],
    "comp": [
        "lexer", "parser", "grammar", "ast", "typecheck", "inference", "hindley",
        "monomorphize", "lowering", "ir", "ssa", "dominator", "phi", "register",
        "spill", "allocation", "coloring", "inlining", "vectorize", "unroll",
        "peephole", "constant", "folding", "deadcode", "elimination", "escape",
        "analysis", "borrow", "codegen", "backend", "linker", "relocation",
        "symbol", "mangling", "abi", "calling", "convention", "epilogue",
        "prologue", "tailcall",
    ],
    "crypto": [
        "aes", "gcm", "nonce", "cipher", "block", "stream", "keyschedule",
        "sbox", "diffie", "hellman", "elliptic", "curve", "scalar", "point",
        "signature", "ecdsa", "ed25519", "hash", "sha", "merkle", "root",
        "hmac", "kdf", "hkdf", "salt", "pbkdf", "commitment", "zeroknowledge",
        "snark", "polynomial", "commit", "lattice", "kyber", "dilithium",
        "postquantum", "entropy", "csprng", "forward", "secrecy", "ratchet",
    ],
}

# Cosmetic connective words woven between content tokens so the text reads like
# prose. They appear across many docs, so BM25 gives them ~zero IDF and the dense
# leg treats them as symmetric noise. Queries never use them, so they never
# dilute query-doc overlap. Deliberately disjoint from every topic pool.
CONNECT = [
    "the", "a", "and", "of", "with", "for", "under", "across", "when",
    "that", "to", "in", "while", "so", "then", "each", "per", "via",
]


def _weave(rng, tokens):
    """Interleave connective words among content tokens to form a sentence."""
    out = []
    for i, tok in enumerate(tokens):
        out.append(tok)
        # Sprinkle a connector after roughly every third token.
        if i != len(tokens) - 1 and rng.random() < 0.34:
            out.append(rng.choice(CONNECT))
    sentence = " ".join(out)
    return sentence[:1].upper() + sentence[1:] + "."


def _plan_clusters(rng, n, topic_keys):
    """Assign a contiguous ~15% of docs into near-duplicate clusters of 2-4.

    Returns (cluster_of, cluster_meta) where cluster_of[d] is a cluster id or
    None, and cluster_meta[cid] = (topic, anchor, shared_filler).
    """
    cluster_of = [None] * n
    cluster_meta = {}
    target = int(n * 0.15)
    clustered = 0
    cid = 0
    d = 0
    while d < n:
        if clustered < target and d + 1 < n and rng.random() < 0.13:
            k = min(rng.randint(2, 4), n - d)
            topic = rng.choice(topic_keys)
            pool = TOPICS[topic]
            anchor = f"{topic}cluster{cid}"
            filler = rng.sample(pool, min(len(pool), rng.randint(9, 14)))
            cluster_meta[cid] = (topic, anchor, filler)
            for j in range(d, d + k):
                cluster_of[j] = cid
            cid += 1
            clustered += k
            d += k
        else:
            d += 1
    return cluster_of, cluster_meta


def generate(docs_n, queries_n, seed, name):
    rng = random.Random(seed)
    topic_keys = sorted(TOPICS.keys())

    cluster_of, cluster_meta = _plan_clusters(rng, docs_n, topic_keys)

    docs = []
    # Per-doc metadata retained for query construction.
    meta = []  # list of dict: id, topic, anchors[list], words[list], cluster
    for d in range(docs_n):
        did = f"d{d:05d}"
        cid = cluster_of[d]
        if cid is not None:
            topic, canchor, filler = cluster_meta[cid]
            # Shared cluster anchor + a per-doc unique variant token; filler is the
            # cluster's shared pool in a per-doc shuffled order (paraphrase).
            a_shared = canchor
            a_var = f"{topic}var{d}"
            words = filler[:]
            rng.shuffle(words)
            anchors = [a_shared, a_var]
            tokens = [a_shared, a_var] + words
        else:
            topic = rng.choice(topic_keys)
            pool = TOPICS[topic]
            a1 = f"{topic}anchor{d}"
            a2 = f"{topic}key{d}"
            words = rng.sample(pool, min(len(pool), rng.randint(9, 16)))
            anchors = [a1, a2]
            tokens = [a1, a2] + words
        text = _weave(rng, tokens)
        docs.append({"id": did, "text": text})
        meta.append(
            {
                "id": did,
                "topic": topic,
                "anchors": anchors,
                "words": words,
                "cluster": cid,
            }
        )

    # ---- queries -----------------------------------------------------------
    # ~1/4 multi-relevant (one per cluster), rest single-doc. Interleaved in a
    # fixed order; every 6th query is "content" mode (~1/6), rest "hybrid".
    cluster_ids = sorted(cluster_meta.keys())
    rng.shuffle(cluster_ids)
    n_multi = min(len(cluster_ids), max(1, queries_n // 4))

    # Members per cluster, in doc order.
    members = {cid: [] for cid in cluster_meta}
    for d, cid in enumerate(cluster_of):
        if cid is not None:
            members[cid].append(f"d{d:05d}")

    singles_pool = [m for m in meta if m["cluster"] is None]
    rng.shuffle(singles_pool)

    queries = []
    multi_used = 0
    single_idx = 0
    for qi in range(queries_n):
        # Decide multi vs single: front-load multi queries deterministically by
        # spacing them roughly every 4th slot until exhausted.
        want_multi = (multi_used < n_multi) and (qi % 4 == 1)
        if want_multi:
            cid = cluster_ids[multi_used]
            multi_used += 1
            topic, canchor, filler = cluster_meta[cid]
            picks = rng.sample(filler, min(len(filler), rng.randint(3, 5)))
            qtokens = [canchor] + picks
            expected = list(members[cid])
        else:
            if single_idx >= len(singles_pool):
                # Wrap deterministically if the trace asks for more singles than
                # non-clustered docs (won't happen at the requested sizes).
                single_idx = 0
                rng.shuffle(singles_pool)
            m = singles_pool[single_idx]
            single_idx += 1
            picks = rng.sample(m["words"], min(len(m["words"]), rng.randint(2, 3)))
            qtokens = list(m["anchors"]) + picks
            expected = [m["id"]]
        mode = "content" if qi % 6 == 5 else "hybrid"
        queries.append(
            {
                "id": f"q{qi:04d}",
                "query": " ".join(qtokens),
                "expected_ids": expected,
                "mode": mode,
            }
        )

    return {"name": name, "docs": docs, "queries": queries}


def main():
    ap = argparse.ArgumentParser(description="Generate a scaled retrieval-bench trace.")
    ap.add_argument("--docs", type=int, required=True)
    ap.add_argument("--queries", type=int, required=True)
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--name", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()

    trace = generate(args.docs, args.queries, args.seed, args.name)
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(trace, f, ensure_ascii=False, indent=2, separators=(",", ": "))
        f.write("\n")
    print(
        f"wrote {args.out}: {len(trace['docs'])} docs, {len(trace['queries'])} queries "
        f"(seed {args.seed})"
    )


if __name__ == "__main__":
    main()
