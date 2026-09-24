//! Frame rendering. All colours live in one place - `crate::theme` - so
//! call sites never hard-code raw `Color` values. The theme is the
//! terminator-rust kanagawa port; the shown bar is the input box, the
//! candidate list under it (or the command palette while that is open) and
//! one status/hints row, painted over the host terminal's background, so
//! only the cursor cell and the selected candidate row carry a background.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::commands;
use crate::keyspec;
use crate::render_list::draw_candidates;
use crate::settings_view;
use crate::state::{App, Mode, Visibility};
use crate::theme::{ACCENT, BORDER, CURSOR, ERR, MUTED, OK, SELECT_BG, TEXT};

/// Input box height in rows: top border, input line, bottom border.
pub(crate) const INPUT_BOX_H: u16 = 3;

/// Hidden-mode one-liner; the wake key is injected at render time so a
/// custom `--wake-key` / stored config shows the real binding.
fn hidden_hint(wake: &str) -> String {
    format!(" xconsoler hidden — {wake} wake · Ctrl+C quit ")
}

/// Key-hints row under the list, shown while there is no status message.
/// A narrow bar drops the least essential hints whole (no mid-word clipping):
/// `Ctrl+C quit` goes first, the wake/run/select trio always survives.
fn keys_hint(wake: &str, width: usize) -> String {
    let segs = [
        format!("{wake} hide"),
        "Enter run".to_string(),
        "↑↓/Tab select".to_string(),
        "Ctrl+U clear".to_string(),
        "Ctrl+C quit".to_string(),
    ];
    let mut keep = segs.len();
    while keep > 1 && hint_width(&segs[..keep]) > width {
        keep -= 1;
    }
    format!(" {} ", segs[..keep].join(HINT_SEP))
}

/// Separator between two hints.
const HINT_SEP: &str = " · ";

/// Rendered width of the joined hints: text + separators + the two spaces
/// `keys_hint` pads the line with.
fn hint_width(segs: &[String]) -> usize {
    let text: usize = segs.iter().map(|s| s.chars().count()).sum();
    let seps = HINT_SEP.chars().count() * segs.len().saturating_sub(1);
    text + seps + 2
}

/// Draw one frame: the hidden one-liner, the shown input box, or the settings
/// page. In settings mode the whole screen belongs to the settings page - the
/// bar layout is not drawn.
pub fn draw(f: &mut Frame, app: &App) {
    // Blank the frame first: switching visibility must not leave stale cells
    // behind (ratatui only diffs what is re-rendered).
    f.render_widget(Clear, f.area());
    match app.mode {
        Mode::Normal => match app.visibility {
            Visibility::Hidden => draw_hidden(f, app),
            Visibility::Shown => draw_shown(f, app),
        },
        Mode::Settings(ref st) => settings_view::draw(f, st, &app.aliases),
    }
}

fn draw_hidden(f: &mut Frame, app: &App) {
    let area = f.area();
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hidden_hint(&keyspec::describe(&app.wake)),
            Style::new().fg(MUTED),
        ))),
        clip(area, row_rect(area.width, 0, 1)),
    );
}

/// Shown mode: the input box, then the command palette while it is open
/// (take precedence over the list), otherwise the candidate list, then one
/// row carrying the status message or the key hints.
fn draw_shown(f: &mut Frame, app: &App) {
    let width = f.area().width;
    let mut y = draw_input_box(f, app, width);
    if app.palette.is_some() {
        y = draw_palette(f, app, width, y);
    } else {
        y = draw_candidates(f, app, width, y);
    }
    draw_status(f, app, width, y);
}

/// Command palette under the input box: one row per built-in `:`/`/` command,
/// Up/Down to move, Enter to accept, Esc to close. Scrolls so the selection
/// stays visible; a terminal with no room degrades to no list.
fn draw_palette(f: &mut Frame, app: &App, width: u16, y: u16) -> u16 {
    let total = commands::len();
    // The block needs 2 border rows and the status row 1.
    let free = f.area().height.saturating_sub(y).saturating_sub(1) as usize;
    let rows = total.min(free.saturating_sub(2));
    if rows == 0 {
        return y;
    }
    let sel = app.palette.unwrap_or(0).min(total - 1);
    let start = sel.saturating_sub(rows - 1).min(total - rows);
    let layout = row_rect(width, y, (rows as u16).saturating_add(2));
    let block = main_block(&format!(" commands \u{b7} {total} "));
    let inner = block.inner(layout);
    let sel_style = Style::new().bg(SELECT_BG).fg(TEXT);
    let mut lines = Vec::with_capacity(rows);
    for (i, spec) in commands::ALL.iter().enumerate().skip(start).take(rows) {
        let segs = vec![
            (format!(" {} ", spec.token), Style::new().fg(ACCENT)),
            (spec.desc.to_string(), Style::new().fg(MUTED)),
        ];
        lines.push(segments_line(
            segs,
            inner.width as usize,
            i == sel,
            sel_style,
        ));
    }
    f.render_widget(Paragraph::new(lines).block(block), clip(f.area(), layout));
    y.saturating_add(rows as u16).saturating_add(2)
}

/// The bar block: rounded border, no title.
fn input_block() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(BORDER))
}

/// Full-width input box: prompt + input + a reverse-space cursor at the end.
/// Returns the first row under the box.
fn draw_input_box(f: &mut Frame, app: &App, width: u16) -> u16 {
    let rect = clip(f.area(), row_rect(width, 0, INPUT_BOX_H));
    let block = input_block();
    let inner = block.inner(rect);
    let budget = inner.width.saturating_sub(3) as usize; // "❯ " + cursor cell
    let shown: String = app.input.chars().take(budget).collect();
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("❯ ", Style::new().fg(ACCENT)),
            Span::styled(shown, Style::new().fg(TEXT)),
            Span::styled(" ", Style::new().bg(CURSOR).fg(CURSOR)),
        ]))
        .block(block),
        rect,
    );
    INPUT_BOX_H
}

/// One line under the list (or box): the app's transient status message
/// (ok / error mark with the message), else the muted key hints.
fn draw_status(f: &mut Frame, app: &App, width: u16, y: u16) {
    let line = match &app.status {
        Some((ok, msg)) => {
            let (mark, color) = if *ok { ("✓ ", OK) } else { ("✗ ", ERR) };
            Line::from(vec![
                Span::styled(mark, Style::new().fg(color)),
                Span::styled(msg.clone(), Style::new().fg(color)),
            ])
        }
        None => Line::from(Span::styled(
            keys_hint(&keyspec::describe(&app.wake), width as usize),
            Style::new().fg(MUTED),
        )),
    };
    // Clipped: a terminal with no row under the list degrades to a no-op.
    f.render_widget(Paragraph::new(line), clip(f.area(), row_rect(width, y, 1)));
}

/// Lay out styled segments on one row: truncation is unicode-safe (whole
/// `char`s only, no byte slicing); a selected row gets an accent background
/// padded across the full inner width. Shared with the settings page.
pub(crate) fn segments_line(
    segs: Vec<(String, Style)>,
    width: usize,
    selected: bool,
    sel_style: Style,
) -> Line<'static> {
    if selected {
        let mut spans: Vec<Span> = Vec::new();
        let mut used = 0usize;
        for (text, _) in segs {
            if used >= width {
                break;
            }
            let taken: String = text.chars().take(width - used).collect();
            used += taken.chars().count();
            if !taken.is_empty() {
                spans.push(Span::styled(taken, sel_style));
            }
        }
        if used < width {
            spans.push(Span::styled(" ".repeat(width - used), sel_style));
        }
        return Line::from(spans);
    }
    let mut spans: Vec<Span> = Vec::new();
    let mut used = 0usize;
    for (text, style) in segs {
        if used >= width {
            break;
        }
        let taken: String = text.chars().take(width - used).collect();
        used += taken.chars().count();
        if !taken.is_empty() {
            spans.push(Span::styled(taken, style));
        }
    }
    Line::from(spans)
}

/// Shared block preset: rounded borders, accent title. Shared with the
/// settings page.
pub(crate) fn main_block(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(BORDER))
        .title(Span::styled(title.to_string(), Style::new().fg(ACCENT)))
}

pub(crate) fn row_rect(width: u16, y: u16, height: u16) -> Rect {
    Rect {
        x: 0,
        y,
        width,
        height,
    }
}

/// Clip a layout rect to the frame: a terminal smaller than the layout must
/// never hand a widget a rect outside the buffer.
pub(crate) fn clip(area: Rect, rect: Rect) -> Rect {
    rect.intersection(area)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_testkit::{
        app_with_history, app_with_recent_history, bg_cells, bg_rows, draw_on, draw_once, row_of,
    };
    use crate::state;
    use crate::storage::Store;

    #[test]
    fn shown_frame_draws_the_box_the_list_and_the_key_hints() {
        let text = draw_once(&app_with_history());
        assert_eq!(text.matches("❯ br").count(), 1, "input drawn once: {text}");
        assert_eq!(row_of(&text, "❯ br"), Some(1), "content row: {text}");
        // Input box first, candidate list under it.
        let top = text.lines().next().unwrap_or("");
        assert!(
            top.starts_with('╭') && top.ends_with('╮'),
            "box border row: {top}"
        );
        assert_eq!(text.matches('╭').count(), 2, "box + list block: {text}");
        assert_eq!(
            row_of(&text, "↻ br docs"),
            Some(INPUT_BOX_H as usize + 1),
            "first list row under the box: {text}"
        );
        assert!(text.contains("★ br ·"), "alias row: {text}");
        assert!(
            text.contains("matches · 1 history · 1 alias"),
            "list title: {text}"
        );
        assert!(text.contains("alt+d hide"), "key hints: {text}");
        assert!(text.contains("Enter run"), "key hints: {text}");
    }

    #[test]
    fn colon_and_settings_inputs_draw_no_candidate_list() {
        let mut app = app_with_history();
        app.input = ":".to_string();
        let colon = draw_once(&app);
        assert!(!colon.contains("matches ·"), "no list: {colon}");
        assert!(!colon.contains('★'), "no list rows: {colon}");
        assert_eq!(colon.matches('╭').count(), 1, "input box only: {colon}");
        assert_eq!(row_of(&colon, "❯ :"), Some(1), "content row: {colon}");
        assert!(colon.contains("Enter run"), "hints still drawn: {colon}");

        app.input = "/settings".to_string();
        let slash = draw_once(&app);
        assert!(!slash.contains("matches ·"), "no list: {slash}");
        assert_eq!(slash.matches('╭').count(), 1, "input box only: {slash}");
        assert_eq!(
            row_of(&slash, "❯ /settings"),
            Some(1),
            "typed input: {slash}"
        );
    }

    #[test]
    fn status_message_renders_under_the_list() {
        let mut app = app_with_history();
        let rows = state::candidates(&app).len();
        let below = INPUT_BOX_H as usize + 2 + rows;

        app.status = Some((true, "br ok: docs".to_string()));
        let ok = draw_once(&app);
        assert_eq!(row_of(&ok, "✓ br ok: docs"), Some(below), "{ok}");
        assert!(
            !ok.contains("alt+d hide"),
            "status replaces the hints: {ok}"
        );

        app.status = Some((false, "no match".to_string()));
        let err = draw_once(&app);
        assert_eq!(row_of(&err, "✗ no match"), Some(below), "{err}");
        assert!(err.contains("↻ br docs"), "list stays: {err}");
    }

    #[test]
    fn selection_and_cursor_paint_the_only_backgrounds() {
        let app = app_with_history();
        let painted = bg_cells(&app, 80, 14);
        let cursor: Vec<_> = painted.iter().filter(|(_, _, c)| *c == CURSOR).collect();
        assert_eq!(cursor.len(), 1, "one cursor cell: {painted:?}");
        assert_eq!(cursor[0].1, 1, "cursor on the box content row: {painted:?}");
        let sel: Vec<_> = painted.iter().filter(|(_, _, c)| *c == SELECT_BG).collect();
        assert_eq!(sel.len(), 78, "selected row padded to inner width");
        assert_eq!(
            bg_rows(&painted, SELECT_BG),
            std::iter::once(INPUT_BOX_H + 1).collect(),
            "the top candidate row only: {painted:?}"
        );
    }

    #[test]
    fn palette_draws_every_command_under_the_box() {
        let mut app = app_with_history();
        app.palette = Some(0);
        let text = draw_once(&app);
        assert!(text.contains("commands"), "palette title: {text}");
        for c in &commands::ALL {
            assert!(text.contains(c.token), "missing {}: {text}", c.token);
        }
        assert!(text.contains(":add "), "token padded for width: {text}");
        assert_eq!(
            row_of(&text, ":add"),
            Some(INPUT_BOX_H as usize + 1),
            "first row under the box: {text}"
        );
    }

    #[test]
    fn palette_selected_row_paints_the_select_background() {
        let mut app = app_with_history();
        app.palette = Some(2);
        let painted = bg_cells(&app, 80, 14);
        let cursor = painted.iter().filter(|(_, _, c)| *c == CURSOR).count();
        assert_eq!(cursor, 1, "still one cursor cell: {painted:?}");
        let sel: Vec<_> = painted.iter().filter(|(_, _, c)| *c == SELECT_BG).collect();
        assert!(!sel.is_empty(), "selected row painted: {painted:?}");
        let rows: std::collections::BTreeSet<u16> = sel.iter().map(|(_, y, _)| *y).collect();
        assert_eq!(
            rows,
            std::iter::once(INPUT_BOX_H + 3).collect(),
            "the selected (third) row only: {painted:?}"
        );
        assert!(
            sel.iter().all(|(x, _, _)| (1..=78).contains(x)),
            "paint stays inside the block border: {painted:?}"
        );
    }

    #[test]
    fn palette_scrolls_the_selection_into_view() {
        let mut app = app_with_history();
        app.palette = Some(commands::len() - 1);
        let text = draw_on(&app, 80, 8);
        assert!(text.contains("/settings"), "last row visible: {text}");
        assert!(text.contains("commands"), "title still drawn: {text}");
    }

    #[test]
    fn palette_in_tiny_terminals_does_not_panic() {
        let mut app = app_with_history();
        app.palette = Some(0);
        for (w, h) in [
            (0, 0),
            (1, 1),
            (2, 2),
            (5, 4),
            (80, 1),
            (80, 2),
            (80, INPUT_BOX_H),
            (80, INPUT_BOX_H + 1),
        ] {
            let _ = draw_on(&app, w, h);
        }
    }

    #[test]
    fn tiny_and_zero_size_terminals_do_not_panic() {
        let typed = app_with_history();
        let mut with_status = app_with_history();
        with_status.status = Some((false, "no match".to_string()));
        // The empty bar draws the recent list; the typed one draws matches.
        let recent = app_with_recent_history(12);
        let mut recent_selected = app_with_recent_history(3);
        recent_selected.cursor = 2;
        for (w, h) in [
            (0, 0),
            (0, 5),
            (1, 1),
            (2, INPUT_BOX_H),
            (5, 2),
            (40, 6),
            (40, INPUT_BOX_H + 2),
            (80, 0),
            (80, INPUT_BOX_H),
            (80, INPUT_BOX_H + 1),
            (80, INPUT_BOX_H + 2),
        ] {
            for a in [&typed, &with_status, &recent, &recent_selected] {
                let _ = draw_on(a, w, h);
            }
        }
    }

    #[test]
    fn hidden_frame_shows_only_the_hint_with_a_custom_wake_key() {
        let mut app = state::new(Store::default(), false);
        app.visibility = Visibility::Hidden;
        app.wake = keyspec::parse("alt+j").expect("fixture key");
        let text = draw_once(&app);
        assert!(text.contains("hidden"), "hint: {text}");
        assert!(text.contains("alt+j"), "custom wake key: {text}");
        assert!(!text.contains('╭'), "no box while hidden: {text}");
    }

    #[test]
    fn key_hints_drop_whole_segments_on_a_narrow_bar() {
        // 52 cells: the quarter-width bar on a 1920px screen.
        let narrow = keys_hint("alt+d", 52);
        assert!(narrow.contains("alt+d hide"), "{narrow}");
        assert!(narrow.contains("Enter run"), "{narrow}");
        assert!(narrow.contains("↑↓/Tab select"), "{narrow}");
        assert!(!narrow.contains("Ctrl+C quit"), "{narrow}");
        assert!(narrow.chars().count() <= 52, "overflows: {narrow}");
    }

    #[test]
    fn key_hints_keep_every_segment_on_a_wide_bar() {
        let wide = keys_hint("alt+d", 140);
        assert!(wide.contains("Ctrl+U clear"), "{wide}");
        assert!(wide.contains("Ctrl+C quit"), "{wide}");
    }

    #[test]
    fn key_hints_never_shrink_past_the_wake_hint() {
        assert_eq!(keys_hint("alt+d", 4), " alt+d hide ");
    }
}
