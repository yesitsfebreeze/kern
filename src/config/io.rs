use std::path::Path;

use serde::de::DeserializeOwned;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
	#[error("io: {0}")]
	Io(String),
	#[error("parse: {0}")]
	Parse(String),
}

pub fn load_layered<T: DeserializeOwned + Default>(
	user: &Path,
	project: &Path,
) -> Result<T, Error> {
	let user_v = read_value(user)?;
	let project_v = read_value(project)?;
	let merged = merge_sections(user_v, project_v);
	merged
		.try_into()
		.map_err(|e: toml::de::Error| Error::Parse(e.to_string()))
}

fn read_value(path: &Path) -> Result<toml::Value, Error> {
	match std::fs::read_to_string(path) {
		// Parse as a document `toml::Table` — a bare-`Value` parse misreads a leading
		// `[section]` header as an array (see read_value_parses_leading_section_header).
		Ok(text) => text
			.parse::<toml::Table>()
			.map(toml::Value::Table)
			.map_err(|e| Error::Parse(e.to_string())),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
			Ok(toml::Value::Table(toml::value::Table::new()))
		}
		Err(e) => Err(Error::Io(e.to_string())),
	}
}

/// Section-level merge: a top-level key in `over` REPLACES `base`'s wholesale — NO
/// deep merge, so user fields the project omits are LOST. Keep a section in one scope.
fn merge_sections(base: toml::Value, over: toml::Value) -> toml::Value {
	match (base, over) {
		(toml::Value::Table(mut a), toml::Value::Table(b)) => {
			for (k, v) in b {
				a.insert(k, v);
			}
			toml::Value::Table(a)
		}
		(_, over) => over,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn read_value_parses_leading_section_header() {
		let dir = std::env::temp_dir().join(format!("cfgio_rv_{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let p = dir.join("c.toml");
		std::fs::write(&p, "[section]\nenabled = true\n").unwrap();
		let v = read_value(&p).expect("read_value should parse a document");
		let enabled = v
			.get("section")
			.and_then(|s| s.get("enabled"))
			.and_then(|b| b.as_bool());
		assert_eq!(enabled, Some(true));
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn load_layered_merges_project_section_over_missing_user() {
		let dir = std::env::temp_dir().join(format!("cfgio_ll_{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let user = dir.join("user.toml");
		let project = dir.join("project.toml");
		std::fs::write(&project, "[section]\nenabled = true\n").unwrap();
		let merged: toml::Table = load_layered(&user, &project).expect("load_layered");
		let enabled = merged
			.get("section")
			.and_then(|s| s.get("enabled"))
			.and_then(|b| b.as_bool());
		assert_eq!(enabled, Some(true));
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn load_layered_project_section_wholly_replaces_user_section() {
		let dir = std::env::temp_dir().join(format!("cfgio_ovr_{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let user = dir.join("user.toml");
		let project = dir.join("project.toml");
		std::fs::write(&user, "[embed]\nurl = \"user-url\"\nkey = \"secret\"\n").unwrap();
		std::fs::write(&project, "[embed]\nurl = \"proj-url\"\n").unwrap();

		let merged: toml::Table = load_layered(&user, &project).expect("load_layered");
		let embed = merged
			.get("embed")
			.and_then(|v| v.as_table())
			.expect("embed table");
		assert_eq!(
			embed.get("url").and_then(|v| v.as_str()),
			Some("proj-url"),
			"project section wins"
		);
		assert!(
			embed.get("key").is_none(),
			"user `key` is NOT inherited — section wholly replaced"
		);
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn load_layered_keeps_sections_present_in_only_one_scope() {
		let dir = std::env::temp_dir().join(format!("cfgio_keep_{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let user = dir.join("user.toml");
		let project = dir.join("project.toml");
		std::fs::write(&user, "[reason]\nmodel = \"qwen\"\n").unwrap();
		std::fs::write(&project, "[embed]\nurl = \"p\"\n").unwrap();

		let merged: toml::Table = load_layered(&user, &project).expect("load");
		assert_eq!(
			merged
				.get("reason")
				.and_then(|s| s.get("model"))
				.and_then(|v| v.as_str()),
			Some("qwen"),
			"user-only [reason] survives",
		);
		assert!(
			merged.get("embed").is_some(),
			"project-only [embed] is present too"
		);
		let _ = std::fs::remove_dir_all(&dir);
	}
}
