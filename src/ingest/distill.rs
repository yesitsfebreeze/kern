//! LLM-gated distillation of a raw conversation into durable claims. Pure-ish:
//! the only side effect is the injected LLM call.

/// One durable, reusable piece of knowledge extracted from a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
	/// Self-contained statement worth remembering across sessions.
	pub text: String,
	/// Descriptor key (the typed-memory taxonomy). One of `DESCRIPTORS`.
	pub descriptor: String,
	/// Optional bi-temporal world-time hint (ISO8601), stamped onto `valid_from`.
	/// Parsed leniently — garbage or absent yields `None` (ingestion time).
	pub valid_from: Option<std::time::SystemTime>,
}

/// The typed-memory taxonomy. Mirrors the descriptors seeded into the kern.
pub const DESCRIPTORS: [&str; 7] = [
	"preference",
	"decision",
	"project",
	"fact",
	"code-fact",
	"reference",
	"procedural",
];

/// `Some([])` = the LLM responded but nothing parseable/worth keeping (archive).
/// `None` = no output at all (transient outage — caller must retry, not archive).
pub fn distill(conversation: &str, llm: &dyn Fn(&str) -> String) -> Option<Vec<Claim>> {
	if conversation.trim().is_empty() {
		return Some(Vec::new());
	}
	let prompt = format!(
		"Extract durable, reusable knowledge from this conversation between a \
user and an AI coding assistant. Output ONLY a JSON array. Each element must be \
{{\"text\": \"<one self-contained statement>\", \"kind\": \"<one of: preference, \
decision, project, fact, code-fact, reference, procedural>\"}}. Optionally add \
\"valid_from\": \"<ISO8601 date>\" ONLY when the statement itself says when it \
became true (e.g. \"since March 2026\", \"as of v2\"); omit it otherwise. \
Include only knowledge worth \
remembering across future sessions: user preferences, decisions and their \
rationale, ongoing project state, durable facts, structural code facts, \
external references, and procedural knowledge (learned workflows, rules, and \
conventions — how we do X, not just what is true). \
Consolidate aggressively: emit ONE claim per distinct fact. Do NOT output \
multiple claims that restate the same idea, and do NOT output sentence \
fragments — each claim must be a complete, standalone statement that captures \
the fact in full. Prefer the single most complete phrasing over several \
partial ones. \
Skip greetings, acknowledgements, one-off task mechanics, and anything \
ephemeral. If nothing is worth keeping, output []. Do not wrap the array in \
markdown.\n\nCONVERSATION:\n{conversation}\n"
	);
	let raw = llm(&prompt);
	if raw.trim().is_empty() {
		return None;
	}
	Some(parse_claims(&raw))
}

/// Parse the first-`[`-to-last-`]` span, tolerant of surrounding prose. A lone
/// nested `[[...]]` is unwrapped; malformed JSON and an unknown `kind` fail soft.
pub(crate) fn parse_claims(raw: &str) -> Vec<Claim> {
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
	// Unwrap a lone `[[...]]` wrapper (LLM quirk).
	if items.len() == 1 {
		if let Some(inner) = items[0].as_array_mut() {
			items = std::mem::take(inner);
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
		// A bad `valid_from` hint never blocks a good claim — ignore it.
		let valid_from = it
			.get("valid_from")
			.and_then(|v| v.as_str())
			.map(str::trim)
			.filter(|s| !s.is_empty())
			.and_then(|s| crate::base::time::parse_rfc3339(s).ok());
		out.push(Claim {
			text,
			descriptor,
			valid_from,
		});
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
		let llm = stub(
			r#"[{"text":"User prefers tabs","kind":"preference"},{"text":"kern owns the graph","kind":"code-fact"}]"#,
		);
		let claims = distill("some conversation", &llm).expect("some");
		assert_eq!(claims.len(), 2);
		assert_eq!(claims[0].text, "User prefers tabs");
		assert_eq!(claims[0].descriptor, "preference");
		assert_eq!(claims[1].descriptor, "code-fact");
	}

	#[test]
	fn procedural_kind_maps_through() {
		let llm = stub(r#"[{"text":"Always run cargo test before committing","kind":"procedural"}]"#);
		let claims = distill("c", &llm).expect("some");
		assert_eq!(claims.len(), 1);
		assert_eq!(claims[0].descriptor, "procedural");
		assert!(DESCRIPTORS.contains(&"procedural"));
	}

	#[test]
	fn unknown_kind_falls_back_to_fact() {
		let llm = stub(r#"[{"text":"x","kind":"banana"}]"#);
		let claims = distill("c", &llm).expect("some");
		assert_eq!(claims[0].descriptor, "fact");
	}

	#[test]
	fn bad_json_yields_empty() {
		let llm = stub("I could not find anything useful, sorry!");
		assert!(distill("c", &llm).expect("some").is_empty());
	}

	#[test]
	fn empty_conversation_skips_llm() {
		let llm = stub(r#"[{"text":"should not appear","kind":"fact"}]"#);
		assert!(distill("   \n  ", &llm).expect("some").is_empty());
	}

	#[test]
	fn empty_llm_response_signals_retry() {
		let llm = stub("");
		assert!(distill("a real conversation worth keeping", &llm).is_none());
	}

	#[test]
	fn whitespace_llm_response_signals_retry() {
		let llm = stub("   \n\t ");
		assert!(distill("a real conversation", &llm).is_none());
	}

	#[test]
	fn genuine_empty_array_is_some_empty() {
		let llm = stub("[]");
		assert_eq!(distill("a real conversation", &llm), Some(Vec::new()));
	}

	#[test]
	fn tolerates_prose_around_json() {
		let llm = stub("Here you go:\n[{\"text\":\"a\",\"kind\":\"fact\"}]\nHope that helps");
		let claims = distill("c", &llm).expect("some");
		assert_eq!(claims.len(), 1);
		assert_eq!(claims[0].text, "a");
	}

	#[test]
	fn valid_from_hint_is_parsed_when_present_and_ignored_when_garbage() {
		let good = stub(
			r#"[{"text":"we moved to spaces","kind":"decision","valid_from":"2026-03-01T00:00:00Z"}]"#,
		);
		let claims = distill("c", &good).expect("some");
		assert_eq!(claims.len(), 1);
		assert!(
			claims[0].valid_from.is_some(),
			"a valid ISO valid_from is parsed"
		);

		let garbage = stub(r#"[{"text":"x","kind":"fact","valid_from":"since March"}]"#);
		assert_eq!(
			distill("c", &garbage).expect("some")[0].valid_from,
			None,
			"an unparseable valid_from is ignored, not fatal"
		);

		let absent = stub(r#"[{"text":"y","kind":"fact"}]"#);
		assert_eq!(distill("c", &absent).expect("some")[0].valid_from, None);
	}

	#[test]
	fn absent_kind_falls_back_to_fact() {
		let llm = stub(r#"[{"text":"x"}]"#);
		let claims = distill("c", &llm).expect("some");
		assert_eq!(claims.len(), 1);
		assert_eq!(claims[0].descriptor, "fact");
	}

	#[test]
	fn empty_or_missing_text_is_skipped() {
		let llm = stub(r#"[{"text":"","kind":"fact"},{"kind":"fact"},{"text":"keep","kind":"fact"}]"#);
		let claims = distill("c", &llm).expect("some");
		assert_eq!(claims.len(), 1);
		assert_eq!(claims[0].text, "keep");
	}

	#[test]
	fn single_nested_array_is_unwrapped() {
		let llm = stub(r#"[[{"text":"a","kind":"fact"}]]"#);
		let claims = distill("c", &llm).expect("some");
		assert_eq!(claims.len(), 1);
		assert_eq!(claims[0].text, "a");
	}

	#[test]
	fn multiple_sibling_arrays_fail_gracefully_to_empty() {
		let two_siblings = stub(r#"[{"text":"a","kind":"fact"}] [{"text":"b","kind":"fact"}]"#);
		assert!(
			distill("c", &two_siblings).expect("some").is_empty(),
			"sibling arrays are not merged — invalid JSON spans to empty",
		);
		let array_of_arrays = stub(r#"[[{"text":"a","kind":"fact"}],[{"text":"b","kind":"fact"}]]"#);
		assert!(
			distill("c", &array_of_arrays).expect("some").is_empty(),
			"a len-2 array-of-arrays is neither unwrapped nor merged",
		);
	}
}
