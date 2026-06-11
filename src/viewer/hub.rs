//! The aggregator hub: fans `/graph` and the oracle out to every live peer,
//! namespaces their ids to avoid cross-project collisions, merges the results,
//! and runs the single agentic generation pass over the merged context.

use std::convert::Infallible;

use axum::extract::State;
use axum::response::sse::{Event, Sse};
use axum::Json;
use futures_util::StreamExt as _;
use serde_json::{json, Value};

use super::registry::live_peers;
use super::{FANOUT_TIMEOUT, MAX_SEARCH_K};

#[derive(Clone)]
pub(super) struct HubState {
	pub(super) client: reqwest::Client,
	pub(super) llm: crate::llm::Client,
}

pub(super) async fn index() -> &'static str {
	"kern viewer aggregator. GET /graph for the merged graph across all running daemons."
}

/// Hub endpoint: fan out to every live peer, namespace ids per peer to avoid
/// cross-project collisions, and merge into one `{nodes,links,kerns}`.
pub(super) async fn aggregate(State(st): State<HubState>) -> Json<Value> {
	let client = &st.client;
	let peers = live_peers();
	let mut nodes = Vec::new();
	let mut links = Vec::new();
	let mut kerns = Vec::new();

	for addr in &peers {
		let url = format!("http://{addr}/graph");
		let resp = match client.get(&url).send().await {
			Ok(r) => r,
			Err(_) => continue, // unreachable peer (race with shutdown) — skip
		};
		let Ok(v) = resp.json::<Value>().await else { continue };
		// Namespace by peer address so identical ids in different daemons (e.g.
		// the same Fact text hashing alike across projects) never merge or
		// shadow. Links stay valid because endpoints share the peer's tag.
		let tag = format!("{addr}|");
		merge_peer(&tag, &v, &mut nodes, &mut links, &mut kerns);
	}

	Json(json!({
		"nodes": nodes,
		"links": links,
		"kerns": kerns,
		"kern_count": kerns.len(),
		"daemons": peers.len(),
	}))
}

/// Re-key one peer's payload under `tag` and append to the merged arrays.
fn merge_peer(tag: &str, v: &Value, nodes: &mut Vec<Value>, links: &mut Vec<Value>, kerns: &mut Vec<Value>) {
	let pre = |id: &Value| -> Value {
		id.as_str().map(|s| Value::String(format!("{tag}{s}"))).unwrap_or(Value::Null)
	};
	for n in v.get("nodes").and_then(Value::as_array).into_iter().flatten() {
		let mut n = n.clone();
		if let Some(o) = n.as_object_mut() {
			if let Some(id) = o.get("id") { let p = pre(id); o.insert("id".into(), p); }
			if let Some(k) = o.get("kern") { let p = pre(k); o.insert("kern".into(), p); }
		}
		nodes.push(n);
	}
	for l in v.get("links").and_then(Value::as_array).into_iter().flatten() {
		let mut l = l.clone();
		if let Some(o) = l.as_object_mut() {
			if let Some(id) = o.get("id") { let p = pre(id); o.insert("id".into(), p); }
			if let Some(s) = o.get("source") { let p = pre(s); o.insert("source".into(), p); }
			if let Some(t) = o.get("target") { let p = pre(t); o.insert("target".into(), p); }
		}
		links.push(l);
	}
	for k in v.get("kerns").and_then(Value::as_array).into_iter().flatten() {
		let mut k = k.clone();
		if let Some(o) = k.as_object_mut() {
			if let Some(id) = o.get("id") { let p = pre(id); o.insert("id".into(), p); }
			match o.get("parent") {
				Some(p) if p.is_string() => { let np = pre(p); o.insert("parent".into(), np); }
				_ => {}
			}
			if let Some(ch) = o.get("children").and_then(Value::as_array) {
				let mapped: Vec<Value> = ch.iter().map(&pre).collect();
				o.insert("children".into(), Value::Array(mapped));
			}
		}
		kerns.push(k);
	}
}

/// Tag one peer's search payload (`{hits, reasons}`) and append every hit to
/// `out`, prefixing `id`/`kern` so they match the namespaced ids `/graph`
/// already shipped to the browser. Both arrays are pooled into one list.
fn merge_search_hits(tag: &str, v: &Value, out: &mut Vec<Value>) {
	let pre = |id: &Value| -> Value {
		id.as_str().map(|s| Value::String(format!("{tag}{s}"))).unwrap_or(Value::Null)
	};
	for arr in ["hits", "reasons"] {
		for h in v.get(arr).and_then(Value::as_array).into_iter().flatten() {
			let mut h = h.clone();
			if let Some(o) = h.as_object_mut() {
				if let Some(id) = o.get("id") { let p = pre(id); o.insert("id".into(), p); }
				if let Some(k) = o.get("kern") { let p = pre(k); o.insert("kern".into(), p); }
			}
			out.push(h);
		}
	}
}

/// Merge every peer's tagged payload, sort by `score` descending, truncate to k.
fn rank_peers(peers: &[(String, Value)], k: usize) -> Vec<Value> {
	let mut out = Vec::new();
	for (tag, v) in peers {
		merge_search_hits(tag, v, &mut out);
	}
	out.sort_by(|a, b| {
		let sa = a.get("score").and_then(Value::as_f64).unwrap_or(f64::NEG_INFINITY);
		let sb = b.get("score").and_then(Value::as_f64).unwrap_or(f64::NEG_INFINITY);
		sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
	});
	out.truncate(k);
	out
}

#[derive(serde::Deserialize)]
struct ChatTurn {
	role: String,
	content: String,
}

#[derive(serde::Deserialize)]
pub(super) struct AskBody {
	question: String,
	#[serde(default)]
	history: Vec<ChatTurn>,
	#[serde(default = "default_ask_k")]
	k: usize,
}

fn default_ask_k() -> usize { 8 }

static AGENT_SYSTEM_PROMPT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn build_agent_system_prompt() -> &'static str {
	AGENT_SYSTEM_PROMPT.get_or_init(|| {
		let defs = crate::mcp::tools::tool_definitions();
		let tools: Vec<String> = defs.iter().map(|d| {
			let name = d.get("name").and_then(Value::as_str).unwrap_or("");
			let desc = d.get("description").and_then(Value::as_str).unwrap_or("");
			format!("- {name}: {desc}")
		}).collect();
		format!(
			"You are kern's assistant. Answer questions using the provided memory context.\n\
			 You can call tools when the user asks to modify the knowledge graph (add/remove memories, \
			 manage anchors, create links). Do not call tools for questions — answer those directly.\n\n\
			 Available tools:\n{}\n\n\
			 To call a tool output exactly: <tool_call>{{\"name\":\"TOOL\",\"args\":{{...}}}}</tool_call>\n\
			 A tool call must be on its own, not embedded mid-sentence.",
			tools.join("\n")
		)
	})
}

struct ToolCall {
	name: String,
	args: Value,
}

/// Extract visible text and any <tool_call>…</tool_call> blocks from an LLM response.
fn extract_tool_calls(text: &str) -> (String, Vec<ToolCall>) {
	let mut visible = String::new();
	let mut calls = Vec::new();
	let mut rest = text;
	while let Some(open) = rest.find("<tool_call>") {
		visible.push_str(&rest[..open]);
		rest = &rest[open + "<tool_call>".len()..];
		if let Some(close) = rest.find("</tool_call>") {
			let json_str = rest[..close].trim();
			rest = &rest[close + "</tool_call>".len()..];
			if let Ok(v) = serde_json::from_str::<Value>(json_str) {
				if let Some(name) = v.get("name").and_then(Value::as_str) {
					let args = v.get("args").cloned().unwrap_or(Value::Object(Default::default()));
					calls.push(ToolCall { name: name.to_string(), args });
				}
			}
		}
	}
	visible.push_str(rest);
	(visible.trim().to_string(), calls)
}

/// Execute one tool on the first available peer. Returns (ok, result_text).
async fn exec_tool(client: &reqwest::Client, peers: &[String], name: &str, args: &Value) -> (bool, String) {
	let body = json!({ "name": name, "args": args });
	for addr in peers {
		let url = format!("http://{addr}/tool");
		if let Ok(r) = client.post(&url).timeout(FANOUT_TIMEOUT).json(&body).send().await {
			if let Ok(v) = r.json::<Value>().await {
				let ok = v.get("ok").and_then(Value::as_bool).unwrap_or(false);
				let result = v.get("result").and_then(Value::as_str).unwrap_or("done").to_string();
				return (ok, result);
			}
		}
	}
	(false, "no daemon available".to_string())
}

/// Hub oracle endpoint: embed the question once, fan retrieval out to peers,
/// merge sources by score, emit a `sources` SSE event, then run an agentic
/// tool loop. The LLM can call kern tools (ingest, anchor, forget, etc.) by
/// emitting <tool_call>…</tool_call> blocks; each is executed and the result
/// fed back before the final answer is streamed as `token` events.
pub(super) async fn ask(State(st): State<HubState>, Json(body): Json<AskBody>) -> Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>> {
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
		let peers = live_peers();
		let reqbody = json!({ "vec": vec, "question": q, "k": k });
		let mut tagged = Vec::new();
		let mut chains: Vec<String> = Vec::new();
		let mut reason_items: Vec<Value> = Vec::new();
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
				if let Some(rs) = v.get("reasons").and_then(Value::as_array) {
					for r in rs {
						let mut r = r.clone();
						let rid = r.get("id").and_then(Value::as_str).map(|s| s.to_string());
						if let (Some(o), Some(rid)) = (r.as_object_mut(), rid) {
							o.insert("id".into(), json!(format!("{addr}|{rid}")));
							if let Some(f) = o.get("from").and_then(Value::as_str).map(|s| s.to_string()) {
								o.insert("from".into(), json!(format!("{addr}|{f}")));
							}
							if let Some(t) = o.get("to").and_then(Value::as_str).map(|s| s.to_string()) {
								o.insert("to".into(), json!(format!("{addr}|{t}")));
							}
						}
						reason_items.push(r);
					}
				}
				tagged.push((format!("{addr}|"), json!({ "hits": v.get("sources").cloned().unwrap_or(json!([])) })));
			}
		}
		let mut merged = rank_peers(&tagged, k);
		for (n, s) in merged.iter_mut().enumerate() {
			if let Some(o) = s.as_object_mut() { o.insert("n".into(), json!(n + 1)); }
		}
		yield Ok(Event::default().event("sources").data(json!({ "entities": merged, "chains": chains, "reasons": reason_items }).to_string()));

		// Build initial message list with system prompt + history + context
		let user_prompt = build_ask_prompt(&merged, &chains, &q);
		let mut messages: Vec<(String, String)> = vec![("system".to_string(), build_agent_system_prompt().to_owned())];
		for t in body.history.iter().rev().take(6).rev() {
			let role = match t.role.as_str() {
				"assistant" | "system" => t.role.clone(),
				_ => "user".to_string(),
			};
			messages.push((role, t.content.clone()));
		}
		messages.push(("user".to_string(), user_prompt));

		// Hand off to the agentic tool loop. It is a standalone stream (no axum
		// State / HTTP dependency) so the iterate→detect-tool→exec→feed-back
		// cycle can be driven directly in tests with a stub LLM + peer set.
		for await ev in run_agent_loop(st.llm.clone(), st.client.clone(), peers, messages) {
			yield ev;
		}
	};
	Sse::new(stream)
}

/// The agentic generation loop, factored out of [`ask`] so it carries no axum
/// `State`/HTTP coupling. Given the assembled `messages`, it repeatedly asks the
/// LLM, parses any `<tool_call>` blocks, executes them against `peers`, and
/// feeds the results back — up to `MAX_ITERS` rounds — emitting `token`,
/// `tool_call`, `tool_result`, `error`, and a terminal `done` SSE event. Returns
/// a stream so the caller just forwards events; testable without binding a port.
fn run_agent_loop(
	llm: crate::llm::Client,
	client: reqwest::Client,
	peers: Vec<String>,
	mut messages: Vec<(String, String)>,
) -> impl futures_core::Stream<Item = Result<Event, Infallible>> {
	async_stream::stream! {
		const MAX_ITERS: usize = 8;
		let mut tool_idx = 0usize;

		for _iter in 0..MAX_ITERS {
			// Collect full response so we can detect tool calls before emitting anything.
			let mut response = String::new();
			let mut gen = Box::pin(llm.answer(crate::llm::AnswerParams {
				messages: messages.clone(),
				stream: false,
				num_predict: None,
			}));
			let mut had_err = false;
			while let Some(item) = gen.next().await {
				match item {
					Ok(tok) => response.push_str(&tok),
					Err(e) => {
						yield Ok(Event::default().event("error").data(json!({ "message": e.to_string() }).to_string()));
						had_err = true;
						break;
					}
				}
			}
			if had_err { return; }

			let (visible, tool_calls) = extract_tool_calls(&response);

			// Emit visible text (text outside any tool call blocks)
			if !visible.is_empty() {
				yield Ok(Event::default().event("token").data(json!({ "t": visible }).to_string()));
			}

			if tool_calls.is_empty() {
				break;
			}

			// Execute each tool call and emit events
			let mut result_ctx = String::new();
			for tc in &tool_calls {
				let idx = tool_idx;
				tool_idx += 1;
				yield Ok(Event::default().event("tool_call").data(
					json!({ "name": tc.name, "args": tc.args, "idx": idx }).to_string()
				));
				let (ok, result_text) = exec_tool(&client, &peers, &tc.name, &tc.args).await;
				yield Ok(Event::default().event("tool_result").data(
					json!({ "name": tc.name, "result": result_text, "ok": ok, "idx": idx }).to_string()
				));
				result_ctx.push_str(&format!("Tool `{}` result: {}\n", tc.name, result_text));
			}

			// Feed tool results back and continue
			messages.push(("assistant".to_string(), response));
			messages.push(("user".to_string(), format!("{result_ctx}Continue.")));
		}

		yield Ok(Event::default().event("done").data("{}"));
	}
}

/// Hub endpoint: broadcast a tool call to all live peers, return first success.
pub(super) async fn hub_tool(State(st): State<HubState>, Json(body): Json<Value>) -> Json<Value> {
	let peers = live_peers();
	if peers.is_empty() {
		return Json(json!({ "ok": false, "error": "no daemons available" }));
	}
	let mut last_ok: Option<Value> = None;
	let mut errors: Vec<String> = Vec::new();
	for addr in &peers {
		let url = format!("http://{addr}/tool");
		match st.client.post(&url).timeout(FANOUT_TIMEOUT).json(&body).send().await {
			Ok(r) => match r.json::<Value>().await {
				Ok(v) => {
					if v.get("ok").and_then(Value::as_bool).unwrap_or(false) {
						last_ok = Some(v);
					} else {
						errors.push(v.get("error").and_then(Value::as_str).unwrap_or("error").to_string());
					}
				}
				Err(e) => errors.push(e.to_string()),
			},
			Err(e) => errors.push(e.to_string()),
		}
	}
	last_ok.map(Json).unwrap_or_else(|| Json(json!({ "ok": false, "error": errors.join("; ") })))
}

/// Hub endpoint: forward an edit to the peer that owns the namespaced id.
pub(super) async fn hub_edit(State(st): State<HubState>, Json(mut body): Json<Value>) -> Json<Value> {
	let id = body.get("id").and_then(Value::as_str).unwrap_or("").to_string();
	let Some((addr, real)) = id.split_once('|') else {
		return Json(json!({ "ok": false, "error": "bad id" }));
	};
	if let Some(o) = body.as_object_mut() {
		o.insert("id".into(), json!(real));
	}
	let url = format!("http://{addr}/edit");
	match st.client.post(&url).json(&body).send().await {
		Ok(r) => match r.json::<Value>().await {
			Ok(v) => Json(v),
			Err(_) => Json(json!({ "ok": false, "error": "peer decode" })),
		},
		Err(_) => Json(json!({ "ok": false, "error": "peer unreachable" })),
	}
}

/// Build the generation prompt from merged source texts + per-daemon chain
/// strings. Numbers each fact so the model can cite them as `[n]`, which the
/// browser links back to the source tiles.
fn build_ask_prompt(sources: &[Value], chains: &[String], question: &str) -> String {
	let mut p = String::from("Context from knowledge graph:\n\n");
	// Cap the provenance chains in the PROMPT — format_chains can emit kilobytes
	// (full entity texts repeated across chains), which balloons prompt-eval
	// latency on local CPU models. The full chains still reach the UI via the
	// `sources` event; the model only needs a compact structural hint.
	let joined: String = chains
		.iter()
		.map(|c| c.trim())
		.filter(|c| !c.is_empty())
		.collect::<Vec<_>>()
		.join("\n");
	if !joined.is_empty() {
		let cap = joined.char_indices().nth(800).map(|(i, _)| i).unwrap_or(joined.len());
		p.push_str(&joined[..cap]);
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn merge_peer_namespaces_ids_and_keeps_links_valid() {
		let payload = json!({
			"nodes": [{ "id": "e1", "kern": "k1", "label": "x" }],
			"links": [{ "source": "e1", "target": "e2", "kind": "Supports" }],
			"kerns": [
				{ "id": "k0", "parent": null, "children": ["k1"] },
				{ "id": "k1", "parent": "k0", "children": [] },
			],
		});
		let (mut n, mut l, mut k) = (Vec::new(), Vec::new(), Vec::new());
		merge_peer("127.0.0.1:7701|", &payload, &mut n, &mut l, &mut k);

		assert_eq!(n[0]["id"], "127.0.0.1:7701|e1");
		assert_eq!(n[0]["kern"], "127.0.0.1:7701|k1");
		// Endpoints carry the same tag, so the edge still resolves post-merge.
		assert_eq!(l[0]["source"], "127.0.0.1:7701|e1");
		assert_eq!(l[0]["target"], "127.0.0.1:7701|e2");
		// Root stays parentless; child parent/children references are re-keyed.
		assert!(k[0]["parent"].is_null());
		assert_eq!(k[0]["children"][0], "127.0.0.1:7701|k1");
		assert_eq!(k[1]["parent"], "127.0.0.1:7701|k0");
	}

	#[test]
	fn merge_peer_tolerates_missing_arrays() {
		let (mut n, mut l, mut k) = (Vec::new(), Vec::new(), Vec::new());
		merge_peer("t|", &json!({}), &mut n, &mut l, &mut k);
		assert!(n.is_empty() && l.is_empty() && k.is_empty());
	}

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
		assert!(p.contains("[n]"));
	}

	#[test]
	fn rank_peers_namespaces_pools_sorts_and_truncates() {
		// Two peers. Each returns entity hits + reason hits with scores.
		let peer_a = json!({
			"hits":    [{ "id": "e1", "kern": "k1", "label": "a", "score": 0.40 }],
			"reasons": [{ "id": "e9", "kern": "k1", "label": "ra", "score": 0.95 }],
		});
		let peer_b = json!({
			"hits":    [{ "id": "e2", "kern": "k2", "label": "b", "score": 0.70 }],
			"reasons": [],
		});
		let tagged = vec![
			("A|".to_string(), peer_a),
			("B|".to_string(), peer_b),
		];
		let out = rank_peers(&tagged, 2);

		// Truncated to k=2, sorted by score desc across BOTH peers and BOTH arrays.
		assert_eq!(out.len(), 2);
		assert_eq!(out[0]["score"], 0.95);
		assert_eq!(out[1]["score"], 0.70);
		// ids + kern are namespaced by peer tag so they match what /graph shipped.
		assert_eq!(out[0]["id"], "A|e9");
		assert_eq!(out[0]["kern"], "A|k1");
		assert_eq!(out[1]["id"], "B|e2");
	}

	// --- extract_tool_calls: the fragile <tool_call> string-scan parser. ---

	#[test]
	fn extract_tool_calls_pulls_call_and_strips_from_visible() {
		let raw = "Adding that now. <tool_call>{\"name\":\"ingest\",\"args\":{\"text\":\"hi\"}}</tool_call> Done.";
		let (visible, calls) = extract_tool_calls(raw);
		assert_eq!(calls.len(), 1);
		assert_eq!(calls[0].name, "ingest");
		assert_eq!(calls[0].args["text"], "hi");
		// The block is removed from the user-visible text; surrounding prose stays.
		assert!(!visible.contains("tool_call"));
		assert!(visible.contains("Adding that now."));
		assert!(visible.contains("Done."));
	}

	#[test]
	fn extract_tool_calls_handles_missing_args_and_multiple_blocks() {
		let raw = "<tool_call>{\"name\":\"pulse\"}</tool_call><tool_call>{\"name\":\"anchor\",\"args\":{\"id\":\"x\"}}</tool_call>";
		let (visible, calls) = extract_tool_calls(raw);
		assert_eq!(calls.len(), 2);
		assert_eq!(calls[0].name, "pulse");
		// Missing args defaults to an empty object, not a panic.
		assert_eq!(calls[0].args, json!({}));
		assert_eq!(calls[1].name, "anchor");
		assert_eq!(calls[1].args["id"], "x");
		assert!(visible.is_empty());
	}

	#[test]
	fn extract_tool_calls_ignores_malformed_json_and_unclosed_blocks() {
		// Invalid JSON inside the block yields no call; an unclosed block ends the scan.
		let bad = "<tool_call>not json</tool_call>plain";
		let (visible, calls) = extract_tool_calls(bad);
		assert!(calls.is_empty());
		assert_eq!(visible, "plain");

		let unclosed = "before <tool_call>{\"name\":\"x\"} and no close";
		let (visible2, calls2) = extract_tool_calls(unclosed);
		assert!(calls2.is_empty());
		// Everything before the unterminated marker survives as visible text.
		assert!(visible2.contains("before"));
	}

	#[test]
	fn extract_tool_calls_plain_text_is_unchanged() {
		let (visible, calls) = extract_tool_calls("just a normal answer with no tools");
		assert!(calls.is_empty());
		assert_eq!(visible, "just a normal answer with no tools");
	}
}
