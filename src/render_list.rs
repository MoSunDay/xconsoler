//! Candidate-list rendering: the ranked rows under the input box. This unit
//! owns the list block (title, windowing, highlight) and the per-kind row
//! segments; frame dispatch, the input box, the palette, the status row and
//! the shared layout helpers stay in `crate::render`.

use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::alias;
use crate::matcher::Candidate;
use crate::platform::{self, Platform};
use crate::render::{clip, main_block, row_rect, segments_line};
use crate::state::{self, App};
use crate::theme::{ACCENT, MUTED, SELECT_BG, SUBTLE, TEXT};

/// Candidate list under the input box (hidden while the palette is open): one
/// row per ranked candidate, Up/Down to move the highlight, Enter to run it.
/// Returns the first free row under the list.
pub(crate) fn draw_candidates(f: &mut Frame, app: &App, width: u16, y: u16) -> u16 {
    let cands = state::candidates(app);
    if cands.is_empty() {
        return y;
    }
    // The block needs 2 border rows and the status/hints row 1; a terminal
    // with no room for both degrades to no list at all.
    let free = f.area().height.saturating_sub(y).saturating_sub(1) as usize;
    // `state::candidates` already caps the count (recent or ranked); here the
    // list only shrinks to the rows the layout leaves free.
    let rows = cands.len().min(free.saturating_sub(2));
    if rows == 0 {
        return y;
    }
    let layout = row_rect(width, y, (rows as u16).saturating_add(2));
    let block = main_block(&list_title(app, &cands, rows));
    let inner = block.inner(layout);
    let sel_style = Style::new().bg(SELECT_BG).fg(TEXT);
    // Same fallback as `state::selected`, then windowed like `draw_palette`
    // so the highlight can follow the cursor past the visible rows (Enter
    // runs `state::selected`, which uses the same unclamped cursor).
    let cursor = if app.cursor < cands.len() {
        app.cursor
    } else {
        0
    };
    let start = cursor
        .saturating_sub(rows.saturating_sub(1))
        .min(cands.len().saturating_sub(rows));
    let pf = platform::current();

    let lines: Vec<Line> = cands
        .iter()
        .skip(start)
        .take(rows)
        .enumerate()
        .map(|(i, cand)| {
            let segs = match cand {
                Candidate::History { idx } => history_segments(app, *idx),
                Candidate::Alias { name } => alias_segments(app, name, pf),
                Candidate::Arg { alias, key } => arg_row_segments(app, alias, key),
            };
            segments_line(segs, inner.width as usize, i + start == cursor, sel_style)
        })
        .collect();
    let rect = clip(f.area(), layout);
    f.render_widget(Paragraph::new(lines).block(block), rect);
    y.saturating_add(rows as u16).saturating_add(2)
}

/// List block title: `recent - n history` for the empty bar, per-kind counts
/// while a query is typed.
fn list_title(app: &App, cands: &[Candidate], rows: usize) -> String {
    let shown = &cands[..rows.min(cands.len())];
    let hist = shown
        .iter()
        .filter(|c| matches!(c, Candidate::History { .. }))
        .count();
    let args = shown
        .iter()
        .filter(|c| matches!(c, Candidate::Arg { .. }))
        .count();
    if app.input.trim().is_empty() {
        return format!(" recent · {hist} history ");
    }
    format!(
        " matches · {hist} history · {} alias · {args} args ",
        shown.len() - hist - args
    )
}

/// History row: the record mark, the entry label and the recorded input.
fn history_segments(app: &App, idx: usize) -> Vec<(String, Style)> {
    match app.store.history.get(idx) {
        Some(entry) => {
            let label = match alias::resolve(&app.aliases, &entry.alias) {
                Some(def) => alias::entry_label(def).to_string(),
                None => entry.alias.clone(),
            };
            vec![
                ("↻ ".into(), Style::new().fg(MUTED)),
                (label, Style::new().fg(ACCENT)),
                (" ".into(), Style::new().fg(TEXT)),
                (entry.input(), Style::new().fg(TEXT)),
            ]
        }
        None => vec![("…".into(), Style::new().fg(MUTED))],
    }
}

/// Alias row: the star mark, the alias label and its command template for
/// the current platform.
fn alias_segments(app: &App, name: &str, pf: Platform) -> Vec<(String, Style)> {
    match alias::resolve(&app.aliases, name) {
        Some(def) => {
            let cmd = match pf {
                Platform::Linux => def.linux.as_deref(),
                Platform::Macos => def.macos.as_deref(),
            }
            .unwrap_or("—");
            vec![
                ("★ ".into(), Style::new().fg(ACCENT)),
                (alias::label(def), Style::new().fg(ACCENT)),
                (" · ".into(), Style::new().fg(SUBTLE)),
                (cmd.to_string(), Style::new().fg(SUBTLE)),
            ]
        }
        None => vec![("★ …".into(), Style::new().fg(MUTED))],
    }
}

/// Named-arg row: the arg mark, the key and its value (the input is
/// `<alias> <partial>`).
fn arg_row_segments(app: &App, alias: &str, key: &str) -> Vec<(String, Style)> {
    let value = alias::resolve(&app.aliases, alias)
        .and_then(|d| d.args.get(key).cloned())
        .unwrap_or_else(|| "…".to_string());
    vec![
        ("↳ ".into(), Style::new().fg(MUTED)),
        (key.to_string(), Style::new().fg(ACCENT)),
        (" · ".into(), Style::new().fg(SUBTLE)),
        (value, Style::new().fg(SUBTLE)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::INPUT_BOX_H;
    use crate::render_testkit::{
        app_with_history, app_with_recent_history, bg_cells, bg_rows, draw_on, history_rows, row_of,
    };
    use crate::state::RECENT_LIMIT;
    use crate::theme::CURSOR;

    #[test]
    fn list_title_counts_kinds_and_names_the_recent_list() {
        let cands = vec![
            Candidate::History { idx: 0 },
            Candidate::Alias {
                name: "br".to_string(),
            },
            Candidate::Arg {
                alias: "br".to_string(),
                key: "baidu".to_string(),
            },
        ];
        let typing = app_with_history();
        assert_eq!(
            list_title(&typing, &cands, 3),
            " matches · 1 history · 1 alias · 1 args "
        );
        let empty = app_with_recent_history(2);
        assert_eq!(list_title(&empty, &cands[..1], 1), " recent · 1 history ");
    }

    #[test]
    fn empty_input_lists_only_the_recent_history_rows() {
        let app = app_with_recent_history(12);
        let text = draw_on(&app, 80, 20);
        assert_eq!(
            history_rows(&text),
            RECENT_LIMIT,
            "exactly the recent limit is drawn: {text}"
        );
        assert!(text.contains("recent · 10 history"), "title: {text}");
        assert!(text.contains("cmd-11"), "newest entry: {text}");
        assert!(text.contains("cmd-02"), "tenth newest entry: {text}");
        assert!(!text.contains("cmd-01"), "oldest dropped: {text}");
        assert!(!text.contains("cmd-00"), "oldest dropped: {text}");
        assert!(
            !text.contains('★'),
            "no alias rows on the empty bar: {text}"
        );
    }

    #[test]
    fn highlight_follows_the_cursor_and_paints_the_select_background() {
        let mut app = app_with_recent_history(5); // history[0] = cmd-04, [2] = cmd-02
        app.cursor = 2;
        let painted = bg_cells(&app, 80, 14);
        let sel = bg_rows(&painted, SELECT_BG);
        let expected = INPUT_BOX_H + 1 + 2; // first list row + cursor
        assert_eq!(sel, std::iter::once(expected).collect(), "{painted:?}");
        assert_eq!(
            bg_rows(&painted, CURSOR),
            std::iter::once(1).collect(),
            "still one cursor cell: {painted:?}"
        );
        let text = draw_on(&app, 80, 14);
        assert_eq!(
            row_of(&text, "cmd-02"),
            Some(expected as usize),
            "highlighted row is the cursor candidate: {text}"
        );
        assert!(
            painted
                .iter()
                .filter(|(_, _, c)| *c == SELECT_BG)
                .all(|(x, _, _)| (1..=78).contains(x)),
            "paint stays inside the block border: {painted:?}"
        );
    }

    /// More candidates than visible rows: the list scrolls so the highlighted
    /// row is always the candidate Enter runs (`state::selected`).
    #[test]
    fn long_lists_window_the_highlight_with_the_cursor() {
        let mut app = app_with_recent_history(12); // 12 entries, 10 candidates
        app.cursor = 9; // last candidate: history[9] = cmd-02
        let painted = bg_cells(&app, 80, 14); // 8 list rows for 10 candidates
                                              // Window starts at candidate 2, so cursor 9 is the 8th drawn row.
        let last_row = INPUT_BOX_H + 1 + 7;
        assert_eq!(
            bg_rows(&painted, SELECT_BG),
            std::iter::once(last_row).collect(),
            "{painted:?}"
        );
        let text = draw_on(&app, 80, 14);
        assert!(
            text.lines()
                .nth(last_row as usize)
                .is_some_and(|l| l.contains("cmd-02")),
            "highlighted row is the cursor candidate: {text}"
        );
        assert!(
            text.contains("cmd-03") && !text.contains("cmd-11"),
            "the window scrolled off the newest rows: {text}"
        );
        assert_eq!(
            state::selected(&app),
            Some(Candidate::History { idx: 9 }),
            "Enter runs the highlighted row"
        );
        assert_eq!(app.store.history[9].input(), "cmd-02");
    }

    /// The list shrinks to the room left between the box and the hint row.
    #[test]
    fn tiny_terminal_clamps_the_list_to_the_room_left() {
        // 40x6: box (3 rows) + 1 list row + 1 hint row leaves no border room
        // for a block, so the list degrades to nothing rather than panicking.
        let text = draw_on(&app_with_recent_history(4), 40, 6);
        assert_eq!(
            history_rows(&text),
            0,
            "no room for a bordered list: {text}"
        );
        assert!(text.contains("❯ "), "box still drawn: {text}");
        // 40x7: two rows of block fit, showing the two newest entries only.
        let text = draw_on(&app_with_recent_history(4), 40, 7);
        assert_eq!(history_rows(&text), 1, "one list row fits: {text}");
        assert!(text.contains("cmd-03"), "newest first: {text}");
        assert!(
            text.contains("recent · 1 history"),
            "title counts drawn rows"
        );
    }
}
