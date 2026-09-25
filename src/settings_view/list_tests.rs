//! List/table rendering tests for `/settings`: columns, footer hints and
//! expandable rows. Split out of `settings_view.rs` so every file stays within
//! the size budget; the wizard-chrome tests live in
//! `settings_view/form_tests.rs`.

use super::test_util::*;
use super::*;
use crate::storage::Store;

#[test]
fn command_column_lines_up_with_the_header() {
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
    let header = lines.iter().find(|l| l.contains("linux")).expect("header");
    let col = header.find("linux").expect("linux column");
    let row = lines.iter().find(|l| l.contains("ls")).expect("alias row");
    assert_eq!(
        row.find("ls").unwrap(),
        col,
        "the triggers column must not shift the command column"
    );
    assert!(
        !text.contains("open -a Finder"),
        "the macos command stays off the linux page"
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
fn table_shows_only_the_page_platform_column() {
    let (_store, aliases) = t_store();
    let text = draw_wide(&settings::new(), &aliases);
    assert!(text.contains("name"));
    assert!(text.contains("triggers"));
    assert!(text.contains("linux"));
    assert!(
        !text.contains("macos"),
        "the other platform's column is gone"
    );
    assert!(text.contains("shortcuts"));
    // t's stored linux command appears exactly once: in the only command
    // column there is.
    let row = text
        .lines()
        .find(|l| l.contains("printf %s {input}"))
        .expect("t row");
    assert_eq!(row.matches("printf %s {input}").count(), 1, "got: {row}");
}

#[test]
fn macos_page_shows_only_the_macos_column() {
    let (_store, aliases) = t_store();
    let mac = Settings {
        platform: Platform::Macos,
        ..settings::new()
    };
    let text = draw_wide(&mac, &aliases);
    assert!(text.contains("name"));
    assert!(text.contains("triggers"));
    assert!(text.contains("macos"));
    assert!(
        !text.contains("linux"),
        "the other platform's column is gone"
    );
    assert!(text.contains("shortcuts"));
}

#[test]
fn a_macos_only_alias_shows_a_dash_on_the_linux_page() {
    let mut store = Store::default();
    store.aliases.push(AliasDef {
        name: "t".to_string(),
        triggers: vec![],
        linux: None,
        macos: Some("open -a Finder {input}".to_string()),
        shortcuts: Default::default(),
    });
    let aliases = settings::view(&store);
    let text = draw_wide(&settings::new(), &aliases);
    assert!(text.contains('—'), "no linux command shows a dash: {text}");
    assert!(
        !text.contains("open -a Finder"),
        "the macos-only command is not leaked onto the linux page"
    );
}

#[test]
fn long_commands_are_truncated_to_keep_the_table_readable() {
    let long = "x".repeat(160);
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
fn truncate_and_pad_helpers() {
    assert_eq!(truncate("abcdef", 4), "abc…");
    assert_eq!(truncate("abc", 8), "abc");
    assert_eq!(pad("ab", 4), "ab  ");
}
