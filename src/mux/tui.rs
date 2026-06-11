//! Crossterm TUI render loop for the mux PTY multiplexer.
//!
//! Entry point: `run_tui(registry, keymap)`. Runs until the user presses
//! the quit key or all panes exit. The terminal is restored on normal exit
//! and on panic via `Guard`.

use std::collections::HashMap;
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

// ── Dirty-rect snapshot types ─────────────────────────────────────────────────

#[derive(Clone, PartialEq, Eq)]
struct RenderedCell {
    content: String,
    style:   CellStyle,
}

pub(crate) struct ScreenSnapshot {
    rows:  u16,
    cols:  u16,
    cells: Vec<RenderedCell>,
}

impl ScreenSnapshot {
    /// Create a snapshot filled with sentinel cells that never match real output,
    /// forcing a full redraw on the first frame.
    fn new(rows: u16, cols: u16) -> Self {
        let len = (rows as usize) * (cols as usize).max(1);
        Self {
            rows,
            cols,
            cells: vec![RenderedCell { content: "\x00".to_string(), style: CellStyle::default() }; len],
        }
    }
    fn get(&self, row: u16, col: u16) -> &RenderedCell {
        &self.cells[row as usize * self.cols as usize + col as usize]
    }
    fn set(&mut self, row: u16, col: u16, cell: RenderedCell) {
        self.cells[row as usize * self.cols as usize + col as usize] = cell;
    }
}

// ── Main TUI loop ─────────────────────────────────────────────────────────────

/// Run the mux TUI until the user quits or all panes exit.
///
/// Acquires `registry` only for brief drain/draw/key operations, releasing it
/// between frames so MCP worker threads can call `mux_*` tools concurrently.
pub fn run_tui(registry: &SharedRegistry, keymap: &KeyMap) -> io::Result<()> {
    install_panic_hook();
    let _guard = Guard::enter()?;
    // Full absolute path — the status bar front-truncates it to fit, so longer
    // is better (it shows as much of the tail as the bar width allows).
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "?".to_string());
    let (mut cols, mut rows) = terminal::size()?;

    // BufWriter coalesces the thousands of per-cell `queue!` writes into a
    // single syscall per frame flush.  Without this, every style-change command
    // is an individual Win32 console API call — catastrophic on a styled TTY.
    let mut stdout = io::BufWriter::with_capacity(1 << 16, io::stdout());
    queue!(stdout, Clear(ClearType::All))?;
    stdout.flush()?;

    let mut snapshots: HashMap<String, ScreenSnapshot> = HashMap::new();
    let mut research: Option<crate::mux::ResearchPanel> = None;

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

        // Draw: research panel takes over when open; otherwise normal pane draw.
        if let Some(ref mut panel) = research {
            panel.tick();
            {
                let reg = registry.lock().unwrap_or_else(|p| p.into_inner());
                draw_status_bar(&reg, &mut stdout, cols, &cwd)?;
            }
            panel.draw(&mut stdout, cols, rows)?;
        } else {
            let reg = registry.lock().unwrap_or_else(|p| p.into_inner());
            draw_frame(&reg, &mut stdout, cols, rows, &cwd, &mut snapshots)?;
        }
        stdout.flush()?;

        // Poll for input — lock is NOT held here, giving MCP threads ~16ms per frame.
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Resize(w, h) => {
                    cols = w;
                    rows = h;
                    if research.is_none() {
                        let mut reg = registry.lock().unwrap_or_else(|p| p.into_inner());
                        reg.resize_all(cols, rows.saturating_sub(1));
                    }
                    queue!(stdout, Clear(ClearType::All))?;
                    snapshots.clear();
                }
                Event::Key(kev) if kev.kind == KeyEventKind::Press => {
                    if keymap.matches_quit(&kev) {
                        break;
                    } else if keymap.matches_research(&kev) {
                        // Toggle the research panel (no registry lock needed).
                        match research.take() {
                            Some(_) => {}
                            None => {
                                let mut panel = crate::mux::ResearchPanel::new();
                                panel.session.on_panel_open();
                                research = Some(panel);
                            }
                        }
                        queue!(stdout, Clear(ClearType::All))?;
                        snapshots.clear();
                    } else if let Some(ref mut panel) = research {
                        // Delegate all key events to the research panel.
                        let close = panel.handle_key(&kev);
                        if close {
                            research = None;
                            queue!(stdout, Clear(ClearType::All))?;
                            snapshots.clear();
                        }
                    } else {
                        // Normal pane key routing.
                        let mut reg = registry.lock().unwrap_or_else(|p| p.into_inner());
                        if keymap.matches_cycle(&kev) {
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
                }
                _ => {}
            }
        }
    }

    Ok(())
}

// ── Rendering ─────────────────────────────────────────────────────────────────

/// Render one frame: top status bar + two-column pane split.
pub(crate) fn draw_frame(
    registry: &PaneRegistry,
    stdout: &mut impl Write,
    cols: u16,
    rows: u16,
    cwd: &str,
    snapshots: &mut HashMap<String, ScreenSnapshot>,
) -> io::Result<()> {
    let pane_rows  = rows.saturating_sub(1);
    let left_cols  = cols / 2;
    let right_cols = cols - left_cols;
    let row_offset: u16 = 1;   // row 0 = status bar

    // Fake cursor: only the focused pane gets a cursor_pos; others get None.
    let main_cursor = if registry.focus == 0 {
        registry.panes.get(0).map(|p| p.parser.screen().cursor_position())
    } else {
        None
    };
    let sub_cursor = if registry.focus > 0 {
        registry.focused().map(|p| p.parser.screen().cursor_position())
    } else {
        None
    };

    if let Some(main) = registry.panes.get(0) {
        let snap = snapshots.entry(main.id.clone()).or_insert_with(|| ScreenSnapshot::new(0, 0));
        draw_pane_diff(stdout, main.parser.screen(), snap, 0, left_cols, pane_rows, row_offset, main_cursor)?;
    }

    for row in 0..pane_rows {
        queue!(stdout, MoveTo(left_cols, row + row_offset), Print("│"))?;
    }

    if registry.focus > 0 {
        if let Some(sub) = registry.focused() {
            let snap = snapshots.entry(sub.id.clone()).or_insert_with(|| ScreenSnapshot::new(0, 0));
            draw_pane_diff(
                stdout,
                sub.parser.screen(),
                snap,
                left_cols + 1,
                right_cols.saturating_sub(1),
                pane_rows,
                row_offset,
                sub_cursor,
            )?;
        }
    }

    // ── Top status bar ────────────────────────────────────────────────────────
    draw_status_bar(registry, stdout, cols, cwd)?;

    Ok(())
}

/// Render the top status bar row (row 0) only.
///
/// Extracted so the research panel can draw the bar without triggering a full
/// pane redraw.
pub(crate) fn draw_status_bar(
    registry: &PaneRegistry,
    stdout: &mut impl Write,
    cols: u16,
    cwd: &str,
) -> io::Result<()> {
    let labels: Vec<(String, String, bool)> = registry
        .panes
        .iter()
        .map(|p| (p.id.clone(), p.label.clone(), p.exited))
        .collect();
    let right_text = format_status_right(&labels, registry.focus);
    let right_w    = right_text.chars().count();
    let total      = cols as usize;
    let chrome_w   = format_status_left("", registry.thoughts, registry.reasons)
        .chars()
        .count();
    let cwd_budget = total.saturating_sub(right_w).saturating_sub(chrome_w);
    let cwd_shown  = truncate_front(cwd, cwd_budget);
    let left_text  = format_status_left(&cwd_shown, registry.thoughts, registry.reasons);
    let left_w     = left_text.chars().count();
    let mid_w      = total.saturating_sub(left_w).saturating_sub(right_w);

    queue!(
        stdout,
        MoveTo(0, 0),
        SetAttribute(Attribute::Reverse),
        Print(&left_text),
        SetAttribute(Attribute::Reset),
        ResetColor,
        Print(" ".repeat(mid_w)),
    )?;
    if right_w > 0 {
        queue!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print(&right_text),
            ResetColor,
        )?;
    }
    Ok(())
}

/// Render a `vt100::Screen` into a rectangular terminal region using dirty-rect
/// diffing: only cells whose content or style changed since the last frame are
/// emitted. Unchanged cells are skipped entirely.
///
/// `cursor_pos` — if `Some((row, col))`, that cell is rendered with its inverse
/// bit toggled, producing a fake block cursor without moving the real terminal
/// cursor. Pass `None` for unfocused panes; the real cursor stays hidden the
/// whole time.
fn draw_pane_diff(
    stdout: &mut impl Write,
    screen: &vt100::Screen,
    snapshot: &mut ScreenSnapshot,
    col_offset: u16,
    width: u16,
    height: u16,
    row_offset: u16,
    cursor_pos: Option<(u16, u16)>,
) -> io::Result<()> {
    // If dimensions changed, reset snapshot (forces full redraw this frame).
    if snapshot.rows != height || snapshot.cols != width {
        *snapshot = ScreenSnapshot::new(height, width);
    }

    let (screen_rows, screen_cols) = screen.size();
    let mut cur = CellStyle::default();      // last applied terminal style
    let mut buf = String::with_capacity(32); // pending text batch
    let mut write_pos: Option<(u16, u16)> = None; // (row, col) terminal write-head

    for row in 0..height {
        for col in 0..width {
            // 1. Compute new cell (content + style), same logic as current draw_pane.
            let (content, mut style) = if col < screen_cols && row < screen_rows {
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

            // Fake cursor: invert the cursor cell's style so it renders as a
            // block cursor. The real terminal cursor stays hidden the whole time.
            if cursor_pos == Some((row, col)) {
                style.inverse = !style.inverse;
            }

            // 2. Compare to snapshot — if identical, skip.
            let snap = snapshot.get(row, col);
            if snap.content == content && snap.style == style {
                // Flush pending buf, reset write_pos (break the write run).
                if !buf.is_empty() {
                    queue!(stdout, Print(&buf))?;
                    buf.clear();
                }
                write_pos = None;
                continue;
            }

            // 3. Cell changed — needs rendering.
            //    Move to cell position if write-head isn't already there.
            if write_pos != Some((row, col)) {
                if !buf.is_empty() {
                    queue!(stdout, Print(&buf))?;
                    buf.clear();
                }
                queue!(stdout, MoveTo(col_offset + col, row + row_offset))?;
                write_pos = Some((row, col));
            }

            // 4. Apply style if changed (flush buf first).
            if style != cur {
                if !buf.is_empty() {
                    queue!(stdout, Print(&buf))?;
                    write_pos = write_pos.map(|(r, c)| (r, c + buf.chars().count() as u16));
                    buf.clear();
                }
                apply_style(stdout, &style)?;
                cur = style.clone();
            }

            // 5. Buffer the content, update snapshot.
            buf.push_str(&content);
            snapshot.set(row, col, RenderedCell { content, style });
        }

        // Flush at end of row.
        if !buf.is_empty() {
            queue!(stdout, Print(&buf))?;
            buf.clear();
        }
        write_pos = None; // New row always needs MoveTo.
    }

    // Reset style after pane to avoid bleed into adjacent regions.
    queue!(stdout, SetAttribute(Attribute::Reset), ResetColor)?;
    Ok(())
}

/// Truncate `s` to at most `max` chars, keeping the END of the string and
/// prefixing an ellipsis when characters are dropped (e.g. a long path renders
/// as `…/deep/tail/dir`). Char-safe: never splits a multi-byte codepoint.
pub fn truncate_front(s: &str, max: usize) -> String {
    let len = s.chars().count();
    if len <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    if max == 1 {
        return "…".to_string();
    }
    // Keep the last (max - 1) chars; the ellipsis fills the remaining slot.
    let tail: String = s.chars().skip(len - (max - 1)).collect();
    format!("…{tail}")
}

/// Left section of the top status bar.
pub fn format_status_left(cwd: &str, thoughts: u32, reasons: u32) -> String {
    format!(" kern | {}/{} | {} ", thoughts, reasons, cwd)
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
        assert!(left.contains("kern"),  "contains app name: {left:?}");
        assert!(left.contains("mydir"), "contains cwd: {left:?}");
        assert!(left.contains("3/42"),  "contains thoughts/reasons counts: {left:?}");
        // Counts come BEFORE the cwd.
        assert!(
            left.find("3/42").unwrap() < left.find("mydir").unwrap(),
            "counts precede cwd: {left:?}"
        );
    }

    #[test]
    fn truncate_front_keeps_tail_with_ellipsis() {
        // Fits: returned unchanged.
        assert_eq!(truncate_front("short", 10), "short");
        assert_eq!(truncate_front("exact", 5), "exact");

        // Too long: ellipsis prefix, exact char budget, path tail preserved.
        let out = truncate_front("/home/sayhe/dev/relay/kern", 12);
        assert_eq!(out.chars().count(), 12, "respects budget: {out:?}");
        assert!(out.starts_with('…'),       "ellipsis prefix: {out:?}");
        assert!(out.ends_with("relay/kern"), "keeps tail: {out:?}");

        // Degenerate budgets.
        assert_eq!(truncate_front("anything", 0), "");
        assert_eq!(truncate_front("anything", 1), "…");
    }
}
