//! Rendering for the `/settings` page: a full-screen alias table
//! (`name | triggers | linux | macos | shortcuts`) with expandable
//! trigger/shortcut rows and the wizard's bottom input line.
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

/// Human title such as `new alias (2/4)`.
fn form_title(f: &Form) -> String {
    let n = settings_form::step_count(f);
    match &f.purpose {
        Purpose::NewAlias => format!("new alias ({}/{})", f.step + 1, n),
        Purpose::NewShortcut { .. } => format!("new shortcut ({}/{})", f.step + 1, n),
        Purpose::NewTrigger { .. } => format!("new trigger ({}/{})", f.step + 1, n),
        Purpose::EditCommand { alias } => {
            format!("edit commands for '{alias}' ({}/{})", f.step + 1, n)
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
        (Purpose::NewAlias, 0) => "name".to_string(),
        (Purpose::NewAlias, 1) => "triggers (comma-separated, empty ok)".to_string(),
        (Purpose::NewAlias, 2) => "linux command — use {input} where the input goes".to_string(),
        (Purpose::NewAlias, 3) => "macos command (empty = same as linux)".to_string(),
        (Purpose::NewShortcut { alias } | Purpose::EditShortcut { alias, .. }, 0) => {
            format!("shortcut key for '{alias}' (one word)")
        }
        (Purpose::NewShortcut { .. } | Purpose::EditShortcut { .. }, 1) => {
            "shortcut value (spaces allowed)".to_string()
        }
        (Purpose::NewTrigger { .. } | Purpose::EditTrigger { .. }, 0) => {
            "trigger (one word, a-z 0-9 - _)".to_string()
        }
        (Purpose::EditCommand { .. }, 0) => {
            "linux command — use {input} where the input goes".to_string()
        }
        (Purpose::EditCommand { .. }, 1) => "macos command (empty = same as linux)".to_string(),
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
    // linux and macos share the leftover width (fixed overhead: 4 gaps of 2
    // chars + the "shortcuts" header). Command text is truncated to this.
    let cmd_w = width.saturating_sub(name_w + sc_w + 17) / 2;
    lines.push(header_line(name_w, sc_w, cmd_w));

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

fn header_line(name_w: usize, sc_w: usize, cmd_w: usize) -> Line<'static> {
    Line::from(Span::styled(
        format!(
            "{:<nw$}  {:<sw$}  {:<lw$}  {:<mw$}  shortcuts",
            "name",
            "triggers",
            "linux",
            "macos",
            nw = name_w,
            sw = sc_w,
            lw = cmd_w,
            mw = cmd_w
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
            (pad(&d.triggers.join(","), sc_w), Style::new().fg(SUBTLE)),
            ("  ".to_string(), Style::new()),
            (
                // truncate then pad: both command columns stay aligned even
                // when one command is much shorter than the other.
                pad(&truncate(d.linux.as_deref().unwrap_or("—"), cmd_w), cmd_w),
                Style::new().fg(SUBTLE),
            ),
            ("  ".to_string(), Style::new()),
            (
                // truncate then pad: both command columns stay aligned even
                // when one command is much shorter than the other.
                pad(&truncate(d.macos.as_deref().unwrap_or("—"), cmd_w), cmd_w),
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

    /// Same page on the real window width (xterm geometry is 140x14), where
    /// the whole footer hint line fits.
    fn draw_wide(st: &Settings, aliases: &[AliasDef]) -> String {
        draw_at(140, 14, st, aliases)
    }

    /// The page at an arbitrary terminal size (the deployed bar is 52 wide).
    fn draw_at(w: u16, h: u16, st: &Settings, aliases: &[AliasDef]) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| draw(f, st, aliases)).unwrap();
        frame_text(&terminal)
    }

    fn t_store() -> (Store, Vec<AliasDef>) {
        let mut store = Store::default();
        store.aliases.push(AliasDef {
            name: "t".to_string(),
            triggers: vec!["tt".to_string()],
            linux: Some("printf %s {input}".to_string()),
            macos: Some("printf %s {input}".to_string()),
            shortcuts: [("baidu".to_string(), "https://www.baidu.com".to_string())]
                .into_iter()
                .collect(),
        });
        let aliases = settings::view(&store);
        (store, aliases)
    }

    #[test]
    fn command_columns_line_up_with_the_header() {
        let mut store = Store::default();
        store.aliases.push(AliasDef {
            name: "f".to_string(),
            triggers: vec![],
            linux: Some("ls".to_string()),
            macos: Some("open -a Finder".to_string()),
            shortcuts: Default::default(),
        });
        let aliases = settings::view(&store);
        let text = draw_wide(&settings::new(), &aliases);
        let lines: Vec<&str> = text.lines().collect();
        let header = lines.iter().find(|l| l.contains("macos")).expect("header");
        let col = header.find("macos").expect("macos column");
        let row = lines
            .iter()
            .find(|l| l.contains("open -a Finder"))
            .expect("alias row");
        assert_eq!(
            row.find("open -a Finder").unwrap(),
            col,
            "a short linux command must not shift the macos column"
        );
    }

    #[test]
    fn list_shows_table_and_hints() {
        let (_store, aliases) = t_store();
        let text = draw_once(&settings::new(), &aliases);
        assert!(text.contains("settings"));
        assert!(text.contains("br"));
        assert!(text.contains("cd"));
        assert!(text.contains("printf %s {input}"));
        assert!(text.contains("n new alias"));
        assert!(text.contains("e edit row"));
    }

    #[test]
    fn footer_lists_every_key_including_the_new_ones() {
        let (_store, aliases) = t_store();
        let text = draw_wide(&settings::new(), &aliases);
        for hint in [
            "↑↓/jk move",
            "Enter/→ expand",
            "← collapse",
            "n new alias",
            "e edit row",
            "s add shortcut",
            "t add trigger",
            "d delete",
            "q/Esc back",
        ] {
            assert!(text.contains(hint), "footer is missing {hint}");
        }
    }

    #[test]
    fn hints_wrap_instead_of_clipping_on_a_narrow_bar() {
        let (_store, aliases) = t_store();
        // the deployed bar: 52 columns, where one hint line cannot hold all
        // nine segments and the old fixed string lost the last four.
        let text = draw_at(52, 16, &settings::new(), &aliases);
        for seg in HINT_SEGMENTS {
            assert!(text.contains(seg), "the 52-column bar clips {seg}");
        }
        for line in text.lines() {
            assert!(
                line.chars().count() <= 52,
                "a rendered line overflows the bar: {line}"
            );
        }
    }

    #[test]
    fn hints_stay_on_one_line_when_wide() {
        let (_store, aliases) = t_store();
        let text = draw_wide(&settings::new(), &aliases);
        let line = text
            .lines()
            .find(|l| l.contains("q/Esc back"))
            .expect("hint line");
        for seg in HINT_SEGMENTS {
            assert!(
                line.contains(seg),
                "140 columns must keep {seg} on one line"
            );
        }
    }

    #[test]
    fn hint_lines_pack_segments_to_the_width() {
        // narrow bar: three lines; the real window: a single line
        assert_eq!(hint_lines(52).len(), 3);
        assert_eq!(hint_lines(140).len(), 1);
        // degenerate widths still yield lines and never panic
        assert!(!hint_lines(0).is_empty());
        assert!(!hint_lines(1).is_empty());
        for width in [0usize, 1, 16, 50, 52, 78, 138, 300] {
            let lines = hint_lines(width);
            let joined = lines.join("");
            for seg in HINT_SEGMENTS {
                assert!(joined.contains(seg), "width {width} drops {seg}");
                // no segment is ever split mid-word across two lines
                assert!(
                    lines.iter().any(|l| l.contains(seg)),
                    "width {width} splits {seg}"
                );
            }
            if width >= 16 {
                // 16 is the widest single segment plus its two padding spaces,
                // so every line fits; below that the widget has to clip.
                for line in &lines {
                    assert!(
                        line.chars().count() <= width,
                        "width {width} renders an over-wide line: {line}"
                    );
                }
            }
        }
    }

    #[test]
    fn table_shows_the_macos_column() {
        let (_store, aliases) = t_store();
        let text = draw_wide(&settings::new(), &aliases);
        assert!(text.contains("name"));
        assert!(text.contains("triggers"));
        assert!(text.contains("linux"));
        assert!(text.contains("macos"));
        assert!(text.contains("shortcuts"));
        // t has an explicit macos command, shown next to its linux one
        // t's row carries the same command in both command columns
        let row = text
            .lines()
            .find(|l| l.contains("printf %s {input}"))
            .expect("t row");
        assert_eq!(row.matches("printf %s {input}").count(), 2, "got: {row}");
    }

    #[test]
    fn missing_macos_command_renders_a_dash() {
        let mut store = Store::default();
        store.aliases.push(AliasDef {
            name: "t".to_string(),
            triggers: vec![],
            linux: Some("printf %s {input}".to_string()),
            macos: None,
            shortcuts: Default::default(),
        });
        let aliases = settings::view(&store);
        let text = draw_wide(&settings::new(), &aliases);
        let row = text
            .lines()
            .find(|l| l.contains("printf %s {input}"))
            .expect("t row");
        assert!(row.contains('—'), "no macos command shows a dash: {row}");
    }

    #[test]
    fn long_commands_are_truncated_to_keep_the_table_readable() {
        let long = "x".repeat(60);
        let mut store = Store::default();
        store.aliases.push(AliasDef {
            name: "t".to_string(),
            triggers: vec![],
            linux: Some(long.clone()),
            macos: Some(long.clone()),
            shortcuts: Default::default(),
        });
        let aliases = settings::view(&store);
        let text = draw_wide(&settings::new(), &aliases);
        assert!(!text.contains(&long), "the raw command is not drawn");
        assert!(text.contains('…'), "truncation is visible");
    }

    #[test]
    fn expanded_alias_lists_trigger_rows() {
        let (_store, aliases) = t_store();
        let mut st = settings::new();
        st.cursor = 2;
        st.expanded = Some(2);
        let text = draw_once(&st, &aliases);
        assert!(text.contains("↳ trigger: tt"));
    }

    #[test]
    fn expanded_alias_shows_indented_shortcuts() {
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
        form.caret = 0; // direct fixture: a caret at the start hides no chars
        st.form = Some(form);
        let text = draw_once(&st, &aliases);
        assert!(text.contains("new alias (3/4)"));
        assert!(text.contains("{input}"));
        assert!(text.contains("❯ █printf"), "caret block precedes the text");
        assert!(text.contains("Esc cancel"));
    }

    #[test]
    fn form_caret_in_the_middle_keeps_every_character() {
        let (_store, aliases) = t_store();
        let mut st = settings::new();
        let mut form = settings_form::new_alias();
        form.step = 2;
        form.input = "printf %s {input}".to_string();
        form.caret = 7; // "printf " | "%s {input}"
        st.form = Some(form);
        let text = draw_wide(&st, &aliases);
        assert!(
            text.contains("❯ printf █%s {input}"),
            "the caret block sits before the char under it: {text}"
        );
    }

    #[test]
    fn form_window_keeps_a_long_prefilled_value_visible() {
        let (_store, aliases) = t_store();
        let mut st = settings::new();
        let long = "x".repeat(400);
        let mut form = settings_form::new_edit_command("t", Some(&long), None);
        form.caret = form.input.chars().count();
        st.form = Some(form);
        let text = draw_wide(&st, &aliases);
        let line = text.lines().find(|l| l.contains('❯')).expect("input line");
        let content = line
            .strip_prefix('│')
            .unwrap_or(line)
            .strip_suffix('│')
            .unwrap_or(line)
            .trim_end();
        // inner width 138 = " ❯ " + 134 text cells + 1 caret cell
        assert_eq!(content.chars().count(), 138, "got: {content}");
        assert!(content.starts_with(" ❯ xxx"), "text before the caret shows");
        assert!(content.ends_with('█'), "the caret cell is at the end");
        assert_eq!(
            content.matches('x').count(),
            134,
            "the whole budget before the caret is used"
        );
    }

    #[test]
    fn long_form_input_fits_the_narrow_bar() {
        let (_store, aliases) = t_store();
        let mut st = settings::new();
        let mut form = settings_form::new_alias();
        form.input = "y".repeat(200);
        form.caret = 150;
        st.form = Some(form);
        let text = draw_at(52, 16, &st, &aliases);
        for line in text.lines() {
            assert!(
                line.chars().count() <= 52,
                "a rendered line overflows the bar: {line}"
            );
        }
        let line = text.lines().find(|l| l.contains('❯')).expect("input line");
        let content = line
            .strip_prefix('│')
            .unwrap_or(line)
            .strip_suffix('│')
            .unwrap_or(line);
        // inner width 50 = " ❯ " + 46 text cells + 1 caret cell
        assert_eq!(content.chars().count(), 50, "got: {content}");
        assert!(content.starts_with(" ❯ yyy"), "got: {content}");
        assert!(
            content.ends_with('█'),
            "the caret block is at the end: {content}"
        );
        // 50 cells - " ❯ " (3) - caret block (1): the 46 y's before it.
        assert_eq!(
            content.matches('y').count(),
            46,
            "the window shows the chars before the caret only: {content}"
        );
    }

    #[test]
    fn edit_command_form_shows_the_prefill_and_title() {
        let (_store, aliases) = t_store();
        let mut st = settings::new();
        let form = settings_form::new_edit_command("t", Some("printf %s {input}"), Some("open"));
        st.form = Some(form);
        let text = draw_wide(&st, &aliases);
        assert!(text.contains("edit commands for 't' (1/2)"));
        assert!(text.contains("linux command"));
        assert!(text.contains("❯ printf %s {input}"), "step 1 is prefilled");
        assert!(text.contains("Enter next/accept"));
    }

    #[test]
    fn shortcut_form_shows_the_key_then_value_steps() {
        let (_store, aliases) = t_store();
        let mut st = settings::new();
        let mut form = settings_form::new_shortcut("t");
        st.form = Some(form.clone());
        let text = draw_wide(&st, &aliases);
        assert!(text.contains("new shortcut (1/2)"));
        assert!(text.contains("shortcut key for 't' (one word)"));

        form.step = 1;
        st.form = Some(form);
        let text = draw_wide(&st, &aliases);
        assert!(text.contains("new shortcut (2/2)"));
        assert!(text.contains("shortcut value (spaces allowed)"));
    }

    #[test]
    fn trigger_form_shows_the_one_step_title() {
        let (_store, aliases) = t_store();
        let mut st = settings::new();
        st.form = Some(settings_form::new_trigger("t"));
        let text = draw_wide(&st, &aliases);
        assert!(text.contains("new trigger (1/1)"));
        assert!(text.contains("trigger (one word"));
    }

    #[test]
    fn edit_shortcut_form_shows_the_key_then_value_steps() {
        let (_store, aliases) = t_store();
        let mut st = settings::new();
        let mut form = settings_form::new_edit_shortcut("t", "baidu", "https://www.baidu.com");
        st.form = Some(form.clone());
        let text = draw_wide(&st, &aliases);
        assert!(text.contains("edit shortcut t.baidu (1/2)"));
        assert!(text.contains("shortcut key for 't' (one word)"));
        assert!(text.contains("❯ baidu"), "the current key is prefilled");

        form.step = 1;
        // `advance` prefills step 1 with the current value (see `prefill`).
        form.input = "https://www.baidu.com".to_string();
        form.caret = form.input.chars().count();
        st.form = Some(form);
        let text = draw_wide(&st, &aliases);
        assert!(text.contains("edit shortcut t.baidu (2/2)"));
        assert!(text.contains("shortcut value (spaces allowed)"));
        assert!(
            text.contains("❯ https://www.baidu.com"),
            "the current value is prefilled"
        );
    }

    #[test]
    fn edit_trigger_form_shows_the_rename_title_and_prompt() {
        let (_store, aliases) = t_store();
        let mut st = settings::new();
        st.form = Some(settings_form::new_edit_trigger("t", "tt"));
        let text = draw_wide(&st, &aliases);
        assert!(text.contains("edit trigger 't' (1/1)"));
        assert!(text.contains("trigger (one word"));
        assert!(text.contains("❯ tt"), "the current word is prefilled");
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
