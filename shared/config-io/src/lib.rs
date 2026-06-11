// Generic TOML load/save/layer for per-binary configs.
// User scope: <XDG_CONFIG>/kern/<bin>.toml
// Project scope: <cwd>/.kern/<bin>.toml
// Section-level merge: project TOML overrides whole sections; missing
// sections fall through to user, then defaults.

use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
	#[error("io: {0}")]
	Io(String),
	#[error("parse: {0}")]
	Parse(String),
	#[error("serialize: {0}")]
	Serialize(String),
	#[error("no config dir")]
	NoConfigDir,
}

pub fn user_dir() -> Result<PathBuf, Error> {
	dirs::config_dir().ok_or(Error::NoConfigDir).map(|p| p.join("kern"))
}

pub fn project_dir(cwd: &Path) -> PathBuf {
	cwd.join(".kern")
}

pub fn load<T: DeserializeOwned + Default>(path: &Path) -> Result<T, Error> {
	match std::fs::read_to_string(path) {
		Ok(text) => toml::from_str(&text).map_err(|e| Error::Parse(e.to_string())),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
		Err(e) => Err(Error::Io(e.to_string())),
	}
}

pub fn save<T: Serialize>(path: &Path, value: &T) -> Result<(), Error> {
	let text = toml::to_string_pretty(value).map_err(|e| Error::Serialize(e.to_string()))?;
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent).map_err(|e| Error::Io(e.to_string()))?;
	}
	std::fs::write(path, text).map_err(|e| Error::Io(e.to_string()))
}

pub fn load_layered<T: DeserializeOwned + Default>(user: &Path, project: &Path) -> Result<T, Error> {
	let user_v = read_value(user)?;
	let project_v = read_value(project)?;
	let merged = merge_sections(user_v, project_v);
	merged.try_into().map_err(|e: toml::de::Error| Error::Parse(e.to_string()))
}

fn read_value(path: &Path) -> Result<toml::Value, Error> {
	match std::fs::read_to_string(path) {
		// Parse as a document TABLE, not a bare `toml::Value`. A bare-value
		// parse misreads a leading `[section]` header as an array literal
		// ("unexpected content"), so any real config file starting with a
		// section would fail to load. Parsing into `toml::Table` treats the
		// input as a document.
		Ok(text) => text
			.parse::<toml::Table>()
			.map(toml::Value::Table)
			.map_err(|e| Error::Parse(e.to_string())),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml::Value::Table(toml::value::Table::new())),
		Err(e) => Err(Error::Io(e.to_string())),
	}
}

/// Shallow, **section-level** merge of `over` onto `base`: each TOP-LEVEL key in
/// `over` REPLACES the same key in `base` wholesale — there is NO recursive,
/// per-field merge. So a project `[embed]` table entirely replaces the user
/// `[embed]` table; a field the user set but the project omits is LOST, not
/// inherited. This is intentional (a project either owns a section or it does
/// not), but it surprises callers who expect a deep merge — keep a whole section
/// in one scope. Top-level keys present in only one of the two are kept as-is.
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

	/// Regression: a document whose first line is a `[section]` header must
	/// parse as a table, not be misread as an array literal. Before the fix,
	/// `read_value` used `parse::<toml::Value>()` which failed on any real
	/// config file (every project `.kern/*.toml`), silently disabling
	/// project-scope config.
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
		let user = dir.join("user.toml"); // intentionally absent
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
		// Section-level (not deep) override: the user sets two keys in [embed];
		// the project overrides the section with ONE key. The project key wins AND
		// the user's other key is dropped (the whole section is replaced).
		let dir = std::env::temp_dir().join(format!("cfgio_ovr_{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let user = dir.join("user.toml");
		let project = dir.join("project.toml");
		std::fs::write(&user, "[embed]\nurl = \"user-url\"\nkey = \"secret\"\n").unwrap();
		std::fs::write(&project, "[embed]\nurl = \"proj-url\"\n").unwrap();

		let merged: toml::Table = load_layered(&user, &project).expect("load_layered");
		let embed = merged.get("embed").and_then(|v| v.as_table()).expect("embed table");
		assert_eq!(embed.get("url").and_then(|v| v.as_str()), Some("proj-url"), "project section wins");
		assert!(embed.get("key").is_none(), "user `key` is NOT inherited — section wholly replaced");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn load_layered_keeps_sections_present_in_only_one_scope() {
		// A section that exists only in user survives (the project does not replace
		// what it omits); a project-only section is added alongside.
		let dir = std::env::temp_dir().join(format!("cfgio_keep_{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let user = dir.join("user.toml");
		let project = dir.join("project.toml");
		std::fs::write(&user, "[reason]\nmodel = \"qwen\"\n").unwrap();
		std::fs::write(&project, "[embed]\nurl = \"p\"\n").unwrap();

		let merged: toml::Table = load_layered(&user, &project).expect("load");
		assert_eq!(
			merged.get("reason").and_then(|s| s.get("model")).and_then(|v| v.as_str()),
			Some("qwen"),
			"user-only [reason] survives",
		);
		assert!(merged.get("embed").is_some(), "project-only [embed] is present too");
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn save_then_load_round_trips_and_creates_parent_dirs() {
		#[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
		struct Demo {
			name: String,
			count: u32,
			on: bool,
		}

		let dir = std::env::temp_dir().join(format!("cfgio_rt_{}", std::process::id()));
		// Nested path that does not exist yet — save() must create the parents.
		let p = dir.join("nested").join("demo.toml");
		let original = Demo { name: "kern".into(), count: 7, on: true };
		save(&p, &original).expect("save");
		assert!(p.exists(), "save created the file (and its parent dirs)");

		let back: Demo = load(&p).expect("load");
		assert_eq!(back, original, "write-then-read preserves every field");
		let _ = std::fs::remove_dir_all(&dir);
	}
}
