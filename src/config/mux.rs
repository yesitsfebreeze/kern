use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};

/// Configuration for `kern mux` — the default PTY-multiplexer TUI mode.
///
/// # Serialization note
/// `MuxConfig` is TOML-only (loaded via `config_io::load_layered`).
/// **Never** add `#[derive(bincode::Encode, bincode::Decode)]` here — doing so
/// and serializing `Config` would corrupt every shard written by the old layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MuxConfig {
    /// Command spawned for each agent pane. Defaults to `claude`.
    pub agent_cmd: String,
    /// Key binding to spawn a new sub-pane. Default `alt+n`.
    pub key_new_pane: String,
    /// Key binding to close the focused sub-pane. Default `ctrl+w`.
    pub key_close_pane: String,
    /// Key binding to cycle focus between panes. Default `tab`.
    pub key_cycle: String,
    /// Key binding to quit mux (kills all panes). Default `ctrl+q`.
    pub key_quit: String,
    /// Key binding to toggle the kern research panel. Default `ctrl+l`.
    pub key_research: String,
    /// Key binding to toggle the waiting-questions overlay. Default `ctrl+k`.
    pub key_questions: String,
}

impl Default for MuxConfig {
    fn default() -> Self {
        Self {
            agent_cmd:      "claude".into(),
            key_new_pane:   "alt+n".into(),
            key_close_pane: "ctrl+w".into(),
            key_cycle:      "tab".into(),
            key_quit:       "ctrl+q".into(),
            key_research:   "ctrl+l".into(),
            key_questions:  "ctrl+k".into(),
        }
    }
}

/// Parsed keybindings built once at startup from [`MuxConfig`].
pub struct KeyMap {
    pub new_pane:   KeyEvent,
    pub close_pane: KeyEvent,
    pub cycle:      KeyEvent,
    pub quit:       KeyEvent,
    pub research:   KeyEvent,
    pub questions:  KeyEvent,
}

impl KeyMap {
    pub fn from_config(cfg: &MuxConfig) -> Self {
        Self {
            new_pane:   parse_key_event(&cfg.key_new_pane)
                .unwrap_or_else(|| KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT)),
            close_pane: parse_key_event(&cfg.key_close_pane)
                .unwrap_or_else(|| KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)),
            cycle:      parse_key_event(&cfg.key_cycle)
                .unwrap_or_else(|| KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            quit:       parse_key_event(&cfg.key_quit)
                .unwrap_or_else(|| KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            research:   parse_key_event(&cfg.key_research)
                .unwrap_or_else(|| KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL)),
            questions:  parse_key_event(&cfg.key_questions)
                .unwrap_or_else(|| KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL)),
        }
    }

    pub fn matches_new_pane(&self, ev: &KeyEvent) -> bool   { self.new_pane == *ev }
    pub fn matches_close_pane(&self, ev: &KeyEvent) -> bool { self.close_pane == *ev }
    pub fn matches_cycle(&self, ev: &KeyEvent) -> bool      { self.cycle == *ev }
    pub fn matches_quit(&self, ev: &KeyEvent) -> bool       { self.quit == *ev }
    pub fn matches_research(&self, ev: &KeyEvent) -> bool   { self.research == *ev }
    pub fn matches_questions(&self, ev: &KeyEvent) -> bool  { self.questions == *ev }
}

/// Parse a key-binding string like `"alt+n"`, `"ctrl+w"`, or `"tab"` into a
/// crossterm [`KeyEvent`]. Returns `None` for unrecognised modifiers or keys.
pub fn parse_key_event(s: &str) -> Option<KeyEvent> {
    let s = s.to_lowercase();
    let parts: Vec<&str> = s.split('+').collect();
    let (mod_parts, key_part) = parts.split_at(parts.len().saturating_sub(1));
    let key_str = key_part.first().copied().unwrap_or("");

    let mut mods = KeyModifiers::NONE;
    for m in mod_parts {
        match *m {
            "ctrl"  => mods |= KeyModifiers::CONTROL,
            "alt"   => mods |= KeyModifiers::ALT,
            "shift" => mods |= KeyModifiers::SHIFT,
            _       => return None,
        }
    }

    let code = match key_str {
        "tab"       => KeyCode::Tab,
        "enter"     => KeyCode::Enter,
        "esc"       => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        "delete"    => KeyCode::Delete,
        "up"        => KeyCode::Up,
        "down"      => KeyCode::Down,
        "left"      => KeyCode::Left,
        "right"     => KeyCode::Right,
        "home"      => KeyCode::Home,
        "end"       => KeyCode::End,
        c if c.chars().count() == 1 => KeyCode::Char(c.chars().next()?),
        _ => return None,
    };

    Some(KeyEvent::new(code, mods))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mux_config_defaults_are_correct() {
        let c = MuxConfig::default();
        assert_eq!(c.agent_cmd, "claude");
        assert_eq!(c.key_new_pane, "alt+n");
        assert_eq!(c.key_close_pane, "ctrl+w");
        assert_eq!(c.key_cycle, "tab");
        assert_eq!(c.key_quit, "ctrl+q");
    }

    #[test]
    fn parse_key_event_tab() {
        let ev = parse_key_event("tab").unwrap();
        assert_eq!(ev.code, KeyCode::Tab);
        assert_eq!(ev.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn parse_key_event_alt_n() {
        let ev = parse_key_event("alt+n").unwrap();
        assert_eq!(ev.code, KeyCode::Char('n'));
        assert!(ev.modifiers.contains(KeyModifiers::ALT));
    }

    #[test]
    fn parse_key_event_ctrl_w() {
        let ev = parse_key_event("ctrl+w").unwrap();
        assert_eq!(ev.code, KeyCode::Char('w'));
        assert!(ev.modifiers.contains(KeyModifiers::CONTROL));
    }

    #[test]
    fn parse_key_event_unknown_returns_none() {
        assert!(parse_key_event("meta+z").is_none());
    }

    #[test]
    fn mux_config_key_research_default() {
        let c = MuxConfig::default();
        assert_eq!(c.key_research, "ctrl+l");
    }

    #[test]
    fn mux_config_key_questions_default() {
        assert_eq!(MuxConfig::default().key_questions, "ctrl+k");
    }

    #[test]
    fn keymap_matches_questions_ctrl_k() {
        let km = KeyMap::from_config(&MuxConfig::default());
        assert!(km.matches_questions(&KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn keymap_matches_research_ctrl_l() {
        let km = KeyMap::from_config(&MuxConfig::default());
        assert!(km.matches_research(&KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn keymap_matches_research_false_for_other_key() {
        let km = KeyMap::from_config(&MuxConfig::default());
        assert!(!km.matches_research(&KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn keymap_matches_defaults() {
        let km = KeyMap::from_config(&MuxConfig::default());
        assert!(km.matches_cycle(&KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert!(km.matches_new_pane(&KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT)));
        assert!(km.matches_close_pane(&KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)));
        assert!(km.matches_quit(&KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)));
    }
}
