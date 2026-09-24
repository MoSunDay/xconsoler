//! Rendering for the `/settings` page: a full-screen alias table with an
//! expandable named-args column and the wizard's bottom input line.
//!
//! Text-only layout (no Table widget): each row is styled segments laid on
//! one line, reusing the truncation/selection helpers from `crate::render`.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::alias::AliasDef;
use crate::render::{main_block, segments_line};
use crate::settings::{self, Row, Settings};
use crate::settings_form::{self, Form, Purpose};
use crate::theme::{ACCENT, ERR, MUTED, OK, SELECT_BG, SUBTLE, TEXT};

const TITLE_BLOCK: &str = " settings ";
const HINTS_LIST: &str =
    " ↑↓/jk move · Enter/→ expand · n new alias · a add arg · d delete · q/Esc back ";
const HINTS_FORM: &str = " Enter next · Ctrl+U clear · Esc cancel ";

/// Human title such as `new alias (2/4)`.
fn form_title(f: &Form) -> String {
    let n = settings_form::step_count(f);
    match &f.purpose {
        Purpose::NewAlias => format!("new alias ({}/{})", f.step + 1, n),
        Purpose::NewArg { alias } => format!("new arg for '{alias}' ({}/{})", f.step + 1, n),
    }
}

/// Prompt for the field currently being edited.
fn form_prompt(f: &Form) -> String {
    match (&f.purpose, f.step) {
        (Purpose::NewAlias, 0) => "name".to_string(),
        (Purpose::NewAlias, 1) => "shortcuts (comma-separated, empty ok)".to_string(),
        (Purpose::NewAlias, 2) => "linux command — use {input} where the input goes".to_string(),
        (Purpose::NewAlias, 3) => "macos command (empty = same as linux)".to_string(),
        (Purpose::NewArg { alias }, 0) => format!("arg key for '{alias}' (one word)"),
        (Purpose::NewArg { .. }, 1) => "arg value (spaces allowed)".to_string(),
        _ => "?".to_string(),
    }
}

/// Draw the whole settings page (the launcher bar is not drawn at all).
pub fn draw(f: &mut Frame, st: &Settings, aliases: &[AliasDef]) {
    let area = f.area();
    let block = main_block(TITLE_BLOCK);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = settings::rows(aliases, st.expanded);
    let footer_h = if st.form.is_some() { 4 } else { 2 };
    let width = inner.width as usize;

    let mut lines: Vec<Line> = Vec::new();
    let (name_w, sc_w) = column_widths(aliases, width);
    let cmd_w = width.saturating_sub(name_w + sc_w + 4 + 6);
    lines.push(header_line(name_w, sc_w));

    // scroll window keeping the cursor visible
    let table_h = (inner.height as usize).saturating_sub(footer_h + 1);
    let off = if table_h == 0 {
        0
    } else {
        st.cursor
            .saturating_sub(table_h - 1)
            .min(rows.len().saturating_sub(table_h))
    };
    let sel_style = Style::new().bg(SELECT_BG).fg(TEXT);
    for (i, row) in rows.iter().enumerate().skip(off).take(table_h) {
        let selected = i == st.cursor;
        let segs = match row {
            Row::Alias { idx } => alias_segments(aliases, *idx, name_w, sc_w, cmd_w),
            Row::Arg { alias, key } => arg_segments(aliases, *alias, key, width),
        };
        lines.push(segments_line(segs, width, selected, sel_style));
    }

    match st.form.as_ref() {
        None => {
            lines.push(status_line(&st.status));
            lines.push(Line::from(Span::styled(HINTS_LIST, Style::new().fg(MUTED))));
        }
        Some(form) => {
            lines.push(Line::from(vec![
                Span::styled(" ", Style::new()),
                Span::styled(form_title(form), Style::new().fg(ACCENT)),
                Span::styled(" — ", Style::new().fg(MUTED)),
                Span::styled(form_prompt(form), Style::new().fg(TEXT)),
            ]));
            lines.push(match form.error.as_deref() {
                Some(e) => Line::from(vec![
                    Span::styled(" ✗ ", Style::new().fg(ERR)),
                    Span::styled(e.to_string(), Style::new().fg(ERR)),
                ]),
                None => Line::from(Span::styled(HINTS_FORM, Style::new().fg(MUTED))),
            });
            lines.push(Line::from(vec![
                Span::styled(" ❯ ", Style::new().fg(ACCENT)),
                Span::styled(form.input.clone(), Style::new().fg(TEXT)),
                Span::styled("█", Style::new().fg(ACCENT)),
            ]));
            lines.push(Line::from(Span::styled(HINTS_FORM, Style::new().fg(MUTED))));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// (name, shortcuts) column widths from the data, capped so the command
/// column keeps most of the line.
fn column_widths(aliases: &[AliasDef], width: usize) -> (usize, usize) {
    let name_w = aliases
        .iter()
        .map(|d| d.name.chars().count())
        .chain([4])
        .max()
        .unwrap_or(4)
        .min(20)
        .min(width / 3);
    let sc_w = aliases
        .iter()
        .map(|d| d.shortcuts.join(",").chars().count())
        .chain([9])
        .max()
        .unwrap_or(9)
        .min(18)
        .min(width / 4);
    (name_w, sc_w)
}

fn header_line(name_w: usize, sc_w: usize) -> Line<'static> {
    Line::from(Span::styled(
        format!(
            "{:<nw$}  {:<sw$}  linux command  args",
            "name",
            "shortcuts",
            nw = name_w,
            sw = sc_w
        ),
        Style::new().fg(MUTED),
    ))
}

type Segs = Vec<(String, Style)>;

fn alias_segments(
    aliases: &[AliasDef],
    idx: usize,
    name_w: usize,
    sc_w: usize,
    cmd_w: usize,
) -> Segs {
    match aliases.get(idx) {
        Some(d) => vec![
            (pad(&d.name, name_w), Style::new().fg(ACCENT)),
            ("  ".to_string(), Style::new()),
            (pad(&d.shortcuts.join(","), sc_w), Style::new().fg(SUBTLE)),
            ("  ".to_string(), Style::new()),
            (truncate(d.linux.as_deref().unwrap_or("—"), cmd_w), Style::new().fg(SUBTLE)),
            ("  ".to_string(), Style::new()),
            (d.args.len().to_string(), Style::new().fg(MUTED)),
        ],
        None => vec![("…".to_string(), Style::new().fg(MUTED))],
    }
}

/// Indented `key → value` row under the expanded alias.
fn arg_segments(aliases: &[AliasDef], alias: usize, key: &str, width: usize) -> Segs {
    let value = aliases
        .get(alias)
        .and_then(|d| d.args.get(key).cloned())
        .unwrap_or_else(|| "…".to_string());
    let text = format!("  {key} → {value}");
    vec![(truncate(&text, width), Style::new().fg(SUBTLE))]
}

fn status_line(status: &Option<(bool, String)>) -> Line<'static> {
    match status {
        Some((true, msg)) => Line::from(vec![
            Span::styled("✓ ", Style::new().fg(OK)),
            Span::styled(msg.clone(), Style::new().fg(OK)),
        ]),
        Some((false, msg)) => Line::from(vec![
            Span::styled("✗ ", Style::new().fg(ERR)),
            Span::styled(msg.clone(), Style::new().fg(ERR)),
        ]),
        None => Line::from(""),
    }
}

fn pad(s: &str, w: usize) -> String {
    format!("{s:<w$}")
}

/// Unicode-safe truncation to at most `w` chars (adds `…` when cut).
fn truncate(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        return s.to_string();
    }
    let cut: String = s.chars().take(w.saturating_sub(1)).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_form;
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

    fn draw_once(st: &Settings, aliases: &[AliasDef]) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal.draw(|f| draw(f, st, aliases)).unwrap();
        frame_text(&terminal)
    }

    fn t_store() -> (Store, Vec<AliasDef>) {
        let mut store = Store::default();
        store.aliases.push(AliasDef {
            name: "t".to_string(),
            shortcuts: vec!["tt".to_string()],
            linux: Some("printf %s {input}".to_string()),
            macos: None,
            args: [("baidu".to_string(), "https://www.baidu.com".to_string())]
                .into_iter()
                .collect(),
            builtin: false,
        });
        let aliases = settings::view(&store);
        (store, aliases)
    }

    #[test]
    fn list_shows_table_and_hints() {
        let (_store, aliases) = t_store();
        let text = draw_once(&settings::new(), &aliases);
        assert!(text.contains("settings"));
        assert!(text.contains("browser"));
        assert!(text.contains("clipboard"));
        assert!(text.contains("printf %s {input}"));
        assert!(text.contains("n new alias"));
        assert!(text.contains("q/Esc back"));
    }

    #[test]
    fn expanded_alias_shows_indented_args() {
        let (_store, aliases) = t_store();
        let mut st = settings::new();
        st.cursor = 2;
        st.expanded = Some(2);
        let text = draw_once(&st, &aliases);
        assert!(text.contains("baidu → https://www.baidu.com"));
    }

    #[test]
    fn form_shows_prompt_and_input() {
        let (_store, aliases) = t_store();
        let mut st = settings::new();
        let mut form = settings_form::new_alias();
        form.step = 2;
        form.input = "printf".to_string();
        st.form = Some(form);
        let text = draw_once(&st, &aliases);
        assert!(text.contains("new alias (3/4)"));
        assert!(text.contains("{input}"));
        assert!(text.contains("❯ printf"));
        assert!(text.contains("Esc cancel"));
    }

    #[test]
    fn form_error_is_inline() {
        let (_store, aliases) = t_store();
        let mut st = settings::new();
        let mut form = settings_form::new_alias();
        form.error = Some("name cannot be empty".to_string());
        st.form = Some(form);
        let text = draw_once(&st, &aliases);
        assert!(text.contains("✗ name cannot be empty"));
    }

    #[test]
    fn truncate_and_pad_helpers() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 8), "abc");
        assert_eq!(pad("ab", 4), "ab  ");
    }
}
