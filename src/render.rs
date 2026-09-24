//! Frame rendering. All colours live in one place - `crate::theme` - so
//! call sites never hard-code raw `Color` values. The theme is the
//! terminator-rust kanagawa port; its 0.7 opacity policy means no explicit
//! cell backgrounds outside the selection/cursor inks.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::alias;
use crate::exec;
use crate::keyspec;
use crate::matcher::Candidate;
use crate::platform::{self, Platform};
use crate::settings_view;
use crate::state::{self, App, Mode, Visibility, CANDIDATE_LIMIT};
use crate::theme::{ACCENT, BORDER, CURSOR, ERR, MUTED, OK, SELECT_BG, SUBTLE, TEXT};

/// Bar title: the brand mark in text - accent chevron + cursor-coloured block,
/// the terminal echo of `assets/icon.svg` - in front of the wordmark.
fn brand_title() -> Line<'static> {
    Line::from(vec![
        Span::styled(" ❯", Style::new().fg(ACCENT)),
        Span::styled("▌", Style::new().fg(CURSOR)),
        Span::styled(" xconsoler ", Style::new().fg(ACCENT)),
    ])
}

/// Hidden-mode one-liner; the wake key is injected at render time so a
/// custom `--wake-key` / stored config shows the real binding.
fn hidden_hint(wake: &str) -> String {
    format!(" xconsoler hidden — {wake} wake · Ctrl+C quit ")
}

/// Key-hints line shown under the list when there is no status message.
fn keys_hint(wake: &str) -> String {
    format!(" {wake} hide · Enter run · ↑↓/Tab select · Ctrl+U clear · Ctrl+C quit ")
}
const HELP_TITLE: &str = " : commands ";

/// Help lines shown while the input starts with `:`; the first word of each
/// line is the token being explained.
const HELP_LINES: [&str; 7] = [
    ":add <name>[,<short>...] <linux-cmd> // <macos-cmd>",
    ":del <name>",
    ":arg <name> <key> <value...> — set a named argument",
    ":unarg <name> <key> — remove a named argument",
    ":help — show this help",
    "{input} — your input, shell-quoted into the command",
    "@stdin — your input is piped to the command's stdin",
];

const SLASH_TITLE: &str = " / commands ";

/// Help lines shown while the input starts with `/`.
const SLASH_LINES: [&str; 1] = ["/settings — open the settings page"];

/// Draw one frame: the hidden one-liner or the full launcher layout. In
/// settings mode the whole screen belongs to the settings page — the
/// long-bar/candidates layout is not drawn at all.
pub fn draw(f: &mut Frame, app: &App) {
    // Blank the frame first: switching visibility or shrinking the list must
    // not leave stale cells behind (ratatui only diffs what is re-rendered).
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
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hidden_hint(&keyspec::describe(&app.wake)),
            Style::new().fg(MUTED),
        ))),
        row_rect(f.area().width, 0, 1),
    );
}

fn draw_shown(f: &mut Frame, app: &App) {
    let width = f.area().width;
    let mut y = draw_input_bar(f, app, width);

    let cands = state::candidates(app);
    if app.input.starts_with(':') {
        y = draw_help(f, width, y, HELP_TITLE, &HELP_LINES);
    } else if app.input.starts_with('/') {
        y = draw_help(f, width, y, SLASH_TITLE, &SLASH_LINES);
    } else if !cands.is_empty() {
        y = draw_list(f, app, &cands, width, y);
    }
    draw_status(f, app, width, y);
}

/// Full-width input bar: `❯ ` + input + a reverse-space cursor at the end.
fn draw_input_bar(f: &mut Frame, app: &App, width: u16) -> u16 {
    let rect = row_rect(width, 0, 3);
    let block = main_block_line(brand_title());
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
    3
}

/// Candidate list under the bar. Returns the next free row.
/// List block title: per-kind row counts. Named-arg rows are only
/// mentioned when present (they take over the list while typing
/// `<alias> <partial>`).
fn list_title(cands: &[Candidate], rows: usize) -> String {
    let shown = &cands[..rows.min(cands.len())];
    let hist = shown
        .iter()
        .filter(|c| matches!(c, Candidate::History { .. }))
        .count();
    let args = shown
        .iter()
        .filter(|c| matches!(c, Candidate::Arg { .. }))
        .count();
    let mut parts = vec![
        format!("{hist} history"),
        format!("{} alias", shown.len() - hist - args),
    ];
    if args > 0 {
        parts.push(format!("{args} args"));
    }
    format!(" matches · {} ", parts.join(" · "))
}

fn draw_list(f: &mut Frame, app: &App, cands: &[Candidate], width: u16, y: u16) -> u16 {
    let rows = cands.len().min(CANDIDATE_LIMIT);
    let title = list_title(cands, rows);
    let rect = row_rect(width, y, rows as u16 + 2);
    let block = main_block(&title);
    let inner = block.inner(rect);
    let sel_style = Style::new().bg(SELECT_BG).fg(TEXT);
    // Same clamp as `state::selected` so the highlight matches what Enter runs.
    let cursor = app.cursor.min(rows.saturating_sub(1));
    let pf = platform::current();

    let lines: Vec<Line> = cands
        .iter()
        .take(rows)
        .enumerate()
        .map(|(i, cand)| {
            let segs = match cand {
                Candidate::History { idx } => history_segments(app, *idx),
                Candidate::Alias { name } => alias_segments(app, name, pf),
                Candidate::Arg { alias, key } => arg_row_segments(app, alias, key),
            };
            segments_line(segs, inner.width as usize, i == cursor, sel_style)
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(block), rect);
    rect.y + rect.height
}

/// `:`/`/` command help block replacing the list. Returns the next free row.
fn draw_help(f: &mut Frame, width: u16, y: u16, title: &str, lines: &[&str]) -> u16 {
    let rect = row_rect(width, y, lines.len() as u16 + 2);
    let block = main_block(title);
    let lines: Vec<Line> = lines
        .iter()
        .map(|l| match l.split_once(' ') {
            Some((token, rest)) => Line::from(vec![
                Span::styled(token, Style::new().fg(ACCENT)),
                Span::styled(format!(" {rest}"), Style::new().fg(SUBTLE)),
            ]),
            None => Line::from(Span::styled(*l, Style::new().fg(SUBTLE))),
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(block), rect);
    rect.y + rect.height
}

/// One line below the list (or bar): status message or key hints.
fn draw_status(f: &mut Frame, app: &App, width: u16, y: u16) {
    if y >= f.area().height {
        return;
    }
    let line = match &app.status {
        Some((true, msg)) => Line::from(vec![
            Span::styled("✓ ", Style::new().fg(OK)),
            Span::styled(msg.clone(), Style::new().fg(OK)),
        ]),
        Some((false, msg)) => Line::from(vec![
            Span::styled("✗ ", Style::new().fg(ERR)),
            Span::styled(msg.clone(), Style::new().fg(ERR)),
        ]),
        None => Line::from(Span::styled(
            keys_hint(&keyspec::describe(&app.wake)),
            Style::new().fg(MUTED),
        )),
    };
    f.render_widget(Paragraph::new(line), row_rect(width, y, 1));
}

/// History row: `↻ <entry label> <recorded input>`.
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

/// Alias row: `★ <label> · <command template for the current platform>` —
/// or, when the input is `<this alias> <rest>`, a preview of the input the
/// command will receive (named args resolved): `★ br · → https://…`.
fn alias_segments(app: &App, name: &str, pf: Platform) -> Vec<(String, Style)> {
    match alias::resolve(&app.aliases, name) {
        Some(def) => {
            let mut parts = app.input.trim().splitn(2, char::is_whitespace);
            let head = parts.next().unwrap_or("");
            let rest = parts.next().unwrap_or("");
            let is_trigger = !rest.trim().is_empty()
                && alias::resolve(&app.aliases, head).is_some_and(|h| h.name == def.name);
            if is_trigger {
                let resolved = exec::resolve_args(def, rest);
                return vec![
                    ("★ ".into(), Style::new().fg(ACCENT)),
                    (alias::label(def), Style::new().fg(ACCENT)),
                    (" · ".into(), Style::new().fg(SUBTLE)),
                    ("→ ".into(), Style::new().fg(MUTED)),
                    (resolved, Style::new().fg(TEXT)),
                ];
            }
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

/// Named-arg sub-candidate row: `↳ <key> · <value>` (shown when the input
/// is `<alias> `).
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

/// Shared block preset: rounded borders, subtle title. Shared with the
/// settings page.
pub(crate) fn main_block(title: &str) -> Block<'static> {
    main_block_line(Line::from(Span::styled(
        title.to_string(),
        Style::new().fg(ACCENT),
    )))
}

/// Same preset for a multi-span title (the input bar carries the brand mark).
fn main_block_line(title: Line<'static>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(BORDER))
        .title(title)
}

fn row_rect(width: u16, y: u16, height: u16) -> Rect {
    Rect {
        x: 0,
        y,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::record;
    use crate::storage::Store;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn bar_title_carries_the_brand_mark() {
        let text = draw_once(&state::new(Store::default(), false));
        assert!(
            text.contains("❯▌ xconsoler"),
            "input-bar title lost the mark: {text}"
        );
    }

    #[test]
    fn list_title_counts_kinds_and_hides_empty_args() {
        let hist = vec![Candidate::History { idx: 0 }];
        assert_eq!(list_title(&hist, 1), " matches · 1 history · 0 alias ");
        let mixed = vec![
            Candidate::History { idx: 0 },
            Candidate::Alias { name: "browser".to_string() },
            Candidate::Arg { alias: "browser".to_string(), key: "baidu".to_string() },
        ];
        assert_eq!(list_title(&mixed, 3), " matches · 1 history · 1 alias · 1 args ");
    }

    fn frame_text(terminal: &Terminal<TestBackend>) -> String {
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

    fn draw_once(app: &App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 14)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        frame_text(&terminal)
    }

    #[test]
    fn hidden_frame_shows_only_the_hint() {
        let mut app = state::new(Store::default(), false);
        app.visibility = Visibility::Hidden; // apps start Shown
        let text = draw_once(&app);
        assert!(text.contains("alt+d"));
        assert!(text.contains("hidden"));
        assert!(!text.contains("❯"));
    }

    #[test]
    fn hints_reflect_a_custom_wake_key() {
        let mut app = state::new(Store::default(), false);
        app.wake = crate::keyspec::parse("ctrl+g").unwrap();
        let shown_text = draw_once(&app);
        assert!(shown_text.contains("ctrl+g hide"));
        assert!(!shown_text.contains("alt+d"));
        app.visibility = Visibility::Hidden;
        let hidden_text = draw_once(&app);
        assert!(hidden_text.contains("ctrl+g wake"));
    }

    #[test]
    fn shown_frame_renders_bar_candidates_selection_and_status() {
        let mut store = Store::default();
        record(&mut store, "browser", "docs", 1);
        let mut app = state::new(store, false);
        app.cursor = 1; // selects the first alias row (browser)

        let text = draw_once(&app);
        assert!(text.contains("❯"));
        assert!(text.contains("↻")); // history row
        assert!(text.contains("★")); // alias rows
        assert!(text.contains("browser (br)"));
        assert!(text.contains("xdg-open")); // linux template of selected row
        assert!(text.contains("alt+d hide")); // key hints (status None)
    }

    #[test]
    fn status_line_replaces_key_hints() {
        let mut app = state::new(Store::default(), false);
        app.status = Some((true, "t ok: hello".to_string()));
        let ok = draw_once(&app);
        assert!(ok.contains("✓ t ok: hello"));
        app.status = Some((false, "boom".to_string()));
        let err = draw_once(&app);
        assert!(err.contains("✗ boom"));
    }

    #[test]
    fn colon_prefix_swaps_list_for_help_block() {
        let mut app = state::new(Store::default(), false);
        app.input = ":add t echo {input}".to_string();
        let text = draw_once(&app);
        assert!(text.contains(":add"));
        assert!(text.contains("{input}"));
        assert!(text.contains("@stdin"));
        assert!(!text.contains("★")); // list replaced by help
    }

    #[test]
    fn no_candidates_draws_no_list_but_keeps_status() {
        let mut app = state::new(Store::default(), false);
        app.input = "zzz".to_string();
        app.aliases.clear();
        let text = draw_once(&app);
        assert!(!text.contains("★"));
        assert!(!text.contains("matches"));
        assert!(text.contains("alt+d hide"));
        assert!(text.contains("❯ zzz"));
    }

    /// Background-policy guard: only the selection row and the cursor block
    /// may paint a background. Every other cell stays at the terminal default,
    /// which the host terminal profile colours (see `scripts/xc-bar`).
    #[test]
    fn only_selection_and_cursor_paint_backgrounds() {
        let mut store = Store::default();
        record(&mut store, "browser", "docs", 1);
        let mut app = state::new(store, false);
        app.cursor = 1; // selects the first alias row

        let mut terminal = Terminal::new(TestBackend::new(80, 14)).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let buf = terminal.backend().buffer();

        let mut sel_cells = 0;
        let mut cursor_cells = 0;
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                match buf[(x, y)].style().bg {
                    // `Clear` writes an explicit Reset - still terminal default.
                    None => {}
                    Some(ratatui::style::Color::Reset) => {}
                    Some(c) if c == SELECT_BG => sel_cells += 1,
                    Some(c) if c == CURSOR => cursor_cells += 1,
                    Some(other) => panic!("unexpected painted background {other:?} at ({x},{y})"),
                }
            }
        }
        assert_eq!(sel_cells, 78); // inner width: 80 minus the two borders
        assert_eq!(cursor_cells, 1); // one reverse-space cursor cell
    }
}
