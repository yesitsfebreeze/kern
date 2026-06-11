//! LoCoMo eval corpus: loader + answer scorers (#36).
//!
//! Dataset: Maharana et al., ACL 2024 (arXiv 2402.17753),
//! `snap-research/locomo` → `data/locomo10.json`. CC BY-NC 4.0 — supplied via a
//! path (`KERN_LOCOMO_PATH`), never redistributed in-repo.
//!
//! This module is the pure half of the harness: parse the JSON into ordered
//! sessions + QA, and score predicted answers (token-F1, ROUGE-L, abstention
//! detection for the adversarial category). The live half — driving
//! capture→distill→retrieve against ollama and the LLM-judge — lives in the
//! `locomo_eval` binary.

use std::collections::HashMap;

/// One conversational turn. Image turns fold their BLIP caption into `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
	pub speaker: String,
	pub dia_id: String,
	pub text: String,
}

/// One dated session of turns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
	/// 1-based session index parsed from the `session_N` key.
	pub index: u32,
	pub date_time: String,
	pub turns: Vec<Turn>,
}

/// One QA probe. Category 5 is adversarial (unanswerable): `answer` is `None`
/// and `adversarial_answer` holds the plausible-but-unsupported distractor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QaItem {
	pub question: String,
	pub answer: Option<String>,
	pub adversarial_answer: Option<String>,
	pub evidence: Vec<String>,
	pub category: u8,
}

impl QaItem {
	pub fn is_adversarial(&self) -> bool {
		self.category == 5
	}
}

/// One LoCoMo dialogue: ordered sessions + its QA set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
	pub sample_id: String,
	pub sessions: Vec<Session>,
	pub qa: Vec<QaItem>,
}

/// Load + parse a LoCoMo dataset file.
pub fn load(path: &str) -> Result<Vec<Sample>, String> {
	let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
	parse_dataset(&raw)
}

/// Parse the LoCoMo JSON corpus into ordered samples.
pub fn parse_dataset(json: &str) -> Result<Vec<Sample>, String> {
	let raw: Vec<RawSample> =
		serde_json::from_str(json).map_err(|e| format!("locomo json: {e}"))?;
	raw.into_iter().map(convert_sample).collect()
}

/// LoCoMo category label. 1=multi-hop, 2=temporal, 3=open-domain,
/// 4=single-hop, 5=adversarial.
pub fn category_name(cat: u8) -> &'static str {
	match cat {
		1 => "multi-hop",
		2 => "temporal",
		3 => "open-domain",
		4 => "single-hop",
		5 => "adversarial",
		_ => "unknown",
	}
}

/// SQuAD-style answer normalization: lowercase, strip punctuation, drop the
/// articles a/an/the, collapse whitespace.
pub fn normalize_answer(s: &str) -> String {
	s.to_lowercase()
		.chars()
		.map(|c| if c.is_alphanumeric() { c } else { ' ' })
		.collect::<String>()
		.split_whitespace()
		.filter(|t| !matches!(*t, "a" | "an" | "the"))
		.collect::<Vec<_>>()
		.join(" ")
}

/// Token-level F1 between predicted and gold answer (normalized).
pub fn token_f1(pred: &str, gold: &str) -> f64 {
	let p: Vec<String> = normalize_answer(pred).split_whitespace().map(String::from).collect();
	let g: Vec<String> = normalize_answer(gold).split_whitespace().map(String::from).collect();
	if p.is_empty() || g.is_empty() {
		// Both empty → vacuous match; exactly one empty → no overlap.
		return if p.is_empty() && g.is_empty() { 1.0 } else { 0.0 };
	}
	let mut gold_counts: HashMap<&str, usize> = HashMap::new();
	for t in &g {
		*gold_counts.entry(t.as_str()).or_insert(0) += 1;
	}
	let mut common = 0usize;
	for t in &p {
		if let Some(c) = gold_counts.get_mut(t.as_str()) {
			if *c > 0 {
				*c -= 1;
				common += 1;
			}
		}
	}
	if common == 0 {
		return 0.0;
	}
	let precision = common as f64 / p.len() as f64;
	let recall = common as f64 / g.len() as f64;
	2.0 * precision * recall / (precision + recall)
}

/// ROUGE-L F1 (longest-common-subsequence based) over normalized tokens.
pub fn rouge_l(pred: &str, gold: &str) -> f64 {
	let p: Vec<String> = normalize_answer(pred).split_whitespace().map(String::from).collect();
	let g: Vec<String> = normalize_answer(gold).split_whitespace().map(String::from).collect();
	if p.is_empty() || g.is_empty() {
		return if p.is_empty() && g.is_empty() { 1.0 } else { 0.0 };
	}
	let lcs = lcs_len(&p, &g) as f64;
	if lcs == 0.0 {
		return 0.0;
	}
	let precision = lcs / p.len() as f64;
	let recall = lcs / g.len() as f64;
	2.0 * precision * recall / (precision + recall)
}

/// Heuristic: does the prediction decline to answer? Used to score the
/// adversarial category, where the correct behavior is abstention.
pub fn is_abstention(pred: &str) -> bool {
	let p = pred.to_lowercase();
	const MARKERS: [&str; 16] = [
		"don't have", "do not have", "not mentioned", "no mention", "no information",
		"not specified", "not stated", "not provided", "not available", "don't know",
		"do not know", "cannot", "can't", "unanswerable", "no answer", "not enough information",
	];
	MARKERS.iter().any(|m| p.contains(m))
}

/// Build the LLM-judge prompt: decide whether `pred` answers the question as
/// well as the gold `answer`. The judge replies CORRECT / INCORRECT.
pub fn judge_prompt(question: &str, gold: &str, pred: &str) -> String {
	format!(
		"You are grading a question-answering system against a gold answer.\n\
		 Question: {question}\n\
		 Gold answer: {gold}\n\
		 Predicted answer: {pred}\n\n\
		 Does the predicted answer convey the same factual information as the gold \
		 answer? Minor wording, formatting, or extra context is fine. Reply with a \
		 single word: CORRECT or INCORRECT."
	)
}

/// Parse a judge reply into a boolean verdict. `INCORRECT` wins over `CORRECT`
/// (the latter is a substring of the former); anything unrecognized is treated
/// as incorrect.
pub fn parse_judge_verdict(raw: &str) -> bool {
	let up = raw.to_uppercase();
	if up.contains("INCORRECT") {
		false
	} else {
		up.contains("CORRECT")
	}
}

/// Length of the longest common subsequence of two token slices.
///
/// Rolling-row DP: `O(a.len() * b.len())` time, `O(b.len())` space. The
/// quadratic product is fine because both inputs are *normalized-answer token
/// lists* — for LoCoMo these are short (a handful to a few dozen tokens), and
/// `rouge_l` never feeds document-length text here, so `m*n` stays tiny. If a
/// future caller passes long sequences, cap the token count before calling.
fn lcs_len(a: &[String], b: &[String]) -> usize {
	let mut dp = vec![0usize; b.len() + 1];
	for ai in a {
		let mut prev = 0usize;
		for (j, bj) in b.iter().enumerate() {
			let cur = dp[j + 1];
			dp[j + 1] = if ai == bj { prev + 1 } else { dp[j + 1].max(dp[j]) };
			prev = cur;
		}
	}
	dp[b.len()]
}

// ── Raw deserialization shapes (the on-disk LoCoMo schema) ──────────────────

#[derive(serde::Deserialize)]
struct RawSample {
	sample_id: String,
	qa: Vec<RawQa>,
	conversation: serde_json::Map<String, serde_json::Value>,
}

#[derive(serde::Deserialize)]
struct RawQa {
	question: String,
	#[serde(default)]
	answer: Option<serde_json::Value>,
	#[serde(default)]
	adversarial_answer: Option<String>,
	#[serde(default)]
	evidence: Vec<String>,
	category: u8,
}

#[derive(serde::Deserialize)]
struct RawTurn {
	speaker: String,
	dia_id: String,
	#[serde(default)]
	text: String,
	#[serde(default)]
	blip_caption: Option<String>,
}

fn convert_sample(raw: RawSample) -> Result<Sample, String> {
	// Collect (index → date_time) and (index → turns) from the dynamic keys,
	// then pair and sort by index.
	let mut dates: HashMap<u32, String> = HashMap::new();
	let mut turn_sets: HashMap<u32, Vec<Turn>> = HashMap::new();
	for (key, val) in &raw.conversation {
		let Some(rest) = key.strip_prefix("session_") else { continue };
		if let Some(idx) = rest.strip_suffix("_date_time") {
			if let Ok(n) = idx.parse::<u32>() {
				dates.insert(n, val.as_str().unwrap_or_default().to_string());
			}
		} else if let Ok(n) = rest.parse::<u32>() {
			let turns: Vec<RawTurn> = serde_json::from_value(val.clone())
				.map_err(|e| format!("session_{n} turns: {e}"))?;
			turn_sets.insert(n, turns.into_iter().map(convert_turn).collect());
		}
	}
	let mut sessions: Vec<Session> = turn_sets
		.into_iter()
		.map(|(index, turns)| Session {
			index,
			date_time: dates.get(&index).cloned().unwrap_or_default(),
			turns,
		})
		.collect();
	sessions.sort_by_key(|s| s.index);

	let qa = raw.qa.into_iter().map(convert_qa).collect();
	Ok(Sample { sample_id: raw.sample_id, sessions, qa })
}

fn convert_turn(t: RawTurn) -> Turn {
	let text = match t.blip_caption.as_deref() {
		Some(cap) if !cap.is_empty() && !t.text.is_empty() => {
			format!("{} [shared image: {cap}]", t.text)
		}
		Some(cap) if !cap.is_empty() => format!("[shared image: {cap}]"),
		_ => t.text,
	};
	Turn { speaker: t.speaker, dia_id: t.dia_id, text }
}

fn convert_qa(q: RawQa) -> QaItem {
	let answer = match q.answer {
		Some(serde_json::Value::String(s)) => Some(s),
		Some(serde_json::Value::Number(n)) => Some(n.to_string()),
		Some(serde_json::Value::Null) | None => None,
		Some(other) => Some(other.to_string()),
	};
	QaItem {
		question: q.question,
		answer,
		adversarial_answer: q.adversarial_answer,
		evidence: q.evidence,
		category: q.category,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const FIXTURE: &str = r#"[
	  {
	    "sample_id": "conv-1",
	    "conversation": {
	      "speaker_a": "Caroline",
	      "speaker_b": "Mel",
	      "session_2_date_time": "2:00 pm on 8 May, 2023",
	      "session_2": [
	        {"speaker": "Mel", "dia_id": "D2:1", "text": "I ran a charity race.", "blip_caption": "a person running"}
	      ],
	      "session_1_date_time": "1:00 pm on 7 May, 2023",
	      "session_1": [
	        {"speaker": "Caroline", "dia_id": "D1:1", "text": "Hey Mel!"},
	        {"speaker": "Mel", "dia_id": "D1:2", "text": "Hi!"}
	      ]
	    },
	    "qa": [
	      {"question": "When did the race happen?", "answer": "8 May 2023", "evidence": ["D2:1"], "category": 2},
	      {"question": "How many medals?", "answer": 3, "evidence": ["D2:1"], "category": 4},
	      {"question": "What did Caroline realize?", "evidence": ["D2:1"], "category": 5, "adversarial_answer": "self-care matters"}
	    ]
	  }
	]"#;

	#[test]
	fn parses_sessions_in_index_order() {
		let samples = parse_dataset(FIXTURE).unwrap();
		assert_eq!(samples.len(), 1);
		let s = &samples[0];
		assert_eq!(s.sample_id, "conv-1");
		assert_eq!(s.sessions.len(), 2);
		// Ordered by index despite session_2 appearing first in the JSON.
		assert_eq!(s.sessions[0].index, 1);
		assert_eq!(s.sessions[1].index, 2);
		assert_eq!(s.sessions[0].date_time, "1:00 pm on 7 May, 2023");
		assert_eq!(s.sessions[0].turns.len(), 2);
		assert_eq!(s.sessions[0].turns[0].speaker, "Caroline");
		assert_eq!(s.sessions[0].turns[0].dia_id, "D1:1");
	}

	#[test]
	fn folds_blip_caption_into_image_turn_text() {
		let s = &parse_dataset(FIXTURE).unwrap()[0];
		let img_turn = &s.sessions[1].turns[0];
		assert!(img_turn.text.contains("I ran a charity race."));
		assert!(img_turn.text.contains("a person running"));
	}

	#[test]
	fn parses_qa_answer_kinds() {
		let qa = &parse_dataset(FIXTURE).unwrap()[0].qa;
		assert_eq!(qa.len(), 3);
		// string answer
		assert_eq!(qa[0].answer.as_deref(), Some("8 May 2023"));
		// integer answer coerced to string
		assert_eq!(qa[1].answer.as_deref(), Some("3"));
		// adversarial: no answer, has distractor
		assert!(qa[2].answer.is_none());
		assert!(qa[2].is_adversarial());
		assert_eq!(qa[2].adversarial_answer.as_deref(), Some("self-care matters"));
	}

	#[test]
	fn category_names() {
		assert_eq!(category_name(1), "multi-hop");
		assert_eq!(category_name(2), "temporal");
		assert_eq!(category_name(3), "open-domain");
		assert_eq!(category_name(4), "single-hop");
		assert_eq!(category_name(5), "adversarial");
	}

	#[test]
	fn normalize_strips_articles_punct_case() {
		assert_eq!(normalize_answer("The Answer!"), "answer");
		assert_eq!(normalize_answer("a cat, an  apple"), "cat apple");
	}

	#[test]
	fn f1_identical_is_one() {
		assert!((token_f1("7 May 2023", "7 May 2023") - 1.0).abs() < 1e-9);
	}

	#[test]
	fn f1_disjoint_is_zero() {
		assert_eq!(token_f1("blue", "orange"), 0.0);
	}

	#[test]
	fn f1_partial_overlap() {
		// pred {cat,sat}, gold {cat,sat,down}: P=1, R=2/3, F1=0.8
		assert!((token_f1("the cat sat", "cat sat down") - 0.8).abs() < 1e-9);
	}

	#[test]
	fn rouge_l_identical_is_one() {
		assert!((rouge_l("x b c d", "x b c d") - 1.0).abs() < 1e-9);
	}

	#[test]
	fn rouge_l_lcs_partial() {
		// pred [x,b,c,d], gold [x,c,d]: LCS=3, R=1, P=3/4, F=6/7
		assert!((rouge_l("x b c d", "x c d") - 6.0 / 7.0).abs() < 1e-9);
	}

	#[test]
	fn judge_verdict_parses_correct_and_incorrect() {
		assert!(parse_judge_verdict("CORRECT"));
		assert!(parse_judge_verdict("The answer is correct."));
		// "INCORRECT" contains "correct" — must not be read as a positive.
		assert!(!parse_judge_verdict("INCORRECT"));
		assert!(!parse_judge_verdict("This is incorrect"));
		// unrecognized → incorrect
		assert!(!parse_judge_verdict("maybe"));
	}

	#[test]
	fn judge_prompt_includes_all_three_parts() {
		let p = judge_prompt("Q?", "gold-ans", "pred-ans");
		assert!(p.contains("Q?"));
		assert!(p.contains("gold-ans"));
		assert!(p.contains("pred-ans"));
	}

	#[test]
	fn abstention_detected() {
		assert!(is_abstention("I don't have information about that."));
		assert!(is_abstention("That was not mentioned in the conversation."));
		assert!(!is_abstention("7 May 2023"));
	}

	#[test]
	fn abstention_is_case_insensitive_and_position_independent() {
		// pred is lowercased before matching, so casing of the marker is irrelevant.
		assert!(is_abstention("Cannot determine that from the context"), "mixed-case 'Cannot'");
		assert!(is_abstention("CANNOT"), "all-caps marker");
		assert!(is_abstention("unanswerable given the dialogue"), "marker at the very start");
		assert!(is_abstention("...ultimately this is unanswerable"), "marker at the end");
		// Unicode around a marker doesn't break detection (lowercasing is char-wise).
		assert!(is_abstention("Désolé — I don't know. 不知道"));
		// Unicode content that simply answers is not a false positive.
		assert!(!is_abstention("réponse : 7 mai 2023"));
	}

	#[test]
	fn rouge_l_all_article_string_normalizes_to_empty() {
		// Every token is a stripped article -> empty normalized form.
		assert_eq!(normalize_answer("the a an"), "");
		// Empty vs empty is a vacuous match (1.0); empty vs content is 0.0 — the
		// guard in rouge_l handles both without a divide-by-zero.
		assert!((rouge_l("the a an", "an the a") - 1.0).abs() < 1e-9);
		assert_eq!(rouge_l("the a an", "real content here"), 0.0);
	}
}
