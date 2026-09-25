//! Rendering for the `/settings` page: a full-screen alias table
//! (`name | triggers | <platform> | shortcuts`, one command column for the
//! platform the page was opened on) with expandable trigger/shortcut rows and
//! the wizard's bottom input line.
//!
//! Text-only layout (no Table widget): each row is styled segments laid on
//! one line, reusing the truncation/selection helpers from `crate::render`.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::alias::{self, AliasDef};
use crate::platform::{self, Platform};
use crate::render::{main_block, segments_line};
use crate::settings::{self, Row, Settings};
use crate::settings_form::{self, Form, Purpose};
use crate::textedit;
use crate::theme::{ACCENT, ERR, MUTED, OK, SELECT_BG, SUBTLE, TEXT};

const TITLE_BLOCK: &str = " settings ";
/// List-screen footer hints, in display order. They are wrapped onto as many
/// lines as the terminal width needs (see [`hint_lines`]) instead of being
/// clipped: the deployed bar is only 52 columns wide, where a single fixed
/// hint string hides the management keys at the end.
const HINT_SEGMENTS: [&str; 9] = [
    "↑↓/jk move",
    "Enter/→ expand",
    "← collapse",
    "n new alias",
    "e edit row",
    "s add shortcut",
    "t add trigger",
    "d delete",
    "q/Esc back",
];
/// Separator between two hint segments (3 chars).
const HINT_SEP: &str = " · ";
const HINTS_FORM: &str = " Enter next/accept · Ctrl+A/E/W/U/K edit · Esc cancel ";

/// Pack [`HINT_SEGMENTS`] onto lines that never exceed `width` chars.
///
/// Narrow-bar rationale: on the deployed 52-column terminal a single hint
/// string is clipped, so the last keys (`s add shortcut`, `a add arg`,
/// `d delete`, `q/Esc back`) are invisible. Wrapping instead of clipping keeps
/// every key reachable; the footer just grows and the table shrinks.
///
/// Packing is greedy: segments keep their order and are joined by
/// [`HINT_SEP`], never split mid-word. Every line gets one leading and one
/// trailing space, so its char count stays within `width`. At least one line
/// is always returned; a segment wider than the whole width still lands on a
/// line of its own (the widget clips it) so nothing is dropped silently.
fn hint_lines(width: usize) -> Vec<String> {
    let sep_chars = HINT_SEP.chars().count();
    let budget = width.saturating_sub(2);
    let mut out: Vec<String> = Vec::new();
    let mut line: Vec<&str> = Vec::new();
    let mut used = 0usize;
    for seg in HINT_SEGMENTS {
        let seg_chars = seg.chars().count();
        // width of the joined segments once `seg` is appended; the first one
        // of a line needs no separator.
        let needed = if line.is_empty() {
            seg_chars
        } else {
            used + sep_chars + seg_chars
        };
        if !line.is_empty() && needed > budget {
            out.push(format!(" {} ", line.join(HINT_SEP)));
            line.clear();
        }
        // after a flush the segment opens a fresh line, so it only counts its
        // own chars; otherwise `needed` is the joined width.
        used = if line.is_empty() { seg_chars } else { needed };
        line.push(seg);
    }
    if !line.is_empty() {
        out.push(format!(" {} ", line.join(HINT_SEP)));
    }
    if out.is_empty() {
        out.push(String::from(" "));
    }
    out
}

/// Human title such as `new alias (2/3)`.
fn form_title(f: &Form) -> String {
    let n = settings_form::step_count(f);
    match &f.purpose {
        Purpose::NewAlias(_) => format!("new alias ({}/{})", f.step + 1, n),
        Purpose::NewShortcut { .. } => format!("new shortcut ({}/{})", f.step + 1, n),
        Purpose::NewTrigger { .. } => format!("new trigger ({}/{})", f.step + 1, n),
        Purpose::EditCommand { alias, .. } => {
            format!("edit command for '{alias}' ({}/{})", f.step + 1, n)
        }
        Purpose::EditShortcut { alias, old_key } => {
            format!("edit shortcut {alias}.{old_key} ({}/{})", f.step + 1, n)
        }
        Purpose::EditTrigger { alias, .. } => {
            format!("edit trigger '{alias}' ({}/{})", f.step + 1, n)
        }
    }
}

/// Prompt for the field currently being edited.
fn form_prompt(f: &Form) -> String {
    match (&f.purpose, f.step) {
        (Purpose::NewAlias(_), 0) => "name".to_string(),
        (Purpose::NewAlias(_), 1) => "triggers (comma-separated, empty ok)".to_string(),
        // One command step, named after the platform being configured.
        (Purpose::NewAlias(platform), 2) | (Purpose::EditCommand { platform, .. }, 0) => {
            format!(
                "{} command — use {{input}} where the input goes",
                platform::name(*platform)
            )
        }
        (Purpose::NewShortcut { alias } | Purpose::EditShortcut { alias, .. }, 0) => {
            format!("shortcut key for '{alias}' (one word)")
        }
        (Purpose::NewShortcut { .. } | Purpose::EditShortcut { .. }, 1) => {
            "shortcut value (spaces allowed)".to_string()
        }
        (Purpose::NewTrigger { .. } | Purpose::EditTrigger { .. }, 0) => {
            "trigger (one word, a-z 0-9 - _)".to_string()
        }
        _ => "?".to_string(),
    }
}

/// Bottom input line: `" ❯ "`, the visible window around the caret (which
/// keeps the whole line inside `width`), and one caret cell. The caret is a
/// solid block *before* the char at the caret, so a wide char keeps both its
/// columns and every character stays on screen.
fn form_input_line(f: &Form, width: usize) -> Line<'static> {
    let (before, at, after) = textedit::window(&f.input, f.caret, width.saturating_sub(3));
    let mut spans = vec![
        Span::styled(" ❯ ", Style::new().fg(ACCENT)),
        Span::styled(before, Style::new().fg(TEXT)),
        Span::styled("█", Style::new().fg(ACCENT)),
    ];
    if let Some(c) = at {
        spans.push(Span::styled(c.to_string(), Style::new().fg(TEXT)));
    }
    spans.push(Span::styled(after, Style::new().fg(TEXT)));
    Line::from(spans)
}

/// Draw the whole settings page (the launcher bar is not drawn at all).
pub fn draw(f: &mut Frame, st: &Settings, aliases: &[AliasDef]) {
    let area = f.area();
    let block = main_block(TITLE_BLOCK);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = settings::rows(aliases, st.expanded);
    let width = inner.width as usize;
    // the list footer is one status line plus as many hint lines as the width
    // needs, so the table area follows the real footer height.
    let hints = hint_lines(width);
    let footer_h = if st.form.is_some() {
        4
    } else {
        1 + hints.len()
    };

    let mut lines: Vec<Line> = Vec::new();
    let (name_w, sc_w) = column_widths(aliases, width);
    // One command column for the page's platform (fixed overhead: 3 gaps of
    // 2 chars + the "shortcuts" header). Command text is truncated to this.
    let cmd_w = width.saturating_sub(name_w + sc_w + 15);
    lines.push(header_line(name_w, sc_w, cmd_w, st.platform));

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
            Row::Alias { idx } => alias_segments(aliases, *idx, name_w, sc_w, cmd_w, st.platform),
            Row::Trigger { alias, idx } => trigger_segments(aliases, *alias, *idx, width),
            Row::Shortcut { alias, key } => shortcut_segments(aliases, *alias, key, width),
        };
        lines.push(segments_line(segs, width, selected, sel_style));
    }

    match st.form.as_ref() {
        None => {
            lines.push(status_line(&st.status));
            for hint in &hints {
                lines.push(Line::from(Span::styled(
                    hint.clone(),
                    Style::new().fg(MUTED),
                )));
            }
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
            lines.push(form_input_line(form, width));
            lines.push(Line::from(Span::styled(HINTS_FORM, Style::new().fg(MUTED))));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// (name, triggers) column widths from the data, capped so the command
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
        .map(|d| d.triggers.join(",").chars().count())
        .chain([9])
        .max()
        .unwrap_or(9)
        .min(18)
        .min(width / 4);
    (name_w, sc_w)
}

fn header_line(name_w: usize, sc_w: usize, cmd_w: usize, platform: Platform) -> Line<'static> {
    Line::from(Span::styled(
        format!(
            "{:<nw$}  {:<sw$}  {:<cw$}  shortcuts",
            "name",
            "triggers",
            platform::name(platform),
            nw = name_w,
            sw = sc_w,
            cw = cmd_w
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
    platform: Platform,
) -> Segs {
    match aliases.get(idx) {
        Some(d) => vec![
            (pad(&d.name, name_w), Style::new().fg(ACCENT)),
            ("  ".to_string(), Style::new()),
            (pad(&d.triggers.join(","), sc_w), Style::new().fg(SUBTLE)),
            ("  ".to_string(), Style::new()),
            (
                // Only this platform's stored command: the other platform is
                // never shown (no cross-platform fallback here). Truncate
                // then pad, so the column stays aligned with the header.
                pad(
                    &truncate(alias::platform_command(d, platform).unwrap_or("—"), cmd_w),
                    cmd_w,
                ),
                Style::new().fg(SUBTLE),
            ),
            ("  ".to_string(), Style::new()),
            (d.shortcuts.len().to_string(), Style::new().fg(MUTED)),
        ],
        None => vec![("…".to_string(), Style::new().fg(MUTED))],
    }
}

/// Indented trigger row under the expanded alias: `↳ trigger: tt`.
fn trigger_segments(aliases: &[AliasDef], alias: usize, idx: usize, width: usize) -> Segs {
    let trigger = aliases
        .get(alias)
        .and_then(|d| d.triggers.get(idx).cloned())
        .unwrap_or_else(|| "…".to_string());
    let text = format!("  ↳ trigger: {trigger}");
    vec![(truncate(&text, width), Style::new().fg(ACCENT))]
}

/// Indented `key → value` concrete shortcut row under the expanded alias.
fn shortcut_segments(aliases: &[AliasDef], alias: usize, key: &str, width: usize) -> Segs {
    let value = aliases
        .get(alias)
        .and_then(|d| d.shortcuts.get(key).cloned())
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
#[path = "settings_view/test_util.rs"]
mod test_util;

#[cfg(test)]
#[path = "settings_view/list_tests.rs"]
mod list_tests;

#[cfg(test)]
#[path = "settings_view/form_tests.rs"]
mod form_tests;
