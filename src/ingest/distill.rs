//! LLM-gated distillation of a raw conversation into durable claims.
//!
//! Pure-ish: the only side effect is the injected LLM call. The caller
//! (capture_spool) turns each `Claim` into an ingested thought.

/// One durable, reusable piece of knowledge extracted from a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
	/// Self-contained statement worth remembering across sessions.
	pub text: String,
	/// Descriptor key (the typed-memory taxonomy). One of `DESCRIPTORS`.
	pub descriptor: String,
}

/// The typed-memory taxonomy. Mirrors the descriptors seeded into the kern.
pub const DESCRIPTORS: [&str; 6] = [
	"preference", "decision", "project", "fact", "code-fact", "reference",
];

/// Extract durable claims from `conversation`. Returns `[]` when the
/// conversation is empty or the LLM produces no parseable JSON array.
pub fn distill(conversation: &str, llm: &dyn Fn(&str) -> String) -> Vec<Claim> {
	if conversation.trim().is_empty() {
		return Vec::new();
	}
	let prompt = format!(
		"Extract durable, reusable knowledge from this conversation between a \
user and an AI coding assistant. Output ONLY a JSON array. Each element must be \
{{\"text\": \"<one self-contained statement>\", \"kind\": \"<one of: preference, \
decision, project, fact, code-fact, reference>\"}}. Include only knowledge worth \
remembering across future sessions: user preferences, decisions and their \
rationale, ongoing project state, durable facts, structural code facts, and \
external references. \
Consolidate aggressively: emit ONE claim per distinct fact. Do NOT output \
multiple claims that restate the same idea, and do NOT output sentence \
fragments — each claim must be a complete, standalone statement that captures \
the fact in full. Prefer the single most complete phrasing over several \
partial ones. \
Skip greetings, acknowledgements, one-off task mechanics, and anything \
ephemeral. If nothing is worth keeping, output []. Do not wrap the array in \
markdown.\n\nCONVERSATION:\n{conversation}\n"
	);
	parse_claims(&llm(&prompt))
}

/// Parse claims from the first contiguous `[..]` span in `raw` (first `[`
/// to last `]`), tolerant of surrounding prose. A lone nested array
/// (`[[...]]`) is unwrapped. Malformed JSON or multiple sibling top-level
/// arrays fail gracefully to an empty vec. The JSON field `kind` maps to
/// `Claim::descriptor`, falling back to `"fact"` when missing or unknown.
fn parse_claims(raw: &str) -> Vec<Claim> {
	let (start, end) = match (raw.find('['), raw.rfind(']')) {
		(Some(s), Some(e)) if e > s => (s, e),
		_ => return Vec::new(),
	};
	let mut items: Vec<serde_json::Value> = match serde_json::from_str(&raw[start..=end]) {
		Ok(v) => v,
		Err(e) => {
			tracing::debug!(target: "kern.distill", error = %e, "claim JSON parse failed");
			return Vec::new();
		}
	};
	// LLMs sometimes wrap the array once more: `[[...]]`. Unwrap a lone
	// nested array so its claims are not silently dropped.
	if items.len() == 1 {
		if let serde_json::Value::Array(inner) = &items[0] {
			items = inner.clone();
		}
	}
	let mut out = Vec::new();
	for it in items {
		let text = it
			.get("text")
			.and_then(|v| v.as_str())
			.unwrap_or("")
			.trim()
			.to_string();
		if text.is_empty() {
			continue;
		}
		let kind_raw = it
			.get("kind")
			.and_then(|v| v.as_str())
			.unwrap_or("fact")
			.trim();
		let descriptor = if DESCRIPTORS.contains(&kind_raw) {
			kind_raw.to_string()
		} else {
			"fact".to_string()
		};
		out.push(Claim { text, descriptor });
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	fn stub(json: &'static str) -> impl Fn(&str) -> String {
		move |_q: &str| json.to_string()
	}

	#[test]
	fn extracts_claims_and_maps_kind() {
		let llm = stub(r#"[{"text":"User prefers tabs","kind":"preference"},{"text":"kern owns the graph","kind":"code-fact"}]"#);
		let claims = distill("some conversation", &llm);
		assert_eq!(claims.len(), 2);
		assert_eq!(claims[0].text, "User prefers tabs");
		assert_eq!(claims[0].descriptor, "preference");
		assert_eq!(claims[1].descriptor, "code-fact");
	}

	#[test]
	fn unknown_kind_falls_back_to_fact() {
		let llm = stub(r#"[{"text":"x","kind":"banana"}]"#);
		let claims = distill("c", &llm);
		assert_eq!(claims[0].descriptor, "fact");
	}

	#[test]
	fn bad_json_yields_empty() {
		let llm = stub("I could not find anything useful, sorry!");
		assert!(distill("c", &llm).is_empty());
	}

	#[test]
	fn empty_conversation_skips_llm() {
		let llm = stub(r#"[{"text":"should not appear","kind":"fact"}]"#);
		assert!(distill("   \n  ", &llm).is_empty());
	}

	#[test]
	fn tolerates_prose_around_json() {
		let llm = stub("Here you go:\n[{\"text\":\"a\",\"kind\":\"fact\"}]\nHope that helps");
		let claims = distill("c", &llm);
		assert_eq!(claims.len(), 1);
		assert_eq!(claims[0].text, "a");
	}

	#[test]
	fn absent_kind_falls_back_to_fact() {
		let llm = stub(r#"[{"text":"x"}]"#);
		let claims = distill("c", &llm);
		assert_eq!(claims.len(), 1);
		assert_eq!(claims[0].descriptor, "fact");
	}

	#[test]
	fn empty_or_missing_text_is_skipped() {
		let llm = stub(r#"[{"text":"","kind":"fact"},{"kind":"fact"},{"text":"keep","kind":"fact"}]"#);
		let claims = distill("c", &llm);
		assert_eq!(claims.len(), 1);
		assert_eq!(claims[0].text, "keep");
	}

	#[test]
	fn single_nested_array_is_unwrapped() {
		let llm = stub(r#"[[{"text":"a","kind":"fact"}]]"#);
		let claims = distill("c", &llm);
		assert_eq!(claims.len(), 1);
		assert_eq!(claims[0].text, "a");
	}
}
