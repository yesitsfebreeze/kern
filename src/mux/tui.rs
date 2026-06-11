//! Crossterm TUI render loop for the mux PTY multiplexer.
//!
//! Entry point: `run_tui(registry, keymap)`. Runs until the user presses
//! the quit key or all panes exit. The terminal is restored on normal exit
//! and on panic via `Guard`.

use std::io::{self, Write};
use std::time::Duration;

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyEventKind},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};

use crate::config::KeyMap;
use crate::mux::pty::key_event_to_bytes;
use crate::mux::registry::{PaneRegistry, SharedRegistry};

// ── Terminal guard ────────────────────────────────────────────────────────────

/// Enters raw mode + alt-screen on construction; restores on `Drop`.
pub struct Guard;

impl Guard {
    pub fn enter() -> io::Result<Self> {
        #[cfg(windows)]
        enable_vt_windows()?;
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(Guard)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

/// Install a panic hook that restores the terminal before printing the panic.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
        default(info);
    }));
}

#[cfg(windows)]
fn enable_vt_windows() -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    unsafe {
        let h = io::stdout().as_raw_handle() as isize;
        let mut mode: u32 = 0;
        if windows_sys::Win32::System::Console::GetConsoleMode(h as _, &mut mode) == 0 {
            return Err(io::Error::last_os_error());
        }
        if windows_sys::Win32::System::Console::SetConsoleMode(h as _, mode | 0x0004) == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

// ── Main TUI loop ─────────────────────────────────────────────────────────────

/// Run the mux TUI until the user quits or all panes exit.
///
/// Acquires `registry` only for brief drain/draw/key operations, releasing it
/// between frames so MCP worker threads can call `mux_*` tools concurrently.
pub fn run_tui(registry: &SharedRegistry, keymap: &KeyMap) -> io::Result<()> {
    install_panic_hook();
    let _guard = Guard::enter()?;
    let (mut cols, mut rows) = terminal::size()?;

    let mut stdout = io::stdout();
    queue!(stdout, Clear(ClearType::All))?;
    stdout.flush()?;

    loop {
        // Drain + reap: brief lock acquisition.
        {
            let mut reg = registry.lock().unwrap_or_else(|p| p.into_inner());
            reg.drain_all();
            reg.reap_exited();
            if reg.panes.is_empty() {
                break;
            }
        }

        // Draw: read-lock for frame rendering.
        {
            let reg = registry.lock().unwrap_or_else(|p| p.into_inner());
            draw_frame(&reg, &mut stdout, cols, rows)?;
        }
        stdout.flush()?;

        // Poll for input — lock is NOT held here, giving MCP threads ~16ms per frame.
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Resize(w, h) => {
                    cols = w;
                    rows = h;
                    let mut reg = registry.lock().unwrap_or_else(|p| p.into_inner());
                    reg.resize_all(cols, rows.saturating_sub(1));
                    queue!(stdout, Clear(ClearType::All))?;
                }
                Event::Key(kev) if kev.kind == KeyEventKind::Press => {
                    let mut reg = registry.lock().unwrap_or_else(|p| p.into_inner());
                    if keymap.matches_quit(&kev) {
                        break;
                    } else if keymap.matches_cycle(&kev) {
                        reg.cycle_focus();
                    } else if keymap.matches_new_pane(&kev) {
                        let n   = reg.panes.len();
                        let cmd = reg.panes[0].cmd.clone();
                        let _ = reg.spawn_pane(
                            format!("sub-{n}"),
                            cmd,
                            cols / 2,
                            rows.saturating_sub(1),
                        );
                    } else if keymap.matches_close_pane(&kev) {
                        if reg.focus > 0 {
                            if let Some(p) = reg.focused_mut() { p.kill(); }
                        }
                    } else if let Some(bytes) = key_event_to_bytes(&kev) {
                        if let Some(p) = reg.focused_mut() { p.write_input(&bytes); }
                    }
                }
                _ => {}
            }
        }
    }

    Ok(())
}

// ── Rendering ─────────────────────────────────────────────────────────────────

/// Render one frame: two-column split + one-row tab strip.
pub fn draw_frame(
    registry: &PaneRegistry,
    stdout: &mut impl Write,
    cols: u16,
    rows: u16,
) -> io::Result<()> {
    let pane_rows = rows.saturating_sub(1);
    let left_cols  = cols / 2;
    let right_cols = cols - left_cols;

    if let Some(main) = registry.panes.get(0) {
        draw_pane(stdout, main.parser.screen(), 0, left_cols, pane_rows)?;
    }

    for row in 0..pane_rows {
        queue!(stdout, MoveTo(left_cols, row), Print("│"))?;
    }

    if registry.focus > 0 {
        if let Some(sub) = registry.focused() {
            draw_pane(stdout, sub.parser.screen(), left_cols + 1, right_cols.saturating_sub(1), pane_rows)?;
        }
    }

    let labels: Vec<(String, String, bool)> = registry
        .panes
        .iter()
        .map(|p| (p.id.clone(), p.label.clone(), p.exited))
        .collect();
    let strip = format_tab_strip(&labels, registry.focus, cols as usize);
    queue!(
        stdout,
        MoveTo(0, pane_rows),
        SetForegroundColor(Color::DarkGrey),
        Print(&strip),
        ResetColor,
    )?;

    Ok(())
}

/// Render a `vt100::Screen` into a rectangular terminal region.
fn draw_pane(
    stdout: &mut impl Write,
    screen: &vt100::Screen,
    col_offset: u16,
    width: u16,
    height: u16,
) -> io::Result<()> {
    let (screen_rows, screen_cols) = screen.size();
    for row in 0..height {
        queue!(stdout, MoveTo(col_offset, row))?;
        let mut line = String::with_capacity(width as usize);
        for col in 0..width {
            if col < screen_cols && row < screen_rows {
                let c = screen.cell(row, col).map(|c| c.contents()).unwrap_or_default();
                if c.is_empty() { line.push(' '); } else { line.push_str(&c); }
            } else {
                line.push(' ');
            }
        }
        queue!(stdout, Print(&line))?;
    }
    Ok(())
}

/// Build the one-row tab strip string.
/// `pane_list` is `(session_id, label, exited)`.
pub fn format_tab_strip(
    pane_list: &[(String, String, bool)],
    focus: usize,
    total_cols: usize,
) -> String {
    let mut left = String::new();
    for (i, (_id, label, exited)) in pane_list.iter().enumerate() {
        let marker = if i == focus { "●" } else { " " };
        let display = if *exited {
            format!(" {marker}†{label} ")
        } else {
            format!(" {marker}{label} ")
        };
        left.push_str(&display);
    }
    let right = format!(" kern ");
    let padding = total_cols.saturating_sub(left.len() + right.len());
    format!("{left}{}{right}", " ".repeat(padding))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_tab_strip_marks_active_pane() {
        let labels = vec![
            ("id1".to_string(), "main".to_string(),  false),
            ("id2".to_string(), "sub-1".to_string(), false),
        ];
        let strip = format_tab_strip(&labels, 0, 60);
        assert!(strip.contains("●main"),   "active pane bullet: {strip:?}");
        assert!(strip.contains("sub-1"),   "inactive pane present: {strip:?}");
        assert!(!strip.contains("●sub-1"), "inactive has no bullet: {strip:?}");
    }

    #[test]
    fn format_tab_strip_shows_exited_as_dead() {
        let labels = vec![
            ("id1".to_string(), "main".to_string(), false),
            ("id2".to_string(), "dead".to_string(), true),
        ];
        let strip = format_tab_strip(&labels, 0, 60);
        assert!(strip.contains("†dead"), "exited pane shown with dagger: {strip:?}");
    }
}
