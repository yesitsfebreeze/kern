pub fn split(text: &str, descriptor: &str, llm: Option<&dyn Fn(&str) -> String>) -> Vec<String> {
	if let Some(llm_fn) = llm {
		let result = llm_split(text, descriptor, llm_fn);
		if !result.is_empty() {
			return result;
		}
	}
	paragraph_split(text)
}

pub(crate) fn llm_split(text: &str, descriptor: &str, llm: &dyn Fn(&str) -> String) -> Vec<String> {
	let context = if descriptor.is_empty() {
		String::new()
	} else {
		format!(" This text describes {descriptor}.")
	};
	let prompt = format!(
		"Extract the key factual statements from the following text.{context} \
		 One statement per line. No numbering. No commentary.\n\n{text}"
	);
	let response = llm(&prompt);
	if response.is_empty() {
		return Vec::new();
	}
	trim_nonempty(response.lines())
}

pub(crate) fn paragraph_split(text: &str) -> Vec<String> {
	let chunks = trim_nonempty(text.split("\n\n"));
	if chunks.is_empty() {
		vec![text.to_string()]
	} else {
		chunks
	}
}

/// Trim each part and drop the empties. Shared by the line-based and
/// paragraph-based splitters.
fn trim_nonempty<'a>(parts: impl Iterator<Item = &'a str>) -> Vec<String> {
	parts
		.map(|p| p.trim().to_string())
		.filter(|p| !p.is_empty())
		.collect()
}
