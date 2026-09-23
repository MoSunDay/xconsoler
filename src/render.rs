//! Frame rendering. All colours are semantic consts at the top — a single
//! source of truth (the opencoder `theme.rs` pattern) so call sites never
//! hard-code raw `Color` values.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::alias;
use crate::keyspec;
use crate::matcher::Candidate;
use crate::platform::{self, Platform};
use crate::state::{self, App, Visibility, CANDIDATE_LIMIT};

// ── Semantic palette ────────────────────────────────────────────────────────
/// Primary accent: titles, prompt, alias labels, selection background.
const ACCENT: Color = Color::Cyan;
/// Success status.
const OK: Color = Color::Green;
/// Error status.
const ERR: Color = Color::Red;
/// Dimmed chrome: hidden hint, key hints, history markers.
const MUTED: Color = Color::DarkGray;
/// Secondary text: command templates, block titles.
const SUBTLE: Color = Color::Gray;
/// Primary text: user input, history inputs.
const TEXT: Color = Color::White;

const MAIN_TITLE: &str = " xconsoler ";

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
const HELP_LINES: [&str; 5] = [
    ":add <name>[,<short>...] <linux-cmd> // <macos-cmd>",
    ":del <name>",
    ":help — show this help",
    "{input} — your input, shell-quoted into the command",
    "@stdin — your input is piped to the command's stdin",
];

/// Draw one frame: the hidden one-liner or the full launcher layout.
pub fn draw(f: &mut Frame, app: &App) {
    // Blank the frame first: switching visibility or shrinking the list must
    // not leave stale cells behind (ratatui only diffs what is re-rendered).
    f.render_widget(Clear, f.area());
    match app.visibility {
        Visibility::Hidden => draw_hidden(f, app),
        Visibility::Shown => draw_shown(f, app),
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
        y = draw_help(f, width, y);
    } else if !cands.is_empty() {
        y = draw_list(f, app, &cands, width, y);
    }
    draw_status(f, app, width, y);
}

/// Full-width input bar: `❯ ` + input + a reverse-space cursor at the end.
fn draw_input_bar(f: &mut Frame, app: &App, width: u16) -> u16 {
    let rect = row_rect(width, 0, 3);
    let block = main_block(MAIN_TITLE);
    let inner = block.inner(rect);
    let budget = inner.width.saturating_sub(3) as usize; // "❯ " + cursor cell
    let shown: String = app.input.chars().take(budget).collect();
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("❯ ", Style::new().fg(ACCENT)),
            Span::styled(shown, Style::new().fg(TEXT)),
            Span::styled(" ", Style::new().bg(ACCENT).fg(ACCENT)),
        ]))
        .block(block),
        rect,
    );
    3
}

/// Candidate list under the bar. Returns the next free row.
fn draw_list(f: &mut Frame, app: &App, cands: &[Candidate], width: u16, y: u16) -> u16 {
    let rows = cands.len().min(CANDIDATE_LIMIT);
    let hist = cands
        .iter()
        .filter(|c| matches!(c, Candidate::History { .. }))
        .count();
    let title = format!(" matches · {hist} history · {} alias ", rows - hist);
    let rect = row_rect(width, y, rows as u16 + 2);
    let block = main_block(&title);
    let inner = block.inner(rect);
    let sel_style = Style::new().bg(ACCENT).fg(Color::Black);
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
            };
            segments_line(segs, inner.width as usize, i == cursor, sel_style)
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(block), rect);
    rect.y + rect.height
}

/// `:command` help block replacing the list. Returns the next free row.
fn draw_help(f: &mut Frame, width: u16, y: u16) -> u16 {
    let rect = row_rect(width, y, HELP_LINES.len() as u16 + 2);
    let block = main_block(HELP_TITLE);
    let lines: Vec<Line> = HELP_LINES
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

/// Alias row: `★ <label> · <command template for the current platform>`.
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

/// Lay out styled segments on one row: truncation is unicode-safe (whole
/// `char`s only, no byte slicing); a selected row gets an accent background
/// padded across the full inner width.
fn segments_line(
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

/// Shared block preset: rounded borders, subtle title.
fn main_block(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::styled(title, Style::new().fg(ACCENT)))
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
}
