//! Shared fixtures and frame-capture helpers for the render tests. Both the
//! frame tests in `crate::render` and the candidate-list tests in
//! `crate::render_list` import from here, so every helper exists exactly
//! once. Test-only module: it is compiled under `cfg(test)` alone.

use ratatui::backend::TestBackend;
use ratatui::style::Color;
use ratatui::Terminal;

use crate::history::record;
use crate::render::draw;
use crate::state::{self, App};
use crate::storage::Store;
use crate::theme::{CURSOR, SELECT_BG};

/// App with one recorded history entry (`br docs`) and the query `br`:
/// the frame must show the history and alias rows plus the key hints.
pub(crate) fn app_with_history() -> App {
    let mut store = Store::default();
    record(&mut store, "br", "docs", 1);
    let mut app = state::new(store, false);
    app.input = "br".to_string();
    app
}

/// App with an empty input and `n` recorded history entries named
/// `cmd-00` .. `cmd-<n-1>` (older index = newer entry).
pub(crate) fn app_with_recent_history(n: usize) -> App {
    let mut store = Store::default();
    for i in 0..n {
        record(&mut store, "br", &format!("cmd-{i:02}"), i as u64 + 1);
    }
    state::new(store, false)
}

pub(crate) fn frame_text(terminal: &Terminal<TestBackend>) -> String {
    let buf = terminal.backend().buffer();
    let mut s = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            s.push_str(buf[(x, y)].symbol());
        }
        s.push('\n');
    }
    s
}

pub(crate) fn draw_on(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| draw(f, app)).unwrap();
    frame_text(&terminal)
}

pub(crate) fn draw_once(app: &App) -> String {
    draw_on(app, 80, 14)
}

/// Row index of the first line containing `needle`.
pub(crate) fn row_of(text: &str, needle: &str) -> Option<usize> {
    text.lines().position(|l| l.contains(needle))
}

/// Position of one cell in the frame buffer.
pub(crate) type CellPos = (u16, u16, Color);

/// Cells painting an explicit background. Only the cursor cell and the
/// palette's selected row may; every other cell stays at the terminal
/// default painted by the host profile, and a stray ink panics instead of
/// slipping into the frame.
pub(crate) fn bg_cells(app: &App, width: u16, height: u16) -> Vec<CellPos> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| draw(f, app)).unwrap();
    let buf = terminal.backend().buffer();
    let mut painted = Vec::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            match buf[(x, y)].style().bg {
                // `Clear` writes an explicit Reset - still terminal default.
                None | Some(Color::Reset) => {}
                Some(c) if c == CURSOR || c == SELECT_BG => painted.push((x, y, c)),
                Some(other) => panic!("unexpected painted background {other:?} at ({x},{y})"),
            }
        }
    }
    painted
}

/// Distinct frame rows carrying at least one cell with that background.
pub(crate) fn bg_rows(painted: &[CellPos], color: Color) -> std::collections::BTreeSet<u16> {
    painted
        .iter()
        .filter(|(_, _, c)| *c == color)
        .map(|(_, y, _)| *y)
        .collect()
}

/// Rows of the drawn candidate list (every history row starts with a
/// record mark).
pub(crate) fn history_rows(text: &str) -> usize {
    text.lines().filter(|l| l.contains("↻ ")).count()
}
