#!/usr/bin/env python3
"""Distill-quality bench for kern's reason leg — the evidence behind
`DEFAULT_REASON_MODEL`.

Scores a candidate model on kern's REAL distill prompt (kept in sync with
`src/ingest/distill.rs` by hand) over a small gold set:
  - format:   conversations yielding a parseable JSON array of {text,kind}
  - recall:   gold facts semantically matched by some emitted claim
  - noise:    emitted claims matching no gold fact (over-extraction)
  - kind_acc: kind label correct among matched claims

Recall is scored by embedding cosine, not string match, so a correct claim
phrased differently still counts.

Reading the output, honestly:
  - 13 gold facts is a SMALL set. A one-fact difference is ~8pp and is noise.
    Treat `format` and `noise` as the robust signals and re-run at both
    thresholds (0.62 / 0.72) — a ranking that flips between them is not real.
  - Cosine matching has a false-positive floor: at 0.62, two topically related
    but distinct facts can match (measured: an unrelated postgres-overhead
    claim matched a "revisit if sharding" gold fact at 0.655). Recall is
    therefore an upper bound, not an exact score.
  - `kind_acc` is ~33% even for a 7B model. kern's taxonomy has overlapping
    categories (decision/project, fact/code-fact); low kind accuracy is a
    prompt/taxonomy signal, NOT a reason to pick a bigger model.

Usage: python scripts/distill_bench.py <ollama|vllm> <model-tag> [more tags...]
Env:   KERN_OLLAMA_URL / KERN_VLLM_URL override the endpoints (a WSL host must
       point at the Windows gateway IP, not localhost).
"""
import json, os, sys, urllib.request

OLLAMA = os.environ.get("KERN_OLLAMA_URL", "http://localhost:11434")
VLLM = os.environ.get("KERN_VLLM_URL", "http://localhost:8100")
EMBED_MODEL = "qwen3-embedding:0.6b"
# Cosine above which an emitted claim counts as covering a gold fact. Re-run at
# 0.62 and 0.72 and trust only rankings that survive both — see module docstring.
MATCH_THRESHOLD = 0.72

PROMPT = """Extract durable, reusable knowledge from this conversation between a user and an AI coding assistant. Output ONLY a JSON array. Each element must be {"text": "<one self-contained statement>", "kind": "<one of: preference, decision, project, fact, code-fact, reference, procedural>"}. Optionally add "valid_from": "<ISO8601 date>" ONLY when the statement itself says when it became true (e.g. "since March 2026", "as of v2"); omit it otherwise. Include only knowledge worth remembering across future sessions: user preferences, decisions and their rationale, ongoing project state, durable facts, structural code facts, external references, and procedural knowledge (learned workflows, rules, and conventions — how we do X, not just what is true). Consolidate aggressively: emit ONE claim per distinct fact. Do NOT output multiple claims that restate the same idea, and do NOT output sentence fragments — each claim must be a complete, standalone statement that captures the fact in full. Prefer the single most complete phrasing over several partial ones. Skip greetings, acknowledgements, one-off task mechanics, and anything ephemeral. If nothing is worth keeping, output []. Do not wrap the array in markdown.

CONVERSATION:
%s
"""

# Each case: conversation + gold facts (text, expected kind).
CASES = [
    {
        "convo": """user: hey, can you fix the login timeout bug?
assistant: Found it - the session TTL in auth/session.rs was 30s instead of 30min. Fixed by changing SESSION_TTL to Duration::from_secs(1800).
user: great. btw we decided to move all config to TOML files going forward, no more env vars for app config
assistant: Noted. I'll use .config/app.toml for future config additions.""",
        "gold": [
            ("The session TTL in auth/session.rs is SESSION_TTL set to Duration::from_secs(1800), 30 minutes", "code-fact"),
            ("All application config moves to TOML files instead of environment variables", "decision"),
        ],
    },
    {
        "convo": """user: what's our deploy process again?
assistant: You deploy via 'just deploy prod' which builds the release binary, runs migrations with sqlx, and rsyncs to the prod box at deploy.example.com. Rollback is 'just rollback'.
user: right. also FYI I prefer short commit messages, one line max
assistant: Got it, one-line commit messages from now on.""",
        "gold": [
            ("Deploy to production with 'just deploy prod', which builds the release binary, runs sqlx migrations, and rsyncs to deploy.example.com", "procedural"),
            ("Rollback a deploy with 'just rollback'", "procedural"),
            ("The user prefers short commit messages, one line maximum", "preference"),
        ],
    },
    {
        "convo": """user: thanks!
assistant: You're welcome! Let me know if you need anything else.""",
        "gold": [],
    },
    {
        "convo": """user: why is the ingest worker so slow on big files?
assistant: It embeds every chunk sequentially. The embedder is qwen3-embedding:0.6b over HTTP, roughly 40ms per chunk, so a 500-chunk document takes 20 seconds.
user: ok let's batch them. use batches of 32
assistant: Done - embed_batch now sends 32 chunks per request.
user: also never use unwrap() in this codebase, always propagate errors with ?
assistant: Understood, I'll use ? propagation instead of unwrap().""",
        "gold": [
            ("The ingest worker embeds chunks using qwen3-embedding:0.6b over HTTP at roughly 40ms per chunk", "code-fact"),
            ("Chunk embedding is batched at 32 chunks per request in embed_batch", "decision"),
            ("Never use unwrap() in this codebase; propagate errors with the ? operator", "preference"),
        ],
    },
    {
        "convo": """user: the API docs are at https://internal.acme.dev/api/v3 if you need them
assistant: Noted, I'll reference the v3 API docs there.
user: we're targeting a beta launch in September 2026, still blocked on the auth rewrite
assistant: Got it - beta launch September 2026, auth rewrite is the blocker.""",
        "gold": [
            ("The internal API documentation is at https://internal.acme.dev/api/v3", "reference"),
            ("Beta launch is targeted for September 2026 and is blocked on the auth rewrite", "project"),
        ],
    },
    {
        "convo": """user: can you rename the variable on line 40 to userCount
assistant: Renamed.
user: perfect, thanks
assistant: Anytime!""",
        "gold": [],
    },
    {
        "convo": """user: postgres or sqlite for this?
assistant: Given the single-node deployment and read-heavy workload, sqlite with WAL mode is simpler and fast enough. Postgres would add operational overhead you don't need yet.
user: agreed, sqlite it is. we can migrate later if we shard
assistant: Decision recorded: sqlite with WAL mode, revisit if sharding becomes necessary.""",
        "gold": [
            ("The project uses sqlite with WAL mode rather than postgres, chosen because the deployment is single-node and read-heavy and postgres would add unneeded operational overhead", "decision"),
            ("The database choice should be revisited if sharding becomes necessary", "project"),
        ],
    },
    {
        "convo": """user: run the tests
assistant: All 646 tests pass.
user: our test convention: every bug fix needs a regression test that fails on the old code first
assistant: Understood - regression test that fails pre-fix for every bug fix.""",
        "gold": [
            ("Every bug fix requires a regression test that fails on the old code before the fix", "procedural"),
        ],
    },
]

KINDS = {"preference", "decision", "project", "fact", "code-fact", "reference", "procedural"}


def post(url, body, timeout=600):
    req = urllib.request.Request(url, json.dumps(body).encode(), {"Content-Type": "application/json"})
    return json.loads(urllib.request.urlopen(req, timeout=timeout).read())


def embed(texts):
    if not texts:
        return []
    r = post(f"{OLLAMA}/api/embed", {
        "model": EMBED_MODEL, "input": texts, "truncate": True,
        "keep_alive": "10m", "options": {"num_ctx": 2048},
    })
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
            "temperature": 0.0, "seed": 7,
        })
        return r["choices"][0]["message"]["content"]
    r = post(f"{OLLAMA}/api/chat", {
        "model": model, "messages": [{"role": "user", "content": prompt}],
        "stream": False, "think": False, "keep_alive": "5m",
        "options": {"temperature": 0.0, "seed": 7, "num_ctx": 8192},
    })
    return r["message"]["content"]


def parse_claims(raw):
    """Mirror of kern's parse_claims: first '[' to last ']', tolerant of prose."""
    s, e = raw.find("["), raw.rfind("]")
    if s == -1 or e <= s:
        return None
    try:
        items = json.loads(raw[s:e + 1])
    except Exception:
        return None
    if not isinstance(items, list):
        return None
    if len(items) == 1 and isinstance(items[0], list):
        items = items[0]
    out = []
    for it in items:
        if isinstance(it, dict) and "text" in it:
            out.append({"text": str(it["text"]), "kind": str(it.get("kind", "fact"))})
    return out


def run(backend, model):
    total_gold = matched_gold = noise = kind_ok = kind_total = 0
    format_ok = 0
    for case in CASES:
        gold = case["gold"]
        # Count gold BEFORE any early-out: a conversation the model failed to
        # emit parseable JSON for scores zero recall, it does not vanish from
        # the denominator (that would reward a format failure with a free pass).
        total_gold += len(gold)

        raw = complete(backend, model, PROMPT % case["convo"])
        claims = parse_claims(raw)
        if claims is None:
            continue
        format_ok += 1

        if not gold:
            noise += len(claims)          # noise-only convo: every claim is over-extraction
            continue
        if not claims:
            continue

        gvecs = embed([g[0] for g in gold])
        cvecs = embed([c["text"] for c in claims])

        for gi, (gtext, gkind) in enumerate(gold):
            sims = [cosine(gvecs[gi], cv) for cv in cvecs]
            best = max(sims)
            if best >= MATCH_THRESHOLD:
                matched_gold += 1
                kind_total += 1
                if claims[sims.index(best)]["kind"] == gkind:
                    kind_ok += 1
        for ci, c in enumerate(cvecs):
            if max(cosine(gv, c) for gv in gvecs) < MATCH_THRESHOLD:
                noise += 1

    return {
        "model": model,
        "format": f"{format_ok}/{len(CASES)}",
        "recall": f"{matched_gold}/{total_gold}" + (f" ({matched_gold/total_gold:.0%})" if total_gold else ""),
        "kind_acc": f"{kind_ok}/{kind_total}" + (f" ({kind_ok/kind_total:.0%})" if kind_total else ""),
        "noise": noise,
    }


if __name__ == "__main__":
    backend = sys.argv[1]
    for model in sys.argv[2:]:
        try:
            r = run(backend, model)
            print(f"{r['model']:34s} format={r['format']:5s} recall={r['recall']:12s} "
                  f"kind={r['kind_acc']:11s} noise={r['noise']}")
        except Exception as ex:
            print(f"{model:34s} ERROR: {type(ex).__name__}: {str(ex)[:90]}")
