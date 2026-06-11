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
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor},
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

// ── Cell style helpers ────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Eq)]
struct CellStyle {
    fg:        vt100::Color,
    bg:        vt100::Color,
    bold:      bool,
    italic:    bool,
    underline: bool,
    inverse:   bool,
}

impl Default for CellStyle {
    fn default() -> Self {
        Self {
            fg: vt100::Color::Default,
            bg: vt100::Color::Default,
            bold: false, italic: false, underline: false, inverse: false,
        }
    }
}

fn cell_style(cell: &vt100::Cell) -> CellStyle {
    CellStyle {
        fg:        cell.fgcolor(),
        bg:        cell.bgcolor(),
        bold:      cell.bold(),
        italic:    cell.italic(),
        underline: cell.underline(),
        inverse:   cell.inverse(),
    }
}

fn vt100_color_to_crossterm(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default    => Color::Reset,
        vt100::Color::Idx(n)     => Color::AnsiValue(n),
        vt100::Color::Rgb(r,g,b) => Color::Rgb { r, g, b },
    }
}

fn apply_style(stdout: &mut impl Write, style: &CellStyle) -> io::Result<()> {
    queue!(stdout, SetAttribute(Attribute::Reset))?;
    let fg = vt100_color_to_crossterm(style.fg);
    if fg != Color::Reset {
        queue!(stdout, SetForegroundColor(fg))?;
    }
    let bg = vt100_color_to_crossterm(style.bg);
    if bg != Color::Reset {
        queue!(stdout, SetBackgroundColor(bg))?;
    }
    if style.bold      { queue!(stdout, SetAttribute(Attribute::Bold))?;      }
    if style.italic    { queue!(stdout, SetAttribute(Attribute::Italic))?;    }
    if style.underline { queue!(stdout, SetAttribute(Attribute::Underlined))?;}
    if style.inverse   { queue!(stdout, SetAttribute(Attribute::Reverse))?;   }
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
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "?".to_string());
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
            draw_frame(&reg, &mut stdout, cols, rows, &cwd)?;
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

/// Render one frame: top status bar + two-column pane split.
pub fn draw_frame(
    registry: &PaneRegistry,
    stdout: &mut impl Write,
    cols: u16,
    rows: u16,
    cwd: &str,
) -> io::Result<()> {
    let pane_rows  = rows.saturating_sub(1);
    let left_cols  = cols / 2;
    let right_cols = cols - left_cols;
    let row_offset: u16 = 1;   // row 0 = status bar

    if let Some(main) = registry.panes.get(0) {
        draw_pane(stdout, main.parser.screen(), 0, left_cols, pane_rows, row_offset)?;
    }

    for row in 0..pane_rows {
        queue!(stdout, MoveTo(left_cols, row + row_offset), Print("│"))?;
    }

    if registry.focus > 0 {
        if let Some(sub) = registry.focused() {
            draw_pane(
                stdout,
                sub.parser.screen(),
                left_cols + 1,
                right_cols.saturating_sub(1),
                pane_rows,
                row_offset,
            )?;
        }
    }

    // ── Top status bar ────────────────────────────────────────────────────────
    let labels: Vec<(String, String, bool)> = registry
        .panes
        .iter()
        .map(|p| (p.id.clone(), p.label.clone(), p.exited))
        .collect();
    let left_text  = format_status_left(cwd, registry.thoughts, registry.reasons);
    let right_text = format_status_right(&labels, registry.focus);
    let left_w     = left_text.chars().count();
    let right_w    = right_text.chars().count();
    let total      = cols as usize;
    let mid_w      = total.saturating_sub(left_w).saturating_sub(right_w);

    // Left: inversed
    queue!(
        stdout,
        MoveTo(0, 0),
        SetAttribute(Attribute::Reverse),
        Print(&left_text),
        SetAttribute(Attribute::Reset),
        ResetColor,
        Print(" ".repeat(mid_w)),
    )?;
    // Right: dark-grey tabs
    if right_w > 0 {
        queue!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print(&right_text),
            ResetColor,
        )?;
    }

    // ── Cursor: position at focused pane's vt100 cursor, then show ───────────
    let (focused_screen, pane_col_offset) = if registry.focus > 0 {
        (registry.focused().map(|p| p.parser.screen()), left_cols + 1)
    } else {
        (registry.panes.get(0).map(|p| p.parser.screen()), 0u16)
    };
    if let Some(screen) = focused_screen {
        let (crow, ccol) = screen.cursor_position();
        queue!(stdout, MoveTo(pane_col_offset + ccol, crow + 1), Show)?;
    }

    Ok(())
}

/// Render a `vt100::Screen` into a rectangular terminal region starting at `row_offset`.
fn draw_pane(
    stdout: &mut impl Write,
    screen: &vt100::Screen,
    col_offset: u16,
    width: u16,
    height: u16,
    row_offset: u16,
) -> io::Result<()> {
    let (screen_rows, screen_cols) = screen.size();
    let mut cur = CellStyle::default();
    let mut buf = String::with_capacity(width as usize);

    for row in 0..height {
        queue!(stdout, MoveTo(col_offset, row + row_offset))?;

        for col in 0..width {
            let (content, style) = if col < screen_cols && row < screen_rows {
                if let Some(cell) = screen.cell(row, col) {
                    let s = cell_style(cell);
                    let c = cell.contents();
                    (if c.is_empty() { " ".to_string() } else { c }, s)
                } else {
                    (" ".to_string(), CellStyle::default())
                }
            } else {
                (" ".to_string(), CellStyle::default())
            };

            if style != cur {
                if !buf.is_empty() {
                    queue!(stdout, Print(&buf))?;
                    buf.clear();
                }
                apply_style(stdout, &style)?;
                cur = style;
            }
            buf.push_str(&content);
        }

        if !buf.is_empty() {
            queue!(stdout, Print(&buf))?;
            buf.clear();
        }
    }

    // Reset style after pane to avoid bleed into adjacent regions
    queue!(stdout, SetAttribute(Attribute::Reset), ResetColor)?;
    Ok(())
}

/// Left section of the top status bar.
pub fn format_status_left(cwd: &str, thoughts: u32, reasons: u32) -> String {
    format!(" kern | {} | {} Thoughts | {} Reasons ", cwd, thoughts, reasons)
}

/// Right section of the top status bar: pane tab list.
pub fn format_status_right(
    pane_list: &[(String, String, bool)],
    focus: usize,
) -> String {
    let mut s = String::new();
    for (i, (_id, label, exited)) in pane_list.iter().enumerate() {
        let marker = if i == focus { "●" } else { " " };
        if *exited {
            s.push_str(&format!(" {marker}†{label} "));
        } else {
            s.push_str(&format!(" {marker}{label} "));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_status_right_marks_active_pane() {
        let labels = vec![
            ("id1".to_string(), "main".to_string(),  false),
            ("id2".to_string(), "sub-1".to_string(), false),
        ];
        let right = format_status_right(&labels, 0);
        assert!(right.contains("●main"),   "active pane bullet: {right:?}");
        assert!(right.contains("sub-1"),   "inactive pane present: {right:?}");
        assert!(!right.contains("●sub-1"), "inactive has no bullet: {right:?}");
    }

    #[test]
    fn format_status_right_shows_exited_as_dead() {
        let labels = vec![
            ("id1".to_string(), "main".to_string(), false),
            ("id2".to_string(), "dead".to_string(), true),
        ];
        let right = format_status_right(&labels, 0);
        assert!(right.contains("†dead"), "exited pane shown with dagger: {right:?}");
    }

    #[test]
    fn format_status_left_contains_kern_and_cwd() {
        let left = format_status_left("mydir", 3, 42);
        assert!(left.contains("kern"),        "contains app name: {left:?}");
        assert!(left.contains("mydir"),       "contains cwd: {left:?}");
        assert!(left.contains("3 Thoughts"),  "contains thoughts: {left:?}");
        assert!(left.contains("42 Reasons"),  "contains reasons: {left:?}");
    }
}
