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
use crate::render::{clip, main_block, panel_rows, row_rect, segments_line};
use crate::state::{self, App};
use crate::theme::{ACCENT, MUTED, SELECT_BG, TEXT};

/// Candidate list under the input box (hidden while the palette is open): one
/// row per ranked candidate, Up/Down to move the highlight, Enter to run it.
/// Returns the first free row under the list.
pub(crate) fn draw_candidates(f: &mut Frame, app: &App, width: u16, y: u16) -> u16 {
    let cands = state::candidates(app);
    if cands.is_empty() {
        return y;
    }
    // `state::candidates` already caps the count (recent or ranked); the
    // panel takes whatever the layout leaves under the box - framed while
    // the border/title and a row fit, bare rows in a slim bar.
    let (framed, rows) = panel_rows(f.area().height, y, cands.len(), app.status.is_some());
    if rows == 0 {
        return y;
    }
    let inner_width = width.saturating_sub(if framed { 2 } else { 0 });
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

    let lines: Vec<Line> = cands
        .iter()
        .skip(start)
        .take(rows)
        .enumerate()
        .map(|(i, cand)| {
            let segs = match cand {
                Candidate::History { idx } => history_segments(app, *idx),
                Candidate::Shortcut { alias, key } => shortcut_row_segments(alias, key),
            };
            segments_line(segs, inner_width as usize, i + start == cursor, sel_style)
        })
        .collect();
    let mut para = Paragraph::new(lines);
    let mut used = rows as u16;
    if framed {
        para = para.block(main_block(&list_title(app, &cands, rows)));
        used = used.saturating_add(2);
    }
    f.render_widget(para, clip(f.area(), row_rect(width, y, used)));
    y.saturating_add(used)
}

/// List block title: `recent - n history` for the empty bar, per-kind counts
/// while a query is typed.
fn list_title(app: &App, cands: &[Candidate], rows: usize) -> String {
    let shown = &cands[..rows.min(cands.len())];
    let hist = shown
        .iter()
        .filter(|c| matches!(c, Candidate::History { .. }))
        .count();
    let shortcuts = shown
        .iter()
        .filter(|c| matches!(c, Candidate::Shortcut { .. }))
        .count();
    if app.input.trim().is_empty() {
        return format!(" recent · {hist} history ");
    }
    format!(" matches · {hist} history · {shortcuts} shortcut ")
}

/// Marker drawn in place of an input the bar must not show.
const REDACTED: &str = "***";

/// History row: the record mark, the entry label and what the row may safely
/// show. A run by a registered shortcut key keeps that key - the key is a
/// label the store itself defines - while any other input (a pasted password,
/// a URL) is redacted to `***`: the row never echoes content the user typed
/// or pasted. The recorded input still sits base64 in the store, and a replay
/// feeds it back verbatim, so nothing is lost.
fn history_segments(app: &App, idx: usize) -> Vec<(String, Style)> {
    match app.store.history.get(idx) {
        Some(entry) => {
            let def = alias::resolve(&app.aliases, &entry.alias);
            let label = match def {
                Some(d) => alias::entry_label(d).to_string(),
                None => entry.alias.clone(),
            };
            let shown = match def.and_then(|d| shortcut_key(d, &entry.input())) {
                Some(key) => (key, Style::new().fg(TEXT)),
                None => (REDACTED.to_string(), Style::new().fg(MUTED)),
            };
            vec![
                ("↻ ".into(), Style::new().fg(MUTED)),
                (label, Style::new().fg(ACCENT)),
                (" ".into(), Style::new().fg(TEXT)),
                shown,
            ]
        }
        None => vec![("…".into(), Style::new().fg(MUTED))],
    }
}

/// The registered shortcut key `input` names, if any: the same head lookup
/// [`crate::exec::resolve_shortcuts`] performs, blank mapped values included,
/// so a key is shown exactly when running the entry would expand that key.
fn shortcut_key(def: &alias::AliasDef, input: &str) -> Option<String> {
    let head = input.trim().split(char::is_whitespace).next().unwrap_or("");
    let named = !head.is_empty()
        && def
            .shortcuts
            .get(head)
            .is_some_and(|v| !v.trim().is_empty());
    named.then(|| head.to_string())
}

/// Shortcut row: the shortcut mark and `alias key`. The registered value is
/// never drawn: it is the content the shortcut keeps off screen (often a
/// password or a URL) and `/settings` is where it can be inspected, shown as
/// stored - in base64.
fn shortcut_row_segments(alias: &str, key: &str) -> Vec<(String, Style)> {
    vec![
        ("↳ ".into(), Style::new().fg(MUTED)),
        (format!("{alias} {key}"), Style::new().fg(ACCENT)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::record;
    use crate::render::INPUT_BOX_H;
    use crate::render_testkit::{
        app_with_history, app_with_recent_history, bg_cells, bg_rows, draw_on, history_rows, row_of,
    };
    use crate::state::RECENT_LIMIT;
    use crate::storage::Store;
    use crate::theme::CURSOR;

    /// App with `n` recorded `br` entries (`cmd-00` .. `cmd-<n-1>`, older
    /// index = newer entry) whose inputs are registered shortcut keys, so
    /// every row shows its key and the windowing tests can tell rows apart.
    fn app_with_keyed_history(n: usize) -> App {
        let mut store = Store::default();
        let def = store
            .aliases
            .iter_mut()
            .find(|d| d.name == "br")
            .expect("seeded br alias");
        for i in 0..n {
            def.shortcuts
                .insert(format!("cmd-{i:02}"), format!("value-{i:02}"));
        }
        for i in 0..n {
            record(&mut store, "br", &format!("cmd-{i:02}"), i as u64 + 1);
        }
        state::new(store, false)
    }

    #[test]
    fn list_title_counts_kinds_and_names_the_recent_list() {
        let cands = vec![
            Candidate::History { idx: 0 },
            Candidate::Shortcut {
                alias: "br".to_string(),
                key: "baidu".to_string(),
            },
        ];
        let typing = app_with_history();
        assert_eq!(
            list_title(&typing, &cands, 2),
            " matches · 1 history · 1 shortcut "
        );
        let empty = app_with_recent_history(2);
        assert_eq!(list_title(&empty, &cands[..1], 1), " recent · 1 history ");
    }

    #[test]
    fn empty_input_lists_only_the_recent_history_rows() {
        let app = app_with_keyed_history(12);
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
            !text.contains('↳'),
            "no shortcut rows on the empty bar: {text}"
        );
    }

    #[test]
    fn highlight_follows_the_cursor_and_paints_the_select_background() {
        let mut app = app_with_keyed_history(5); // history[0] = cmd-04, [2] = cmd-02
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
        let mut app = app_with_keyed_history(12); // 12 entries, 10 candidates
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
        // 40x6: box (3 rows) + a bare 3-row list would leave no hint row, so
        // the list is framed down to the single row the border leaves; the
        // hint row is the first thing to go, never the picks.
        let text = draw_on(&app_with_keyed_history(4), 40, 6);
        assert_eq!(history_rows(&text), 1, "one framed list row: {text}");
        assert!(text.contains("recent · 1 history"), "title kept: {text}");
        assert!(text.contains("❯ "), "box still drawn: {text}");
        // 40x7: two rows of block fit, showing the two newest entries only.
        let text = draw_on(&app_with_keyed_history(4), 40, 7);
        assert_eq!(history_rows(&text), 1, "one list row fits: {text}");
        assert!(text.contains("cmd-03"), "newest first: {text}");
        assert!(
            text.contains("recent · 1 history"),
            "title counts drawn rows"
        );
    }

    /// The slim bar the launcher sizes from a history-less store still shows
    /// the typed quick picks: bare rows under the box, no border/title, and
    /// the hint row is the one that gives way.
    #[test]
    fn slim_bar_keeps_the_typed_candidates_visible() {
        let mut app = app_with_recent_history(0);
        app.input = "br".to_string();
        app.cursor = 2;
        let text = draw_on(&app, 40, 4);
        assert_eq!(row_of(&text, "↳ br baidu"), Some(3), "{text}");
        assert_eq!(
            text.matches('╭').count(),
            1,
            "only the input box is framed: {text}"
        );
        let mut app = app_with_recent_history(0);
        app.input = "br".to_string();
        app.cursor = 2;
        app.status = Some((false, "boom".to_string()));
        let text = draw_on(&app, 40, 4);
        assert!(
            text.contains("boom") && !text.contains("↳"),
            "a status message owns the only free row: {text}"
        );
    }

    /// Content the alias never named is never drawn: only the alias label and
    /// the redaction marker appear, whatever the user typed or pasted.
    #[test]
    fn history_row_redacts_an_input_the_alias_never_named() {
        let mut store = Store::default();
        record(&mut store, "cd", "hunter2", 1);
        let app = state::new(store, false);
        let text = draw_on(&app, 80, 14);
        assert!(text.contains("↻ cd ***"), "redacted row: {text}");
        assert!(!text.contains("hunter2"), "input never drawn: {text}");
    }

    /// A registered shortcut key is a store-defined label, so the row keeps
    /// it - `br cmd-01` stays readable while the mapped value stays hidden.
    #[test]
    fn history_row_keeps_a_registered_shortcut_key() {
        let app = app_with_keyed_history(2);
        let text = draw_on(&app, 80, 14);
        assert!(text.contains("↻ br cmd-01"), "key is a label: {text}");
        assert!(text.contains("↻ br cmd-00"), "key is a label: {text}");
    }

    /// The shortcut row names the key only: the registered value is what the
    /// shortcut keeps off screen, and `/settings` is where it can be read.
    #[test]
    fn shortcut_row_hides_the_registered_value() {
        let mut app = app_with_recent_history(0);
        app.input = "br baidu".to_string();
        app.caret = app.input.chars().count();
        let text = draw_on(&app, 80, 14);
        assert!(text.contains("↳ br baidu"), "key kept: {text}");
        assert!(
            !text.contains("https://www.baidu.com"),
            "value never drawn: {text}"
        );
    }
}
