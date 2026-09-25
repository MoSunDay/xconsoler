//! Wizard-chrome rendering tests for `/settings`: titles, prompts, the
//! prefilled input line and its caret window. Split out of `settings_view.rs`
//! so every file stays within the size budget; the table tests live in
//! `settings_view/list_tests.rs`.

use super::test_util::*;
use super::*;

#[test]
fn form_shows_prompt_and_input() {
    let (_store, aliases) = t_store();
    let mut st = settings::new();
    let mut form = settings_form::new_alias(Platform::Linux);

    form.step = 1;
    st.form = Some(form.clone());
    let text = draw_once(&st, &aliases);
    assert!(
        text.contains("new alias (2/3)"),
        "the wizard has three steps"
    );

    form.step = 2;
    form.input = "printf".to_string();
    form.caret = 0; // direct fixture: a caret at the start hides no chars
    st.form = Some(form);
    let text = draw_once(&st, &aliases);
    assert!(text.contains("new alias (3/3)"));
    assert!(text.contains("linux command — use {input}"));
    assert!(text.contains("❯ █printf"), "caret block precedes the text");
    assert!(text.contains("Esc cancel"));
}

#[test]
fn form_caret_in_the_middle_keeps_every_character() {
    let (_store, aliases) = t_store();
    let mut st = settings::new();
    let mut form = settings_form::new_alias(Platform::Linux);
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
    let mut form = settings_form::new_edit_command("t", Platform::Linux, Some(&long));
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
    let mut form = settings_form::new_alias(Platform::Linux);
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
    let form = settings_form::new_edit_command("t", Platform::Linux, Some("printf %s {input}"));
    st.form = Some(form);
    let text = draw_wide(&st, &aliases);
    assert!(text.contains("edit command for 't' (1/1)"));
    assert!(text.contains("linux command — use {input}"));
    assert!(
        text.contains("❯ printf %s {input}"),
        "the step is prefilled"
    );
    assert!(text.contains("Enter next/accept"));
}

#[test]
fn edit_command_on_macos_names_the_macos_command() {
    let (_store, aliases) = t_store();
    let mut st = settings::new();
    let form = settings_form::new_edit_command("t", Platform::Macos, Some("open {input}"));
    st.form = Some(form);
    let text = draw_wide(&st, &aliases);
    assert!(text.contains("edit command for 't' (1/1)"));
    assert!(text.contains("macos command — use {input}"));
    assert!(text.contains("❯ open {input}"));
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
    let mut form = settings_form::new_alias(Platform::Linux);
    form.error = Some("name cannot be empty".to_string());
    st.form = Some(form);
    let text = draw_once(&st, &aliases);
    assert!(text.contains("✗ name cannot be empty"));
}
