use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Default)]
pub struct State {
	pub project_root: PathBuf,
	pub active_recipe: Option<String>,
	pub current_model: Option<String>,
	pub current_provider: Option<String>,
	pub journal_day: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct StateHandle(Arc<RwLock<State>>);

impl std::fmt::Display for State {
	/// Render the state as an aligned key/value block (no trailing newline). The
	/// authoritative formatter — `StateHandle::render` just snapshots and calls it.
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		writeln!(f, "state:")?;
		writeln!(f, "  project        {}", self.project_root.display())?;
		writeln!(f, "  active recipe  {}", self.active_recipe.as_deref().unwrap_or("(none)"))?;
		let model = match (self.current_provider.as_deref(), self.current_model.as_deref()) {
			(Some(p), Some(m)) => format!("{p} / {m}"),
			_ => "(unbound)".into(),
		};
		writeln!(f, "  model          {model}")?;
		// Last line: no trailing newline (write!, not writeln!).
		write!(f, "  journal day    {}", self.journal_day.as_deref().unwrap_or("(not opened)"))
	}
}

impl StateHandle {
	pub fn new(initial: State) -> Self {
		Self(Arc::new(RwLock::new(initial)))
	}

	pub fn snapshot(&self) -> State {
		self.0.read().expect("state lock poisoned").clone()
	}

	pub fn update(&self, f: impl FnOnce(&mut State)) {
		let mut g = self.0.write().expect("state lock poisoned");
		f(&mut g);
	}

	pub fn render(&self) -> String {
		self.snapshot().to_string()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn render_shows_defaults_for_unset_fields() {
		let out = State::default().to_string();
		assert!(out.starts_with("state:\n"), "header line present");
		assert!(out.contains("active recipe  (none)"));
		assert!(out.contains("model          (unbound)"));
		assert!(out.contains("journal day    (not opened)"));
		assert!(!out.ends_with('\n'), "no trailing newline");
	}

	#[test]
	fn render_shows_provider_slash_model_when_both_set() {
		let s = State {
			current_provider: Some("ollama".into()),
			current_model: Some("qwen3".into()),
			active_recipe: Some("rust".into()),
			journal_day: Some("2026-06-10".into()),
			..Default::default()
		};
		let out = s.to_string();
		assert!(out.contains("active recipe  rust"));
		assert!(out.contains("model          ollama / qwen3"));
		assert!(out.contains("journal day    2026-06-10"));
	}

	#[test]
	fn model_is_unbound_unless_both_provider_and_model_are_set() {
		let only_model = State { current_model: Some("m".into()), ..Default::default() };
		assert!(only_model.to_string().contains("model          (unbound)"));
		let only_provider = State { current_provider: Some("p".into()), ..Default::default() };
		assert!(only_provider.to_string().contains("model          (unbound)"));
	}

	#[test]
	fn handle_render_matches_snapshot_display() {
		let h = StateHandle::new(State { active_recipe: Some("r".into()), ..Default::default() });
		assert_eq!(h.render(), h.snapshot().to_string());
	}
}
