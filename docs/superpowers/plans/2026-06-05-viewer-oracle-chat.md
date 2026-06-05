# Viewer RAG Oracle Chat — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the kern viewer's explorer with a streaming RAG oracle — ask a question, get an LLM answer grounded in kern memory (streamed token-by-token) with its source thoughts + provenance chains shown in a single bento box.

**Architecture:** Peers retrieve over their own graph via the existing `retrieval::answer::query` (no generation); the hub embeds the query once, fans out, merges sources by score, builds one prompt, and streams the generation via a new `Client::complete_stream` over Server-Sent Events. The frontend (`App.vue`) is rewritten to chat-left + source-bento-right.

**Tech Stack:** Rust (axum 0.8 SSE, reqwest streaming, async-stream, serde_json, tokio), Vue 3 `<script setup>`, Ollama (qwen2.5 chat + bge-m3 embed).

---

## File Structure

- `kern/Cargo.toml` (workspace deps, ~line 33) — **modify.** Add `"stream"` to reqwest features (needed for `Response::bytes_stream`).
- `kern/src/llm.rs` — **modify.** Add a pure SSE-delta parser + `complete_stream(messages)`.
- `kern/src/viewer.rs` — **modify.** Thread `RetrievalConfig` into `run` + local state; add peer `POST /ask_retrieve`; add hub `POST /ask` (SSE); a pure prompt builder; then remove the now-superseded `/search` endpoints. Reuse `rank_peers`/`merge_search_hits` for source merging.
- `kern/src/commands.rs` (~527) — **modify.** Pass `cfg.retrieval.clone()` into `viewer::run`.
- `kern/viewer/src/App.vue` — **rewrite.** Chat transcript + SSE client + source bento; delete explorer machinery.

**Sequencing rationale:** Tasks 2-3 ADD the oracle path while `/search` still exists, so `rank_peers` always has a caller (the workspace is `warnings = "deny"`; dead code is a hard error). Task 4 removes `/search` only after `/ask` reuses `rank_peers`. Task 5 rewrites the frontend last.

---

## Task 1: `complete_stream` + SSE delta parser (TDD)

**Files:**
- Modify: `kern/Cargo.toml` (reqwest features)
- Modify: `kern/src/llm.rs` (parser + `complete_stream` + test)

- [ ] **Step 1: Add the `stream` feature to reqwest**

In `kern/Cargo.toml`, find the workspace reqwest dependency (around line 33):
```toml
reqwest = { version = "0.13", default-features = false, features = ["json", "rustls"] }
```
Change it to:
```toml
reqwest = { version = "0.13", default-features = false, features = ["json", "rustls", "stream"] }
```

- [ ] **Step 2: Write the failing test for the pure delta parser**

Add to the `#[cfg(test)] mod tests` block in `kern/src/llm.rs`:
```rust
#[test]
fn sse_line_yields_token_then_done() {
    // A content delta line.
    assert_eq!(
        parse_sse_line(r#"data: {"choices":[{"delta":{"content":"He"}}]}"#),
        Some(SseDelta::Token("He".to_string()))
    );
    // The OpenAI/ollama terminator.
    assert_eq!(parse_sse_line("data: [DONE]"), Some(SseDelta::Done));
    // A blank keep-alive / non-data line is ignored.
    assert_eq!(parse_sse_line(""), None);
    assert_eq!(parse_sse_line(": keep-alive"), None);
    // A delta with no content (e.g. role-only opening chunk) yields an empty token,
    // which callers skip.
    assert_eq!(
        parse_sse_line(r#"data: {"choices":[{"delta":{}}]}"#),
        Some(SseDelta::Token(String::new()))
    );
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --lib llm::tests::sse_line_yields_token_then_done`
Expected: FAIL — `cannot find function parse_sse_line` / `cannot find type SseDelta`.

- [ ] **Step 4: Implement the parser**

Add to `kern/src/llm.rs` (non-test code):
```rust
#[derive(Debug, PartialEq)]
enum SseDelta {
    Token(String),
    Done,
}

/// Parse one SSE line from an OpenAI/ollama streaming chat response.
/// `data: [DONE]` → `Done`; `data: {json}` → `Token(delta.content)`; anything
/// else (blank lines, comments, non-`data:` fields) → `None`.
fn parse_sse_line(line: &str) -> Option<SseDelta> {
    let rest = line.strip_prefix("data:")?.trim();
    if rest == "[DONE]" {
        return Some(SseDelta::Done);
    }
    let v: Value = serde_json::from_str(rest).ok()?;
    let content = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("delta"))
        .and_then(|d| d.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("");
    Some(SseDelta::Token(content.to_string()))
}
```
Ensure `use serde_json::Value;` is available in `llm.rs` (add it to the imports if not present).

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test --lib llm::tests::sse_line_yields_token_then_done`
Expected: PASS.

- [ ] **Step 6: Add `complete_stream`**

Add this method inside `impl Client` in `kern/src/llm.rs` (near `complete`):
```rust
/// Stream a chat completion as a sequence of content-delta strings.
/// `messages` is the full multi-turn context (role/content pairs). Yields each
/// non-empty token delta in order; ends when the server sends `[DONE]` or the
/// body closes. Errors (HTTP, network) surface as a single `Err` item.
pub fn complete_stream(
    &self,
    messages: Vec<(String, String)>,
) -> impl futures_core::Stream<Item = Result<String, LlmError>> + Send {
    let client = self.clone();
    async_stream::stream! {
        let url = format!("{}/v1/chat/completions", client.inner.reason_url);
        let msgs: Vec<ChatMessage> = messages
            .iter()
            .map(|(r, c)| ChatMessage { role: r, content: c })
            .collect();
        let body = serde_json::json!({
            "model": client.inner.reason_model,
            "messages": msgs,
            "stream": true,
        });
        let resp = match client.inner.http.post(&url)
            .headers(client.inner.reason_headers.clone())
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => { yield Err(LlmError::from(e)); return; }
        };
        let status = resp.status().as_u16();
        if status >= 400 {
            let body = resp.text().await.unwrap_or_default();
            yield Err(LlmError::Api { status, body });
            return;
        }
        use futures_core::Stream as _;
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        use futures_util::StreamExt as _;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(b) => b,
                Err(e) => { yield Err(LlmError::from(e)); return; }
            };
            buf.push_str(&String::from_utf8_lossy(&chunk));
            // Drain complete lines (SSE frames are newline-delimited).
            while let Some(nl) = buf.find('\n') {
                let line: String = buf.drain(..=nl).collect();
                match parse_sse_line(line.trim_end()) {
                    Some(SseDelta::Done) => return,
                    Some(SseDelta::Token(t)) if !t.is_empty() => yield Ok(t),
                    _ => {}
                }
            }
        }
    }
}
```
Note: this uses `futures_util::StreamExt`. If `futures-util` is not already a dependency, add `futures-util = "0.3"` to `kern/Cargo.toml` `[dependencies]` (the workspace already has `futures-core` and `async-stream`). Verify and add if missing.

- [ ] **Step 7: Verify build + parser test**

Run: `cargo build -p kern && cargo test --lib llm::tests::sse_line_yields_token_then_done`
Expected: clean build, test PASS. (If `LlmError::from(reqwest::Error)` doesn't exist, check the existing `?`-based conversions in `complete` — there is a `From<reqwest::Error>` impl since `complete` uses `?` on `.send()`; reuse it.)

- [ ] **Step 8: Commit**

```bash
git add kern/Cargo.toml kern/src/llm.rs
git commit -m "feat(llm): streaming chat completion + SSE delta parser"
```

---

## Task 2: Thread RetrievalConfig + peer `POST /ask_retrieve`

**Files:**
- Modify: `kern/src/viewer.rs` (run signature, local state, new handler + route)
- Modify: `kern/src/commands.rs` (~527, pass `cfg.retrieval.clone()`)

- [ ] **Step 1: Change local server state to carry the graph + retrieval config**

In `kern/src/viewer.rs`, add near the top (after `type Graph = ...`):
```rust
use crate::config::RetrievalConfig;

#[derive(Clone)]
struct LocalState {
    graph: Graph,
    retrieval: RetrievalConfig,
}
```

- [ ] **Step 2: Change `run` to accept the retrieval config and build the local app with `LocalState`**

Change the signature:
```rust
pub async fn run(graph: Graph, llm: crate::llm::Client, retrieval: RetrievalConfig, agg_addr: &str) -> std::io::Result<()> {
```
Replace the local app construction:
```rust
let local_state = LocalState { graph: graph.clone(), retrieval: retrieval.clone() };
let local_app = Router::new()
    .route("/graph", get(graph_json))
    .route("/search", post(peer_search))
    .route("/ask_retrieve", post(ask_retrieve))
    .with_state(local_state);
```
The existing `graph_json` and `peer_search` handlers take `State<Graph>`. Update them to take `State<LocalState>` and use `state.graph` — OR keep them on `Graph` by extracting it. Cleanest: change both to `State(st): State<LocalState>` and bind `let g = st.graph;` at the top. Do that for `graph_json` and `peer_search` (replace their `State(g): State<Graph>` param with `State(st): State<LocalState>` and add `let g = st.graph;`).

- [ ] **Step 3: Add the peer retrieval handler**

Add to `kern/src/viewer.rs` (non-test code), using HARD TABS:
```rust
#[derive(serde::Deserialize)]
struct AskRetrieveBody {
	vec: Vec<f64>,
	question: String,
	#[serde(default = "default_k")]
	k: usize,
}

/// Peer endpoint for the oracle: retrieve (no generation) over THIS daemon's
/// graph and return scored source thoughts + a pre-formatted provenance string.
/// The hub merges these across daemons and does the single generation.
async fn ask_retrieve(State(st): State<LocalState>, Json(body): Json<AskRetrieveBody>) -> Json<Value> {
	use crate::retrieval::answer;
	use crate::retrieval::seed::Mode;
	let k = body.k.min(MAX_SEARCH_K);
	let g = read_recovered(&st.graph);
	let result = answer::query(&g, &st.retrieval, &body.vec, &body.question, Mode::Hybrid, None, None, None);
	let sources: Vec<Value> = result.entities.iter().take(k).map(|se| {
		json!({
			"id": se.entity.id,
			"label": truncate(&se.entity.text(), 80),
			"text": truncate(&se.entity.text(), 300),
			"kind": format!("{:?}", se.entity.kind),
			"kern": g.kern_of_entity(&se.entity.id).unwrap_or_default(),
			"heat": se.entity.heat,
			"conf": se.entity.conf_mean(),
			"score": se.score,
		})
	}).collect();
	let chain_text = answer::format_chains(&g, &result.path_chains);
	Json(json!({ "sources": sources, "chain_text": chain_text }))
}
```
Verify `g.kern_of_entity(id) -> Option<&String>` (used in `search.rs`); if it returns `Option<&String>`, use `.cloned().unwrap_or_default()`. Verify `Entity` exposes `id`, `text()`, `kind`, `heat`, `conf_mean()` (it does — `graph_json` uses them).

- [ ] **Step 4: Update the call site in `commands.rs`**

In `kern/src/commands.rs` (~527), change:
```rust
let viewer_llm = llm_client.clone();
// ... crate::viewer::run(vg, viewer_llm, &vaddr).await
```
to pass the retrieval config too:
```rust
let viewer_llm = llm_client.clone();
let viewer_retrieval = cfg.retrieval.clone();
```
and update the `run` call inside the spawned task to `crate::viewer::run(vg, viewer_llm, viewer_retrieval, &vaddr).await`. (Inspect the exact spawn block ~520-531 and thread both clones into the `async move`.)

- [ ] **Step 5: Verify build + existing tests**

Run: `cargo build -p kern && cargo test --lib viewer::`
Expected: clean build; existing viewer tests pass. (`peer_search`/`graph_json` now read `st.graph`.)

- [ ] **Step 6: Commit**

```bash
git add kern/src/viewer.rs kern/src/commands.rs
git commit -m "feat(viewer): thread retrieval config + peer /ask_retrieve endpoint"
```

---

## Task 3: Hub `POST /ask` (SSE) + prompt builder (TDD on builder)

**Files:**
- Modify: `kern/src/viewer.rs` (prompt builder + test, hub handler + route)

- [ ] **Step 1: Write the failing test for the prompt builder**

Add to the `#[cfg(test)] mod tests` block in `kern/src/viewer.rs`:
```rust
#[test]
fn ask_prompt_numbers_facts_and_requests_citations() {
    let sources = vec![
        json!({ "text": "confidence join uses max" }),
        json!({ "text": "max is monotone" }),
    ];
    let chains = vec!["Chain 1:\n  [Entity] conf\n".to_string()];
    let p = build_ask_prompt(&sources, &chains, "how sure are we?");
    assert!(p.contains("1. confidence join uses max"));
    assert!(p.contains("2. max is monotone"));
    assert!(p.contains("Chain 1:"));
    assert!(p.contains("how sure are we?"));
    assert!(p.contains("[n]")); // citation instruction present
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --lib viewer::tests::ask_prompt_numbers_facts_and_requests_citations`
Expected: FAIL — `cannot find function build_ask_prompt`.

- [ ] **Step 3: Implement the prompt builder**

Add to `kern/src/viewer.rs` (non-test code), HARD TABS:
```rust
/// Build the generation prompt from merged source texts + per-daemon chain
/// strings. Numbers each fact so the model can cite them as `[n]`, which the
/// browser links back to the source tiles.
fn build_ask_prompt(sources: &[Value], chains: &[String], question: &str) -> String {
	let mut p = String::from("Context from knowledge graph:\n\n");
	for c in chains.iter().filter(|c| !c.trim().is_empty()) {
		p.push_str(c);
		p.push('\n');
	}
	p.push_str("Relevant facts:\n");
	for (i, s) in sources.iter().enumerate() {
		let text = s.get("text").and_then(Value::as_str).unwrap_or("");
		p.push_str(&format!("{}. {}\n", i + 1, text));
	}
	p.push_str(&format!(
		"\nQuestion: {question}\n\
		 Answer concisely using only the context above. Cite the facts you use \
		 inline as [n] where n is the fact number. Do not restate the context. Be direct."
	));
	p
}
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test --lib viewer::tests::ask_prompt_numbers_facts_and_requests_citations`
Expected: PASS.

- [ ] **Step 5: Add the hub SSE handler + route**

Add imports at the top of `kern/src/viewer.rs`:
```rust
use axum::response::sse::{Event, Sse};
use std::convert::Infallible;
use futures_util::StreamExt as _;
```
Add the handler (HARD TABS):
```rust
#[derive(serde::Deserialize)]
struct ChatTurn {
	role: String,
	content: String,
}

#[derive(serde::Deserialize)]
struct AskBody {
	question: String,
	#[serde(default)]
	history: Vec<ChatTurn>,
	#[serde(default = "default_ask_k")]
	k: usize,
}

fn default_ask_k() -> usize { 8 }

/// Hub oracle endpoint: embed the question once, fan retrieval out to peers,
/// merge sources by score, emit a `sources` SSE event, then stream the generated
/// answer as `token` events, ending with `done`. Embed/LLM failure → `error`.
async fn ask(State(st): State<HubState>, Json(body): Json<AskBody>) -> Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>> {
	let stream = async_stream::stream! {
		let q = body.question.trim().to_string();
		if q.is_empty() {
			yield Ok(Event::default().event("done").data("{}"));
			return;
		}
		let k = body.k.min(MAX_SEARCH_K);
		let vec = match st.llm.embed(&q).await {
			Ok(v) => v,
			Err(e) => {
				yield Ok(Event::default().event("error").data(json!({ "message": e.to_string() }).to_string()));
				return;
			}
		};
		// Fan retrieval out to peers (retrieval only — no generation on peers).
		let peers = live_peers();
		let reqbody = json!({ "vec": vec, "question": q, "k": k });
		let mut tagged = Vec::new();
		let mut chains: Vec<String> = Vec::new();
		for addr in &peers {
			let url = format!("http://{addr}/ask_retrieve");
			let resp = match st.client.post(&url).json(&reqbody).send().await {
				Ok(r) => r,
				Err(_) => continue,
			};
			if let Ok(v) = resp.json::<Value>().await {
				if let Some(ct) = v.get("chain_text").and_then(Value::as_str) {
					if !ct.trim().is_empty() { chains.push(ct.to_string()); }
				}
				tagged.push((format!("{addr}|"), json!({ "hits": v.get("sources").cloned().unwrap_or(json!([])) })));
			}
		}
		// Reuse rank_peers: it reads `hits`/`reasons`, namespaces id/kern, sorts by score.
		let mut merged = rank_peers(&tagged, k);
		for (n, s) in merged.iter_mut().enumerate() {
			if let Some(o) = s.as_object_mut() { o.insert("n".into(), json!(n + 1)); }
		}
		yield Ok(Event::default().event("sources").data(json!({ "entities": merged, "chains": chains }).to_string()));
		// Build the prompt and the multi-turn message list.
		let prompt = build_ask_prompt(&merged, &chains, &q);
		let mut messages: Vec<(String, String)> = body.history.iter()
			.rev().take(6).rev()
			.map(|t| (t.role.clone(), t.content.clone()))
			.collect();
		messages.push(("user".to_string(), prompt));
		// Stream the generation.
		let mut gen = Box::pin(st.llm.complete_stream(messages));
		while let Some(item) = gen.next().await {
			match item {
				Ok(tok) => yield Ok(Event::default().event("token").data(json!({ "t": tok }).to_string())),
				Err(e) => { yield Ok(Event::default().event("error").data(json!({ "message": e.to_string() }).to_string())); return; }
			}
		}
		yield Ok(Event::default().event("done").data("{}"));
	};
	Sse::new(stream)
}
```
Register the route on the HUB app (the one with `index`/`aggregate`/`hub_search`):
```rust
.route("/ask", post(ask))
```

- [ ] **Step 6: Verify build + tests**

Run: `cargo build -p kern && cargo test --lib viewer::`
Expected: clean build; `ask_prompt_*` + existing tests pass. (If `Box::pin` + `.next()` needs `futures_util::StreamExt`, it's imported in Step 5.)

- [ ] **Step 7: Commit**

```bash
git add kern/src/viewer.rs
git commit -m "feat(viewer): hub /ask SSE — fan-out retrieval, merge, stream answer"
```

---

## Task 4: Remove the superseded `/search` endpoints

**Files:**
- Modify: `kern/src/viewer.rs` (delete `peer_search`, `hub_search`, `SearchBody`, `SearchQuery`, the two `/search` routes, and any now-unused imports)

- [ ] **Step 1: Delete the search handlers, structs, and routes**

In `kern/src/viewer.rs`, delete:
- `async fn peer_search(...)` and `struct SearchBody`.
- `async fn hub_search(...)` and `struct SearchQuery`.
- `.route("/search", post(peer_search))` from the local app.
- `.route("/search", get(hub_search))` from the hub app.

KEEP `rank_peers`, `merge_search_hits`, `default_k`, `MAX_SEARCH_K` — they are now used by `/ask` and `/ask_retrieve`. KEEP the `rank_peers_*` unit test.

- [ ] **Step 2: Fix imports flagged by the compiler**

Run `cargo build -p kern`. Under `warnings = "deny"`, any import that `peer_search`/`hub_search` used but `/ask` does not will now error as unused. Common ones to check: `Query` (hub_search used it; `/ask` uses `Json`, not `Query` — likely now unused → remove from the `axum::extract::{Query, State}` import, leaving `State`), and `StatusCode` (if no longer used). Remove exactly the imports the compiler reports as unused; do NOT remove anything still used by `ask`/`ask_retrieve`/`graph_json` (`post`, `get`, `Json`, `State`, `IntoResponse`/`Response` if still referenced, `Sse`, `Event`, `Infallible`, `StreamExt`).

- [ ] **Step 3: Verify build + tests**

Run: `cargo build -p kern && cargo test --lib viewer::`
Expected: clean build; `rank_peers_*` and `ask_prompt_*` tests still pass.

- [ ] **Step 4: Commit**

```bash
git add kern/src/viewer.rs
git commit -m "refactor(viewer): drop semantic /search, superseded by oracle /ask"
```

---

## Task 5: Frontend rewrite — chat + source bento + SSE client

**Files:**
- Rewrite: `kern/viewer/src/App.vue`

This task replaces the explorer with the oracle UI. Because `App.vue` is large and tangled with the old explorer, replace the `<script setup>` and `<template>` wholesale; keep the existing CSS custom properties / color ramp and the warm tile styling, adding chat + bento styles.

- [ ] **Step 1: Replace `<script setup>`**

Set the `<script setup>` block of `kern/viewer/src/App.vue` to:
```js
import { ref, onMounted, onBeforeUnmount, nextTick } from 'vue'

const KIND = { Fact: '#e5c07b', Document: '#61afef', Question: '#c678dd', Claim: '#98c379' }
const MARK = { Fact: '◆', Document: '■', Question: '▲', Claim: '●' }

const stats = ref('')
const err = ref('')
const turns = ref([])      // {role:'user'|'oracle', text, sources?, chains?}
const sources = ref([])    // current answer's source tiles [{n,id,label,kind,kern,heat,conf,score}]
const chains = ref([])     // current answer's provenance strings
const input = ref('')
const busy = ref(false)
const hot = ref(null)      // hovered/active citation number
const inputEl = ref(null)
const scrollEl = ref(null)

let history = []           // [{role, content}] sent to the server
let ctrl = null            // AbortController for the in-flight stream
let pulse = null

const heatMax = ref(1)
function ramp(h) {
  const t = Math.min(1, Math.sqrt((h || 0) / (heatMax.value || 1)))
  const lo = [42, 24, 9], hi = [255, 226, 166]
  const c = lo.map((v, i) => Math.round(v + (hi[i] - v) * (0.12 + 0.85 * t)))
  return `rgb(${c[0]},${c[1]},${c[2]})`
}
function textColor(bg) {
  const m = bg.match(/\d+/g) || [0, 0, 0]
  return (0.299 * m[0] + 0.587 * m[1] + 0.114 * m[2]) / 255 > 0.62 ? '#1c1206' : '#fdfaf3'
}

// Split an answer into text + [n] citation chips for rendering.
function segments(text) {
  const out = []
  const re = /\[(\d+)\]/g
  let last = 0, m
  while ((m = re.exec(text))) {
    if (m.index > last) out.push({ t: text.slice(last, m.index) })
    out.push({ cite: +m[1] })
    last = m.index + m[0].length
  }
  if (last < text.length) out.push({ t: text.slice(last) })
  return out
}

async function loadStats() {
  try {
    const g = await (await fetch('/graph')).json()
    const groups = (g.kerns || []).filter(k => k.id !== '__all__').length
    heatMax.value = Math.max(1, ...(g.nodes || []).map(n => +n.heat || 0))
    stats.value = `${(g.nodes || []).length} thoughts · ${groups} groups`
    err.value = ''
  } catch (e) { err.value = String(e) }
}

async function ask() {
  const q = input.value.trim()
  if (!q || busy.value) return
  if (ctrl) ctrl.abort()
  ctrl = new AbortController()
  input.value = ''
  busy.value = true
  sources.value = []; chains.value = []; hot.value = null
  turns.value.push({ role: 'user', text: q })
  const oracle = { role: 'oracle', text: '', sources: [], chains: [] }
  turns.value.push(oracle)
  await scrollDown()
  try {
    const res = await fetch('/ask', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ question: q, history: history.slice(-6) }),
      signal: ctrl.signal,
    })
    if (!res.ok || !res.body) throw new Error('oracle unavailable')
    const reader = res.body.getReader()
    const dec = new TextDecoder()
    let buf = ''
    for (;;) {
      const { value, done } = await reader.read()
      if (done) break
      buf += dec.decode(value, { stream: true })
      let i
      while ((i = buf.indexOf('\n\n')) >= 0) {
        const frame = buf.slice(0, i); buf = buf.slice(i + 2)
        handleFrame(frame, oracle)
      }
    }
    history.push({ role: 'user', content: q })
    history.push({ role: 'assistant', content: oracle.text })
  } catch (e) {
    if (e.name !== 'AbortError') oracle.text = oracle.text || '⚠ oracle unavailable'
  } finally {
    busy.value = false
    await scrollDown()
  }
}

// Parse one SSE frame ("event: x\ndata: {json}") and apply it.
function handleFrame(frame, oracle) {
  let ev = 'message', data = ''
  for (const line of frame.split('\n')) {
    if (line.startsWith('event:')) ev = line.slice(6).trim()
    else if (line.startsWith('data:')) data += line.slice(5).trim()
  }
  let d = {}
  try { d = data ? JSON.parse(data) : {} } catch (_) { return }
  if (ev === 'sources') {
    sources.value = d.entities || []
    chains.value = d.chains || []
    oracle.sources = sources.value; oracle.chains = chains.value
    scrollDown()
  } else if (ev === 'token') {
    oracle.text += d.t || ''
    scrollDown()
  } else if (ev === 'error') {
    oracle.text = (oracle.text || '') + `\n⚠ ${d.message || 'oracle error'}`
  }
}

async function scrollDown() {
  await nextTick()
  const el = scrollEl.value
  if (el) el.scrollTop = el.scrollHeight
}

function onKey(ev) {
  if (ev.key === 'Enter' && !ev.shiftKey) { ev.preventDefault(); ask() }
}

onMounted(() => {
  loadStats(); pulse = setInterval(loadStats, 5000)
  inputEl.value?.focus()
})
onBeforeUnmount(() => { if (pulse) clearInterval(pulse); if (ctrl) ctrl.abort() })
```

- [ ] **Step 2: Replace `<template>`**

Set the `<template>` block to:
```html
<template>
  <div class="app">
    <header class="rail">
      <div class="brand"><b>kern</b><span class="sub">oracle</span></div>
      <div class="rstats"><span class="dot"></span>{{ stats }}<span v-if="err" class="err"> — {{ err }}</span></div>
    </header>

    <div class="stage">
      <!-- left: conversation -->
      <section class="chat">
        <div class="scroll" ref="scrollEl">
          <div v-if="!turns.length" class="hint">Ask the oracle anything about your memory.</div>
          <div v-for="(t, i) in turns" :key="i" class="turn" :class="t.role">
            <template v-if="t.role === 'oracle'">
              <span class="oglyph">◈</span>
              <div class="obody">
                <span v-for="(s, j) in segments(t.text)" :key="j">
                  <span v-if="s.cite" class="cite" :class="{ on: hot === s.cite }"
                    @mouseenter="hot = s.cite" @mouseleave="hot = null">{{ s.cite }}</span>
                  <span v-else>{{ s.t }}</span>
                </span>
                <span v-if="busy && i === turns.length - 1" class="caret">▍</span>
              </div>
            </template>
            <div v-else class="ubody">{{ t.text }}</div>
          </div>
        </div>
        <div class="ask">
          <input ref="inputEl" v-model="input" @keydown="onKey" :disabled="busy"
            placeholder="ask the oracle…" />
          <button @click="ask" :disabled="busy || !input.trim()">↵</button>
        </div>
      </section>

      <!-- right: one bento of sources + provenance -->
      <section class="panel">
        <header class="phead"><span class="ptitle">sources</span>
          <span class="count" v-if="sources.length">{{ sources.length }}</span></header>
        <div class="bwrap">
          <div class="bento">
            <div v-for="s in sources" :key="s.id" class="tile" :class="{ on: hot === s.n }"
              :style="{ background: ramp(s.heat), color: textColor(ramp(s.heat)) }"
              @mouseenter="hot = s.n" @mouseleave="hot = null">
              <span class="tn">{{ s.n }}</span>
              <span class="tmark">{{ MARK[s.kind] || '·' }}</span>
              <div class="tname">{{ s.label }}</div>
              <div class="tmeta">{{ s.kind }} · {{ (+s.score).toFixed(2) }}</div>
            </div>
            <div v-if="!sources.length" class="empty">sources appear here</div>
          </div>
          <pre v-if="chains.length" class="trail">{{ chains.join('\n') }}</pre>
        </div>
      </section>
    </div>
  </div>
</template>
```

- [ ] **Step 3: Replace `<style>` with a focused layout**

Set the `<style>` block to (keeps the warm dark theme; defines chat + bento):
```css
<style>
:root { --ink:#f4f1ea; --muted:#8b8678; --line:rgba(244,241,234,0.10); --panel:rgba(244,241,234,0.018);
  --display:'Bricolage Grotesque',system-ui,sans-serif; --body:'Hanken Grotesk',system-ui,sans-serif; --mono:'IBM Plex Mono',ui-monospace,monospace; }
* { box-sizing:border-box; } html,body,#app { height:100%; margin:0; }
.app { height:100%; display:flex; flex-direction:column;
  background:radial-gradient(120% 90% at 50% -10%, #16130f 0%, #0a0a0c 55%, #08080a 100%); color:var(--ink); font-family:var(--body); }
.rail { display:flex; align-items:baseline; gap:14px; padding:14px 22px; border-bottom:1px solid var(--line); }
.brand b { font-family:var(--display); font-weight:800; font-size:17px; } .brand .sub { color:var(--muted); font-family:var(--mono); font-size:11px; letter-spacing:.16em; text-transform:uppercase; margin-left:6px; }
.rstats { color:var(--muted); font-family:var(--mono); font-size:11px; display:flex; align-items:center; gap:8px; }
.dot { width:7px; height:7px; border-radius:50%; background:#98c379; box-shadow:0 0 8px #98c379; }
.err { color:#e8705e; }
.stage { flex:1; min-height:0; display:flex; gap:18px; padding:18px 22px; }
.chat { flex:1; min-width:0; display:flex; flex-direction:column; border-radius:18px; background:var(--panel); box-shadow:inset 0 0 0 1px rgba(244,241,234,0.06); overflow:hidden; }
.scroll { flex:1; overflow-y:auto; padding:20px; display:flex; flex-direction:column; gap:16px; }
.hint { color:var(--muted); margin:auto; font-size:14px; }
.turn.user { align-self:flex-end; max-width:80%; } .ubody { background:rgba(242,169,62,0.14); color:var(--ink); padding:9px 13px; border-radius:13px 13px 3px 13px; font-size:14px; }
.turn.oracle { display:flex; gap:10px; max-width:92%; } .oglyph { color:#98c379; } .obody { font-size:15px; line-height:1.5; }
.cite { display:inline-flex; align-items:center; justify-content:center; min-width:17px; height:17px; padding:0 4px; margin:0 1px; border-radius:5px;
  background:rgba(97,175,239,0.18); color:#9cd0ff; font-family:var(--mono); font-size:11px; cursor:pointer; vertical-align:1px; }
.cite.on { background:#61afef; color:#08080a; }
.caret { color:#f2a93e; animation:blink 1s steps(2) infinite; } @keyframes blink { 50% { opacity:0; } }
.ask { display:flex; gap:10px; padding:14px; border-top:1px solid var(--line); }
.ask input { flex:1; background:#131210; border:0; outline:none; color:var(--ink); font-family:var(--body); font-size:14px; padding:13px 15px; border-radius:11px; box-shadow:inset 0 0 0 1px var(--line); }
.ask input::placeholder { color:#4f4b43; } .ask button { background:#f2a93e; color:#1c1206; border:0; border-radius:11px; width:46px; font-size:16px; cursor:pointer; } .ask button:disabled { opacity:.4; cursor:default; }
.panel { width:40%; min-width:300px; display:flex; flex-direction:column; border-radius:18px; background:var(--panel); box-shadow:inset 0 0 0 1px rgba(244,241,234,0.06); overflow:hidden; }
.phead { font-family:var(--mono); font-size:11px; letter-spacing:.16em; text-transform:uppercase; padding:13px 16px; border-bottom:1px solid var(--line); display:flex; gap:10px; }
.phead .count { margin-left:auto; color:var(--muted); }
.bwrap { flex:1; overflow-y:auto; padding:14px; }
.bento { display:grid; grid-template-columns:1fr 1fr; gap:10px; }
.tile { position:relative; border-radius:13px; padding:12px; min-height:96px; display:flex; flex-direction:column; justify-content:flex-end; gap:5px; box-shadow:inset 0 0 0 1px rgba(255,255,255,0.08); transition:box-shadow .12s; }
.tile.on { box-shadow:inset 0 0 0 2px #61afef, 0 0 0 2px rgba(97,175,239,0.4); }
.tn { position:absolute; top:9px; left:11px; font-family:var(--mono); font-size:11px; opacity:.8; } .tmark { position:absolute; top:8px; right:11px; opacity:.8; }
.tname { font-family:var(--display); font-weight:800; font-size:14px; line-height:1.12; display:-webkit-box; -webkit-line-clamp:3; -webkit-box-orient:vertical; overflow:hidden; }
.tmeta { font-family:var(--mono); font-size:10px; opacity:.75; }
.empty { grid-column:1/-1; color:var(--muted); text-align:center; padding:30px 0; font-size:13px; }
.trail { margin-top:14px; padding:12px; border-radius:11px; background:#0d0c0b; box-shadow:inset 0 0 0 1px var(--line);
  font-family:var(--mono); font-size:11px; line-height:1.5; color:var(--muted); white-space:pre-wrap; }
</style>
```

- [ ] **Step 4: Build**

Run from `kern/viewer`: `npm run build`
Expected: successful Vite build, no errors. (Run `npm install` first if needed.)

- [ ] **Step 5: Commit**

```bash
git add kern/viewer/src/App.vue
git commit -m "feat(viewer): oracle chat UI — streaming answer + source bento"
```

---

## Task 6: Manual end-to-end verification

**Files:** none (verification only)

- [ ] **Step 1: Prerequisites**

Ensure Ollama is running with both models: `ollama pull bge-m3` and `ollama pull qwen2.5`. Start a kern daemon in a directory with memory (so the graph is non-empty), which serves the aggregator on `127.0.0.1:7700`.

- [ ] **Step 2: Run the viewer and verify the oracle**

Serve the viewer (`cd kern/viewer && npm run dev`) and open it. Verify:
- Ask a question → the **sources bento fills first**, then the answer **streams in** token-by-token with a blinking caret.
- The answer contains `[n]` chips; hovering a chip highlights source tile `n` and vice-versa.
- A provenance trail renders under the tiles when chains exist.
- Ask a follow-up ("why?") → the answer reflects the prior turn (multi-turn history).
- Stop Ollama, ask again → the oracle bubble shows "⚠ oracle unavailable"; the transcript is preserved; no uncaught console error.

- [ ] **Step 3: Record the result**

No commit. Report which checks passed and any that failed.

---

## Self-Review

**Spec coverage:**
- Streaming chat + multi-turn (`complete_stream`, messages with history) → Task 1, Task 3. ✓
- Peer retrieve-only over own graph → Task 2 (`ask_retrieve`, `query(... None ...)`). ✓
- Hub embed-once + fan-out + merge + single generation → Task 3 (`ask`). ✓
- Sources + provenance chains in one bento, `[n]` citations linking answer↔tiles → Task 3 (`sources` event, `build_ask_prompt` citation instruction), Task 5 (`segments`, `hot`, tiles). ✓
- SSE (sources → token → done; error path) → Task 3 emit, Task 5 `handleFrame`. ✓
- Replace explorer; remove `/search` → Task 4 (backend), Task 5 (frontend rewrite). ✓
- Abort on new question; history cap (~6) → Task 5 (`ctrl.abort`, `history.slice(-6)`), Task 3 (`.rev().take(6).rev()`). ✓
- Errors: embed/LLM down, empty retrieval, empty question → Task 3 (`error` events, empty-q `done`), Task 5 (catch). ✓
- Tests: SSE parser, prompt builder, source merge (rank_peers reused/tested) → Tasks 1, 3; rank_peers test retained Task 4. ✓

**Placeholder scan:** No TBD/TODO; every code step has full code. Verification points (e.g. `kern_of_entity` return type, `LlmError::From<reqwest::Error>`, `futures-util` presence) are explicit "verify and adjust" instructions with the expected resolution, not vague hand-waves. ✓

**Type consistency:** `complete_stream(Vec<(String,String)>)` (Task 1) is called with `messages: Vec<(String,String)>` (Task 3). `ask_retrieve` returns `{sources, chain_text}` (Task 2); `ask` reads `v.get("sources")` and `v.get("chain_text")` (Task 3). `rank_peers(&[(String, Value)], usize)` is fed `{hits: sources}` tagged tuples (Task 3) — matches its existing `"hits"` reader. Source objects carry `n/id/label/kind/kern/heat/conf/score`; frontend reads `s.n/s.label/s.kind/s.heat/s.score` (Task 5) and citations match `s.n`. SSE events `sources/token/done/error` emitted (Task 3) all handled (Task 5). ✓
