//! Default data-type descriptors: one `SOURCE_*` kind (see
//! [`crate::base::constants`]) → the distiller's extraction hint for it.

use std::collections::HashMap;

use crate::base::constants::{
	AGENT_SOURCE, SOURCE_CHAT, SOURCE_CODE, SOURCE_CONFIG, SOURCE_DECISION, SOURCE_DEP, SOURCE_DIFF,
	SOURCE_DOC, SOURCE_ERROR, SOURCE_FILE, SOURCE_IDEA, SOURCE_LOG, SOURCE_REQUEST, SOURCE_SCHEMA,
	SOURCE_TEST,
};

pub fn default_descriptors() -> HashMap<String, String> {
	let pairs: &[(&str, &str)] = &[
		(SOURCE_CHAT, "A conversation turn between a user and an AI agent. Extract decisions made, questions asked, action items, and key information exchanged."),
		(SOURCE_REQUEST, "A user request or task description given to an AI agent. Extract the goal, constraints, acceptance criteria, and any referenced files or systems."),
		(SOURCE_DECISION, "An architectural or design decision. Extract the decision itself, the alternatives considered, the rationale, and any trade-offs noted."),
		(SOURCE_IDEA, "A brainstorm, hypothesis, or speculative note. Extract the core idea, any supporting reasoning, open questions, and connections to other concepts."),
		(SOURCE_FILE, "File content from the project filesystem. Extract the file's purpose, key exports or interfaces, dependencies, and structural patterns."),
		(SOURCE_CODE, "Source code from a programming language. Extract function signatures, type definitions, key algorithms, error handling patterns, and module boundaries."),
		(SOURCE_DIFF, "A git diff or patch showing changes to source files. Extract what was added, removed, or modified, the intent behind the change, and any files affected."),
		(SOURCE_ERROR, "A build error, test failure, or runtime exception. Extract the error type, message, location (file:line), likely cause, and any stack trace context."),
		(SOURCE_DOC, "Documentation such as a README, wiki page, or manual. Extract the subject, key concepts, usage instructions, API surface, and any warnings or caveats."),
		(SOURCE_TEST, "A test case or test output. Extract what is being tested, the expected vs actual behavior, assertion patterns, and pass/fail status."),
		(SOURCE_CONFIG, "A configuration file (YAML, TOML, JSON, .env). Extract key settings, their values, what they control, and any environment-specific overrides."),
		(SOURCE_LOG, "Log output or structured log entries. Extract timestamps, severity levels, error messages, request IDs, and any patterns or anomalies."),
		(SOURCE_SCHEMA, "A database schema, API schema, or interface definition. Extract entity names, field types, relationships, constraints, and versioning information."),
		(SOURCE_DEP, "Dependency information from a package manifest. Extract direct dependencies, version constraints, notable transitive dependencies, and any security notes."),
		(AGENT_SOURCE, "An AI agent-generated summary, plan, or reflection. Extract the key conclusions, next steps, open questions, and any referenced artifacts."),
	];
	pairs
		.iter()
		.map(|(k, v)| ((*k).to_string(), (*v).to_string()))
		.collect()
}

/// Insert missing defaults only, leaving existing keys untouched. Returns the
/// number newly inserted.
pub fn register_default_descriptors(descriptors: &mut HashMap<String, String>) -> usize {
	let defaults = default_descriptors();
	let mut n = 0;
	for (name, desc) in defaults {
		if let std::collections::hash_map::Entry::Vacant(e) = descriptors.entry(name) {
			e.insert(desc);
			n += 1;
		}
	}
	n
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_descriptors_have_no_colliding_keys() {
		// A count below 15 means two SOURCE_* consts share a string and one
		// silently overwrote the other (the AGENT_SOURCE/SOURCE_AGENT regression).
		let d = default_descriptors();
		assert_eq!(d.len(), 15, "every descriptor key must be unique");
		assert!(
			d.contains_key(AGENT_SOURCE),
			"agent source descriptor present"
		);
	}

	#[test]
	fn register_into_empty_map_inserts_all_defaults() {
		let mut m = HashMap::new();
		let n = register_default_descriptors(&mut m);
		assert_eq!(n, 15);
		assert_eq!(m.len(), 15);
		assert_eq!(register_default_descriptors(&mut m), 0);
	}
}
