#!/usr/bin/env python3
"""Embedder bench for kern's embed leg — the evidence behind `DEFAULT_EMBED_MODEL`.

Embedders exist to retrieve, so this scores retrieval, not similarity vibes:
every query is embedded against the whole fact corpus and ranked.

  - recall@1 / recall@3: gold fact ranked first / in the top 3
  - MRR:                 mean reciprocal rank of the gold fact
  - margin:              mean cosine gap between the gold fact and the best
                         WRONG fact. This is the number that matters for kern:
                         `place_document` dedups on an absolute cosine
                         threshold, so a model whose scores all bunch near 1.0
                         retrieves fine but dedups badly. High recall with a
                         thin margin is a trap.
  - dim:                 vector width. CHANGING THIS IS A MIGRATION — the graph
                         locks the dimension on first ingest and a switch needs
                         `kern reembed` over every entity.

Corpus is deliberately adversarial: several facts are near-duplicates of each
other (HNSW/index facts, deploy/rollback), so a weak embedder confuses them.

Usage: python scripts/embed_bench.py <model-tag> [more tags...]
Env:   KERN_OLLAMA_URL overrides the endpoint.
"""
import json, os, sys, urllib.request

OLLAMA = os.environ.get("KERN_OLLAMA_URL", "http://localhost:11434")

FACTS = [
    "The session TTL in auth/session.rs is SESSION_TTL set to Duration::from_secs(1800), which is 30 minutes.",
    "All application config moves to TOML files instead of environment variables.",
    "Deploy to production with 'just deploy prod', which builds the release binary, runs sqlx migrations, and rsyncs to deploy.example.com.",
    "Rollback a deploy with 'just rollback'.",
    "The user prefers short commit messages, one line maximum.",
    "The ingest worker embeds chunks using qwen3-embedding:0.6b over HTTP at roughly 40ms per chunk.",
    "Chunk embedding is batched at 32 chunks per request in embed_batch.",
    "Never use unwrap() in this codebase; propagate errors with the ? operator.",
    "The internal API documentation is at https://internal.acme.dev/api/v3.",
    "Beta launch is targeted for September 2026 and is blocked on the auth rewrite.",
    "The project uses sqlite with WAL mode rather than postgres.",
    "The database choice should be revisited if sharding becomes necessary.",
    "Every bug fix requires a regression test that fails on the old code before the fix.",
    # near-duplicate distractors: separable only by a good embedder
    "The retrieval layer uses an HNSW index over embeddings.",
    "HNSW node levels are derived from the entity id so index builds stay deterministic.",
    "HNSW ties are broken on (distance, id) rather than distance alone.",
    "The lexical index is a separate BM25 store, not part of HNSW.",
]

# (query, index of the ONE correct fact)
QUERIES = [
    ("how long before a session expires", 0),
    ("where does app configuration live now", 1),
    ("command to ship to production", 2),
    ("how do I undo a bad deploy", 3),
    ("what commit message style is preferred", 4),
    ("how fast is a single chunk embedded", 5),
    ("how many chunks go in one embedding request", 6),
    ("what should I use instead of unwrap", 7),
    ("where are the API docs", 8),
    ("when is the beta shipping", 9),
    ("which database are we on", 10),
    ("when should we reconsider the database", 11),
    ("do I need a test for a bug fix", 12),
    ("what index does retrieval use", 13),
    ("how are HNSW levels chosen", 14),
    ("how are equal distances resolved in HNSW", 15),
    ("what does BM25 do here", 16),
]


def post(url, body, timeout=600):
    req = urllib.request.Request(url, json.dumps(body).encode(), {"Content-Type": "application/json"})
    return json.loads(urllib.request.urlopen(req, timeout=timeout).read())


def embed(model, texts):
    r = post(f"{OLLAMA}/api/embed", {"model": model, "input": texts, "truncate": True,
                                     "keep_alive": "5m", "options": {"num_ctx": 2048}})
    return r["embeddings"]


def cosine(a, b):
    dot = sum(x * y for x, y in zip(a, b))
    na = sum(x * x for x in a) ** 0.5
    nb = sum(x * x for x in b) ** 0.5
    return dot / (na * nb) if na and nb else 0.0


def run(model):
    fvecs = embed(model, FACTS)
    qvecs = embed(model, [q for q, _ in QUERIES])
    dim = len(fvecs[0])

    hit1 = hit3 = 0
    rr_sum = margin_sum = 0.0
    for (q, gold), qv in zip(QUERIES, qvecs):
        sims = [cosine(qv, fv) for fv in fvecs]
        order = sorted(range(len(sims)), key=lambda i: -sims[i])
        rank = order.index(gold) + 1
        if rank == 1:
            hit1 += 1
        if rank <= 3:
            hit3 += 1
        rr_sum += 1.0 / rank
        best_wrong = max(s for i, s in enumerate(sims) if i != gold)
        margin_sum += sims[gold] - best_wrong

    n = len(QUERIES)
    return (f"{model:28s} dim={dim:<5d} recall@1={hit1}/{n} ({hit1/n:.0%})  "
            f"recall@3={hit3}/{n} ({hit3/n:.0%})  MRR={rr_sum/n:.3f}  "
            f"margin={margin_sum/n:+.3f}")


if __name__ == "__main__":
    for m in sys.argv[1:]:
        try:
            print(run(m))
        except Exception as ex:
            print(f"{m:28s} ERROR: {type(ex).__name__}: {str(ex)[:70]}")
