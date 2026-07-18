#!/usr/bin/env python3
"""Answer-quality bench for kern's answer leg — the evidence behind
`DEFAULT_ANSWER_MODEL`.

The answer leg is NOT the reason leg: it glues already-retrieved graph facts
into short grounded prose for `/ask`. Capability barely matters; grounding and
brevity do. So this bench scores what that job needs, using kern's REAL answer
prompt (kept in sync with `retrieval::answer::answer_prompt_from` by hand):

  - correct:   answer semantically contains the expected fact (embedding cosine)
  - grounded:  when the context does NOT contain the answer, the model declines
               instead of inventing one. This is the leg's real failure mode —
               a fluent wrong answer over a good graph is worse than "unknown".
  - verbose:   answers longer than VERBOSE_CHARS ("Do not restate the context.
               Be direct." is in the prompt, so length is a prompt-compliance
               signal, not a style nit)

Reading the output, honestly: 10 cases is a SMALL set. A one-case difference is
10pp and is noise. `grounded` is the signal that should decide a model — it is
the failure that corrupts answers rather than merely lengthening them.

Usage: python scripts/answer_bench.py <ollama|vllm> <model-tag> [more tags...]
Env:   KERN_OLLAMA_URL / KERN_VLLM_URL override endpoints.
"""
import json, os, re, sys, urllib.request

OLLAMA = os.environ.get("KERN_OLLAMA_URL", "http://localhost:11434")
VLLM = os.environ.get("KERN_VLLM_URL", "http://localhost:8100")
EMBED_MODEL = "qwen3-embedding:0.6b"
MATCH_THRESHOLD = 0.62
VERBOSE_CHARS = 400

# Mirrors retrieval::answer::answer_prompt_from.
def build_prompt(facts, question):
    p = "Context from knowledge graph:\n\nRelevant facts:\n"
    for i, f in enumerate(facts):
        p += f"{i + 1}. {f}\n"
    p += (f"\nQuestion: {question}\n"
          "Answer the question concisely using only the context above. "
          "Do not restate the context. Be direct.")
    return p


# expect=None marks an UNANSWERABLE case: the fact is absent from context and
# the correct behavior is to decline.
CASES = [
    {"facts": ["The session TTL in auth/session.rs is SESSION_TTL set to Duration::from_secs(1800), which is 30 minutes.",
               "All application config moves to TOML files instead of environment variables."],
     "q": "How long is the session TTL?", "expect": "30 minutes, or 1800 seconds"},
    {"facts": ["Deploy to production with 'just deploy prod', which builds the release binary, runs sqlx migrations, and rsyncs to deploy.example.com.",
               "Rollback a deploy with 'just rollback'."],
     "q": "How do I roll back a deploy?", "expect": "Run 'just rollback'"},
    {"facts": ["The project uses sqlite with WAL mode rather than postgres.",
               "The database choice should be revisited if sharding becomes necessary."],
     "q": "Which database does the project use?", "expect": "sqlite with WAL mode"},
    {"facts": ["The ingest worker embeds chunks using qwen3-embedding:0.6b over HTTP at roughly 40ms per chunk.",
               "Chunk embedding is batched at 32 chunks per request in embed_batch."],
     "q": "What batch size is used for embedding?", "expect": "32 chunks per request"},
    {"facts": ["Beta launch is targeted for September 2026 and is blocked on the auth rewrite.",
               "The internal API documentation is at https://internal.acme.dev/api/v3."],
     "q": "What is blocking the beta launch?", "expect": "The auth rewrite"},
    {"facts": ["Never use unwrap() in this codebase; propagate errors with the ? operator.",
               "Every bug fix requires a regression test that fails on the old code before the fix."],
     "q": "What is the error handling convention?", "expect": "Propagate errors with ?, never unwrap()"},
    # --- unanswerable: context is topically close but lacks the answer ---
    {"facts": ["The project uses sqlite with WAL mode rather than postgres.",
               "The database choice should be revisited if sharding becomes necessary."],
     "q": "What port does the database listen on?", "expect": None},
    {"facts": ["Deploy to production with 'just deploy prod'.",
               "Rollback a deploy with 'just rollback'."],
     "q": "Who is the on-call engineer this week?", "expect": None},
    {"facts": ["The session TTL in auth/session.rs is 30 minutes.",
               "All application config moves to TOML files."],
     "q": "What hashing algorithm is used for passwords?", "expect": None},
    {"facts": ["Beta launch is targeted for September 2026, blocked on the auth rewrite."],
     "q": "How many people are on the engineering team?", "expect": None},
    # --- harder: the easy cases above saturate, these discriminate ---
    # multi-hop: answer requires chaining two facts, neither sufficient alone
    {"facts": ["The auth rewrite is owned by the platform team.",
               "Beta launch is blocked on the auth rewrite.",
               "The platform team is currently frozen until Q4 2026."],
     "q": "Can the beta launch happen before Q4 2026?",
     "expect": "No — the launch depends on the auth rewrite, owned by the platform team, which is frozen until Q4 2026"},
    # distractors: six near-miss facts, one relevant
    {"facts": ["The retrieval layer uses an HNSW index.",
               "The HNSW index rebuilds on every restart.",
               "HNSW node levels are derived from the entity id.",
               "The lexical index is a separate BM25 store.",
               "The embedder is qwen3-embedding:0.6b.",
               "HNSW ties are broken on (distance, id) to keep rebuilds deterministic."],
     "q": "Why are HNSW ties broken on id as well as distance?",
     "expect": "To keep index rebuilds deterministic"},
    # superseded: newer fact must win over the stale one
    {"facts": ["The default reason model was qwen2.5:7b.",
               "The default reason model is now granite4:3b, superseding qwen2.5:7b."],
     "q": "What is the current default reason model?",
     "expect": "granite4:3b"},
    # negation: context states an explicit NON-fact; must not flip it
    {"facts": ["The eval judge does NOT use the default reason model; it stays pinned to qwen2.5:7b.",
               "The default reason model is granite4:3b."],
     "q": "Does the eval judge use granite4:3b?",
     "expect": "No, the judge stays pinned to qwen2.5:7b"},
]

DECLINE = re.compile(
    r"\b(not|no|does\s?n[o']t|cannot|can't|unable|unknown|unclear|unspecified|"
    r"insufficient|lack|absent|isn't|is not|doesn't (?:say|mention|specify|contain|provide)|"
    r"not (?:in|provided|mentioned|specified|available|stated|given|included|present|found))\b",
    re.I)


def post(url, body, timeout=600):
    req = urllib.request.Request(url, json.dumps(body).encode(), {"Content-Type": "application/json"})
    return json.loads(urllib.request.urlopen(req, timeout=timeout).read())


def embed(texts):
    r = post(f"{OLLAMA}/api/embed", {"model": EMBED_MODEL, "input": texts, "truncate": True,
                                     "keep_alive": "10m", "options": {"num_ctx": 2048}})
    return r["embeddings"]


def cosine(a, b):
    dot = sum(x * y for x, y in zip(a, b))
    na = sum(x * x for x in a) ** 0.5
    nb = sum(x * x for x in b) ** 0.5
    return dot / (na * nb) if na and nb else 0.0


def complete(backend, model, prompt):
    if backend == "vllm":
        r = post(f"{VLLM}/v1/chat/completions", {
            "model": model, "messages": [{"role": "user", "content": prompt}],
            "temperature": 0.0, "seed": 7})
        return r["choices"][0]["message"]["content"]
    r = post(f"{OLLAMA}/api/chat", {
        "model": model, "messages": [{"role": "user", "content": prompt}],
        "stream": False, "think": False, "keep_alive": "5m",
        "options": {"temperature": 0.0, "seed": 7, "num_ctx": 8192}})
    return r["message"]["content"]


def run(backend, model):
    answerable = [c for c in CASES if c["expect"]]
    unanswerable = [c for c in CASES if not c["expect"]]
    correct = grounded = verbose = 0

    for case in answerable:
        ans = complete(backend, model, build_prompt(case["facts"], case["q"])).strip()
        if len(ans) > VERBOSE_CHARS:
            verbose += 1
        v = embed([case["expect"], ans])
        if cosine(v[0], v[1]) >= MATCH_THRESHOLD:
            correct += 1

    for case in unanswerable:
        ans = complete(backend, model, build_prompt(case["facts"], case["q"])).strip()
        if len(ans) > VERBOSE_CHARS:
            verbose += 1
        # Grounded = the model signals the context does not answer the question.
        if DECLINE.search(ans):
            grounded += 1

    return (f"{model:24s} correct={correct}/{len(answerable)}  "
            f"grounded={grounded}/{len(unanswerable)}  verbose={verbose}/{len(CASES)}")


if __name__ == "__main__":
    backend = sys.argv[1]
    for model in sys.argv[2:]:
        try:
            print(run(backend, model))
        except Exception as ex:
            print(f"{model:24s} ERROR: {type(ex).__name__}: {str(ex)[:80]}")
