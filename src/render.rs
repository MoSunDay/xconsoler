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
    format!(" xconsoler hidden — {wake} wake · Ctrl+C/D quit ")
}

/// Key-hints row under the list, shown while there is no status message.
/// A narrow bar drops the least essential hints whole (no mid-word clipping):
/// `Ctrl+C/D quit` goes first, the wake/run/select trio always survives.
fn keys_hint(wake: &str, width: usize) -> String {
    let segs = [
        format!("{wake} hide"),
        "Enter run".to_string(),
        "↑↓/Tab select".to_string(),
        "Ctrl+W/U edit".to_string(),
        "Ctrl+C/D quit".to_string(),
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

/// Command palette under the input box: one row per command matching the
/// current `:`/`/` query (the whole catalog for plain input), Up/Down to
/// move, Enter to accept, Esc to close. Scrolls so the selection stays
/// visible; no rows or no room degrades to no list.
fn draw_palette(f: &mut Frame, app: &App, width: u16, y: u16) -> u16 {
    let items = commands::palette_items(&app.input);
    let total = items.len();
    if total == 0 {
        return y;
    }
    // Same framed/bare split as the candidate list (see `panel_rows`).
    let (framed, rows) = panel_rows(f.area().height, y, total, app.status.is_some());
    if rows == 0 {
        return y;
    }
    let sel = app.palette.unwrap_or(0).min(total - 1);
    let start = sel.saturating_sub(rows - 1).min(total - rows);
    let inner_width = width.saturating_sub(if framed { 2 } else { 0 });
    let sel_style = Style::new().bg(SELECT_BG).fg(TEXT);
    let mut lines = Vec::with_capacity(rows);
    for (i, spec) in items.iter().enumerate().skip(start).take(rows) {
        let segs = vec![
            (format!(" {} ", spec.token), Style::new().fg(ACCENT)),
            (spec.desc.to_string(), Style::new().fg(MUTED)),
        ];
        lines.push(segments_line(
            segs,
            inner_width as usize,
            i == sel,
            sel_style,
        ));
    }
    let mut para = Paragraph::new(lines);
    let mut used = rows as u16;
    if framed {
        para = para.block(main_block(&format!(" commands \u{b7} {total} ")));
        used = used.saturating_add(2);
    }
    f.render_widget(para, clip(f.area(), row_rect(width, y, used)));
    y.saturating_add(used)
}

/// The bar block: rounded border, no title.
fn input_block() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(BORDER))
}

/// Full-width input box: prompt, the text before the caret, a one-cell caret
/// block, the char the caret sits on and the trailing text. The caret is a
/// blank cell *before* the character at the caret, so a wide char keeps both
/// its columns and never drifts against the caret. Returns the first row
/// under the box.
fn draw_input_box(f: &mut Frame, app: &App, width: u16) -> u16 {
    let rect = clip(f.area(), row_rect(width, 0, INPUT_BOX_H));
    let block = input_block();
    let inner = block.inner(rect);
    let budget = inner.width.saturating_sub(2) as usize; // "❯ " is painted first
    let mut spans = vec![Span::styled("❯ ", Style::new().fg(ACCENT))];
    if budget > 0 {
        let (before, at, after) = crate::textedit::window(&app.input, app.caret, budget);
        spans.push(Span::styled(before, Style::new().fg(TEXT)));
        spans.push(Span::styled(" ", Style::new().bg(CURSOR)));
        if let Some(c) = at {
            spans.push(Span::styled(c.to_string(), Style::new().fg(TEXT)));
        }
        spans.push(Span::styled(after, Style::new().fg(TEXT)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).block(block), rect);
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

/// Rows a panel under the input box may claim, as `(framed, rows)`.
///
/// `keep_status` is true while a status message owns the row under the
/// panel; otherwise the key-hints row sits there and the framed layout
/// keeps it whenever it can. A bar too short for the border (`room < 3`)
/// draws the rows bare instead - no border, no title - so the quick picks
/// stay visible in a slim bar, giving the hint row up first.
pub(crate) fn panel_rows(area_h: u16, y: u16, want: usize, keep_status: bool) -> (bool, usize) {
    let below = area_h.saturating_sub(y) as usize;
    let room = below.saturating_sub(usize::from(keep_status));
    if room >= 3 {
        // Border/title (2) and the hint row go first; one row is always kept.
        (true, want.min(room.saturating_sub(3).max(1)))
    } else {
        (false, want.min(room))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_testkit::{
        app_with_history, app_with_recent_history, bg_cells, bg_rows, cell_symbol, draw_on,
        draw_once, row_of,
    };
    use crate::state;
    use crate::storage::Store;

    #[test]
    fn panel_rows_frames_only_when_border_and_a_row_fit() {
        // Roomy terminal: the whole panel, hint or status row below.
        assert_eq!(panel_rows(14, 3, 5, false), (true, 5));
        assert_eq!(panel_rows(14, 3, 5, true), (true, 5));
        // Framed layouts keep the hint row: rows = room - border - hint.
        assert_eq!(panel_rows(11, 3, 9, true), (true, 4));
        // Exactly border + one row + the message row.
        assert_eq!(panel_rows(7, 3, 9, true), (true, 1));
        // Three rows below: border + one row, the hint row drops.
        assert_eq!(panel_rows(6, 3, 9, false), (true, 1));
        // Two rows left: bare rows take both, hint row included.
        assert_eq!(panel_rows(5, 3, 9, false), (false, 2));
        assert_eq!(panel_rows(4, 3, 9, false), (false, 1));
        // A status message always keeps its row.
        assert_eq!(panel_rows(4, 3, 9, true), (false, 0));
        assert_eq!(panel_rows(3, 3, 9, false), (false, 0));
    }

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
        assert!(text.contains("↳ br baidu ·"), "shortcut row: {text}");
        assert!(
            text.contains("matches · 1 history · 2 shortcut"),
            "list title: {text}"
        );
        assert!(text.contains("alt+d hide"), "key hints: {text}");
        assert!(text.contains("Enter run"), "key hints: {text}");
    }

    #[test]
    fn caret_cell_sits_left_of_the_char_at_the_caret() {
        let mut app = app_with_history();
        app.caret = 1; // input is "br": caret between the two chars
        let painted = bg_cells(&app, 80, 14);
        let cursor: Vec<_> = painted.iter().filter(|(_, _, c)| *c == CURSOR).collect();
        assert_eq!(cursor.len(), 1, "one cursor cell: {painted:?}");
        assert_eq!(cursor[0].1, 1, "cursor on the box content row: {painted:?}");
        assert_eq!(
            cursor[0].0, 4,
            "border(1) + prompt(2) + 'b' -> caret cell x=4: {painted:?}"
        );
        assert_eq!(
            cell_symbol(&app, 80, 14, 4, 1),
            " ",
            "the caret cell itself is blank: {painted:?}"
        );
        assert_eq!(
            cell_symbol(&app, 80, 14, 5, 1),
            "r",
            "the char at the caret follows it: {painted:?}"
        );
        let text = draw_once(&app);
        assert_eq!(row_of(&text, "❯ b"), Some(1), "both chars kept: {text}");
        assert!(text.contains('r'), "the char at the caret is drawn: {text}");
    }

    #[test]
    fn long_input_scrolls_so_the_caret_stays_visible() {
        let mut app = app_with_history();
        app.input = format!("{}the-end", "x".repeat(50));
        app.caret = app.input.chars().count();
        let text = draw_on(&app, 40, 14);
        assert!(text.contains("the-end"), "tail visible: {text}");
        let painted = bg_cells(&app, 40, 14);
        let cursor: Vec<_> = painted.iter().filter(|(_, _, c)| *c == CURSOR).collect();
        assert_eq!(cursor.len(), 1, "one cursor cell: {painted:?}");
        assert_eq!(
            *cursor[0],
            (38, 1, CURSOR),
            "caret parks on the last inner cell: {painted:?}"
        );

        // Caret at the start: the window shows the head, not the tail.
        app.caret = 0;
        let head = draw_on(&app, 40, 14);
        assert!(!head.contains("the-end"), "tail scrolled away: {head}");
        // The caret cell sits between "❯ " and the first x.
        assert!(head.contains("❯  x"), "head visible: {head}");
    }

    #[test]
    fn text_chars_are_preserved_when_not_scrolled() {
        let mut app = app_with_history();
        app.input = "café au lait".to_string();
        app.caret = 3; // before the 'é'
        let text = draw_once(&app);
        assert_eq!(
            row_of(&text, "❯ caf"),
            Some(1),
            "the text before the caret is drawn: {text}"
        );
        assert!(text.contains("au lait"), "the tail is drawn: {text}");
        let painted = bg_cells(&app, 80, 14);
        let cursor: Vec<_> = painted.iter().filter(|(_, _, c)| *c == CURSOR).collect();
        assert_eq!(cursor.len(), 1, "one cursor cell: {painted:?}");
        assert_eq!(
            cell_symbol(&app, 80, 14, cursor[0].0, cursor[0].1),
            " ",
            "the caret cell is blank"
        );
        assert_eq!(
            cell_symbol(&app, 80, 14, cursor[0].0 + 1, cursor[0].1),
            "é",
            "the accented char follows the caret"
        );
    }

    #[test]
    fn cjk_caret_is_one_cell_and_the_row_stays_in_bounds() {
        let mut app = app_with_history();
        app.input = "中文中文中文".to_string();
        app.caret = app.input.chars().count();
        // width 12: border(2) + prompt(2) + six text cells + one caret cell.
        let text = draw_on(&app, 12, 14);
        let row = text.lines().nth(1).unwrap_or("");
        assert_eq!(row.chars().count(), 12, "row fills the width: {row:?}");
        assert!(
            row.starts_with('│') && row.ends_with('│'),
            "content row keeps both borders: {row:?}"
        );
        let painted = bg_cells(&app, 12, 14);
        let cursor: Vec<_> = painted
            .iter()
            .filter(|(_, _, c)| *c == CURSOR)
            .copied()
            .collect();
        assert_eq!(
            cursor,
            vec![(9, 1, CURSOR)],
            "one caret cell after three wide chars: {painted:?}"
        );
        // Caret in the middle: three wide chars before it are six cells.
        app.caret = 3;
        let painted = bg_cells(&app, 12, 14);
        let cursor: Vec<_> = painted
            .iter()
            .filter(|(_, _, c)| *c == CURSOR)
            .copied()
            .collect();
        assert_eq!(
            cursor,
            vec![(9, 1, CURSOR)],
            "caret after 3 wide chars stays at x=9: {painted:?}"
        );
        assert_eq!(
            cell_symbol(&app, 12, 14, 9, 1),
            " ",
            "the caret keeps one blank cell"
        );
    }

    #[test]
    fn colon_input_draws_no_candidate_list_while_slash_shows_the_palette() {
        let mut app = app_with_history();
        app.input = ":".to_string();
        app.caret = 1;
        let colon = draw_once(&app);
        assert!(!colon.contains("matches ·"), "no list: {colon}");
        assert!(!colon.contains('★'), "no list rows: {colon}");
        assert_eq!(colon.matches('╭').count(), 1, "input box only: {colon}");
        assert_eq!(row_of(&colon, "❯ :"), Some(1), "content row: {colon}");
        assert!(colon.contains("Enter run"), "hints still drawn: {colon}");

        // `/settings` is a live slash query: the palette follows it.
        app.input = "/settings".to_string();
        app.caret = app.input.chars().count();
        app.palette = Some(0);
        let slash = draw_once(&app);
        assert!(slash.contains("commands · 1"), "palette title: {slash}");
        assert_eq!(
            slash.matches('╭').count(),
            2,
            "input box + palette frame: {slash}"
        );
        assert_eq!(
            row_of(&slash, "❯ /settings"),
            Some(1),
            "content row: {slash}"
        );
        assert_eq!(
            row_of(&slash, "open the settings page"),
            Some(INPUT_BOX_H as usize + 1),
            "the /settings row under the box: {slash}"
        );
        assert!(slash.contains("Enter run"), "hints below: {slash}");
    }

    #[test]
    fn slash_query_filters_the_palette_rows() {
        let mut app = app_with_history();
        app.input = "/se".to_string();
        app.caret = app.input.chars().count();
        app.palette = Some(0);
        let text = draw_once(&app);
        assert!(text.contains("commands · 1"), "one match: {text}");
        assert_eq!(
            row_of(&text, "open the settings page"),
            Some(INPUT_BOX_H as usize + 1),
            "the /settings row: {text}"
        );

        // A query with no hits paints no palette frame at all.
        app.input = "/zz".to_string();
        app.caret = app.input.chars().count();
        let none = draw_once(&app);
        assert!(!none.contains("commands ·"), "no palette title: {none}");
        assert_eq!(none.matches('╭').count(), 1, "input box only: {none}");
        assert_eq!(row_of(&none, "❯ /zz"), Some(1), "content row: {none}");
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
        assert!(!narrow.contains("Ctrl+C/D quit"), "{narrow}");
        assert!(narrow.chars().count() <= 52, "overflows: {narrow}");
    }

    #[test]
    fn key_hints_keep_every_segment_on_a_wide_bar() {
        let wide = keys_hint("alt+d", 140);
        assert!(wide.contains("Ctrl+W/U edit"), "{wide}");
        assert!(wide.contains("Ctrl+C/D quit"), "{wide}");
    }

    #[test]
    fn key_hints_never_shrink_past_the_wake_hint() {
        assert_eq!(keys_hint("alt+d", 4), " alt+d hide ");
    }
}
