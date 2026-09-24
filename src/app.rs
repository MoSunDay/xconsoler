//! Action application and execution: the only place `App` is mutated.

use crossterm::event::KeyEvent;
use std::path::Path;

use crate::action::Action;
use crate::alias::{self, AliasOp};
use crate::commands;
use crate::platform::Platform;
use crate::settings;
use crate::state::{self, App, Mode, Visibility};
use crate::storage;

/// Colon-command help shown for `:help` / unknown sub-commands.
const COLON_HELP: &str = "\":add name[,short] <linux-cmd> [// <macos-cmd>]\" \u{b7} \":del name\" \u{b7} \":arg name key value\" \u{b7} \":unarg name key\"";

/// Apply an action in place. `path` is the store.json location used for saves.
pub fn apply(app: &mut App, action: Action, platform: Platform, path: &Path) {
    match action {
        Action::Nop => {}
        Action::Quit => app.quit = true,
        Action::ToggleBar => {
            app.palette = None;
            app.visibility = match app.visibility {
                Visibility::Hidden => Visibility::Shown,
                Visibility::Shown => Visibility::Hidden,
            };
            app.status = None;
        }
        Action::OpenPalette => {
            app.palette = Some(0);
            app.status = None;
        }
        Action::ClosePalette => app.palette = None,
        Action::PaletteAccept => palette_accept(app, path),
        Action::Execute => crate::run::execute(app, platform, path),
        Action::SubmitColon => submit_colon(app, path),
        Action::InsertChar(c) => {
            app.palette = None;
            let mut s = app.input.clone();
            s.push(c);
            set_input(app, s);
        }
        Action::Backspace => {
            app.palette = None;
            let mut s = app.input.clone();
            s.pop();
            set_input(app, s);
        }
        Action::ClearInput => {
            app.palette = None;
            set_input(app, String::new());
        }
        Action::MoveUp => move_selection(app, -1),
        Action::MoveDown => move_selection(app, 1),
    }
}

/// Target resolution via the selected candidate. `Err` carries the message
/// for the status line (no match / dangling alias name).
/// Persist the store and report it on the status line: a failed save must not
/// look like a successful edit (the in-memory change would be lost on exit).
fn save_store(app: &mut App, path: &Path, ok: String) {
    app.status = match storage::save(path, &app.store) {
        Ok(()) => Some((true, ok)),
        Err(e) => Some((false, format!("{ok} - but saving failed: {e}"))),
    };
}

/// Submit a `:` command (`:add` / `:del` / `:help`).
pub fn submit_colon(app: &mut App, path: &Path) {
    let line = app.input.trim().trim_start_matches(':').trim().to_string();
    match alias::parse_colon_cmd(&line) {
        Err(e) => app.status = Some((false, e)),
        Ok(None) => {
            // help / empty / unknown: show usage, keep the input.
            app.status = Some((true, COLON_HELP.to_string()));
        }
        Ok(Some(AliasOp::Add(mut def))) => {
            // `:add t <cmd>` (no `//`) configures one command for both
            // platforms, like the settings wizard's "empty macos = linux"
            // rule; a `//`-separated pair keeps its own macos command.
            if def.macos.is_none() {
                def.macos = def.linux.clone();
            }
            let label = alias::label(&def);
            match app.store.aliases.iter().position(|d| d.name == def.name) {
                Some(i) => app.store.aliases[i] = def,
                None => app.store.aliases.push(def),
            }
            rebuild_aliases(app);
            set_input(app, String::new());
            save_store(app, path, format!("alias added: {label}"));
        }
        Ok(Some(AliasOp::Remove(name))) => {
            if alias::defaults().iter().any(|d| d.name == name) {
                app.status = Some((
                    false,
                    "builtin alias cannot be deleted (override with :add)".to_string(),
                ));
                return;
            }
            match app.store.aliases.iter().position(|d| d.name == name) {
                Some(i) => {
                    app.store.aliases.remove(i);
                    rebuild_aliases(app);
                    set_input(app, String::new());
                    save_store(app, path, format!("alias removed: {name}"));
                }
                None => app.status = Some((false, format!("alias not found: {name}"))),
            }
        }
        Ok(Some(AliasOp::SetArg { name, key, value })) => {
            let canonical = alias::resolve(&app.aliases, &name).map(|d| d.name.clone());
            match canonical {
                None => app.status = Some((false, format!("alias not found: {name}"))),
                Some(cname) => match alias::set_arg(&mut app.store.aliases, &cname, &key, &value) {
                    Ok(prev) => {
                        rebuild_aliases(app);
                        set_input(app, String::new());
                        let verb = if prev.is_some() {
                            "arg updated"
                        } else {
                            "arg added"
                        };
                        save_store(app, path, format!("{verb}: {cname} {key}"));
                    }
                    Err(e) => app.status = Some((false, e)),
                },
            }
        }
        Ok(Some(AliasOp::DelArg { name, key })) => {
            let canonical = alias::resolve(&app.aliases, &name).map(|d| d.name.clone());
            match canonical {
                None => app.status = Some((false, format!("alias not found: {name}"))),
                Some(cname) => match alias::remove_arg(&mut app.store.aliases, &cname, &key) {
                    Ok(()) => {
                        rebuild_aliases(app);
                        set_input(app, String::new());
                        save_store(app, path, format!("arg removed: {cname} {key}"));
                    }
                    Err(e) => app.status = Some((false, e)),
                },
            }
        }
    }
}

/// Submit a `/` command. Only `/settings` exists today; unknown ones keep
/// the input (the help block above the bar lists what is available).
pub(crate) fn submit_slash(app: &mut App, line: &str) {
    if line == "/settings" {
        set_input(app, String::new());
        app.mode = Mode::Settings(Box::new(settings::new()));
    } else {
        app.status = Some((
            false,
            format!("unknown command: {line} \u{b7} try /settings"),
        ));
    }
}

/// Route one key event to the settings page and apply its effect.
/// The transition itself is pure; effect application (and persistence) lives
/// in [`crate::settings_apply`] so this module stays a thin router.
pub fn apply_settings(app: &mut App, key: KeyEvent, path: &Path) {
    crate::settings_apply::apply(app, key, path);
}

/// Replace the input; resets cursor and transient status.
pub(crate) fn set_input(app: &mut App, s: String) {
    app.input = s;
    app.cursor = 0;
    app.status = None;
}

/// Move the selection by `delta`, clamped into the candidate list.
fn move_sel(app: &mut App, delta: i32) {
    let len = state::candidates(app).len();
    if len == 0 {
        app.cursor = 0;
        return;
    }
    let cur = app.cursor.min(len - 1) as i32;
    app.cursor = (cur + delta).clamp(0, len as i32 - 1) as usize;
}

/// Up/Down: move the palette selection while it is open, else the ranked
/// candidate cursor Enter will run.
fn move_selection(app: &mut App, delta: i32) {
    let Some(cur) = app.palette else {
        return move_sel(app, delta);
    };
    let len = commands::len();
    if len == 0 {
        app.palette = Some(0);
        return;
    }
    let cur = cur.min(len - 1) as i32;
    app.palette = Some((cur + delta).clamp(0, len as i32 - 1) as usize);
}

/// Enter on an open palette: complete commands (`:help`, `/settings`) run
/// immediately, arg-taking ones prefill the input (`:add `). Closes either way.
fn palette_accept(app: &mut App, path: &Path) {
    let Some(spec) = app.palette.and_then(commands::get).copied() else {
        app.palette = None;
        return;
    };
    app.palette = None;
    let text = commands::insert_text(&spec);
    set_input(app, text.clone());
    if spec.needs_arg {
        return;
    }
    if text.starts_with('/') {
        submit_slash(app, &text);
    } else {
        submit_colon(app, path);
    }
}

/// Recompute the effective alias list after a store mutation.
fn rebuild_aliases(app: &mut App) {
    app.aliases = storage::merge_aliases(&app.store.aliases);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alias::AliasOp;
    use crate::history;
    use crate::storage::Store;
    use crossterm::event::{KeyCode, KeyModifiers};
    use tempfile::TempDir;

    fn setup() -> (App, TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("store.json");
        (state::new(Store::default(), false), dir, path)
    }

    /// Force the shown state directly (apps start shown; the old helper
    /// toggled from the old hidden start).
    fn shown(app: &mut App) {
        app.visibility = Visibility::Shown;
    }

    fn type_str(app: &mut App, s: &str, path: &Path) {
        for c in s.chars() {
            apply(app, Action::InsertChar(c), Platform::Linux, path);
        }
    }

    #[test]
    fn colon_add_single_command_mirrors_linux_to_macos() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        type_str(&mut app, ":add t,tt printf %s {input}", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        assert_eq!(app.status, Some((true, "alias added: t (tt)".to_string())));

        let def = app.aliases.iter().find(|d| d.name == "t").unwrap();
        assert_eq!(def.linux.as_deref(), Some("printf %s {input}"));
        assert_eq!(
            def.macos.as_deref(),
            Some("printf %s {input}"),
            "one command serves both platforms"
        );
        // The mirror is persisted, so a reload keeps working on macOS.
        let reloaded = storage::load(&path);
        assert_eq!(
            reloaded.aliases[0].macos.as_deref(),
            Some("printf %s {input}")
        );

        app.input = "t hi".to_string();
        apply(&mut app, Action::Execute, Platform::Macos, &path);
        assert_eq!(app.status, Some((true, "t ok: hi".to_string())));
    }

    #[test]
    fn colon_add_with_separator_keeps_both_commands() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        type_str(&mut app, ":add t printf %s {input} // echo {input}", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);

        let def = app.aliases.iter().find(|d| d.name == "t").unwrap();
        assert_eq!(def.linux.as_deref(), Some("printf %s {input}"));
        assert_eq!(def.macos.as_deref(), Some("echo {input}"));

        // One-sided aliases still run through the other platform's command.
        type_str(&mut app, ":add m - // printf %s {input}", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        let def = app.aliases.iter().find(|d| d.name == "m").unwrap();
        assert_eq!(def.linux, None);
        assert_eq!(def.macos.as_deref(), Some("printf %s {input}"));
        app.input = "m hi".to_string();
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert_eq!(app.status, Some((true, "m ok: hi".to_string())));
    }

    #[test]
    fn repeat_execution_dedupes_history() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        type_str(&mut app, ":add t printf %s {input}", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);

        for _ in 0..2 {
            app.input = "t hello".to_string();
            apply(&mut app, Action::Execute, Platform::Linux, &path);
        }
        assert!(app.status.as_ref().unwrap().0);
        assert_eq!(app.store.history.len(), 1);
    }

    #[test]
    fn selected_history_candidate_reuses_recorded_pair() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        type_str(&mut app, ":add t printf %s {input}", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);

        app.input = "t a".to_string();
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        app.input = "t b".to_string();
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert_eq!(app.store.history.len(), 2);

        // Empty input => history-first list; cursor 1 is the older "a".
        app.input = String::new();
        app.cursor = 1;
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert_eq!(app.store.history.len(), 2);
        assert_eq!(app.store.history[0].alias, "t");
        assert_eq!(app.store.history[0].input(), "a"); // moved to head
    }

    #[test]
    fn submit_colon_del_and_errors() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);

        type_str(&mut app, ":add t echo {input}", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        assert_eq!(app.store.aliases.len(), 1);

        // unknown command -> usage help, input kept
        type_str(&mut app, ":frobnicate", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        let (ok, msg) = app.status.clone().unwrap();
        assert!(ok && msg.contains(":add"));
        assert_eq!(app.input, ":frobnicate");

        // parse error
        app.input = ":add".to_string();
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        assert!(!app.status.as_ref().unwrap().0);

        // builtin delete refused
        app.input = ":del br".to_string();
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        assert_eq!(
            app.status,
            Some((
                false,
                "builtin alias cannot be deleted (override with :add)".to_string()
            ))
        );

        // unknown user alias
        app.input = ":del t".to_string(); // still exists
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        assert_eq!(app.status, Some((true, "alias removed: t".to_string())));
        assert!(app.store.aliases.is_empty());

        app.input = ":del t".to_string();
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        assert_eq!(app.status, Some((false, "alias not found: t".to_string())));
    }

    #[test]
    fn move_sel_clamps_to_candidate_list() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        // empty input lists recent history only: seed two rows to move over
        history::record(&mut app.store, "br", "baidu", 1);
        history::record(&mut app.store, "br", "gm", 2);
        apply(&mut app, Action::MoveDown, Platform::Linux, &path);
        assert_eq!(app.cursor, 1);
        apply(&mut app, Action::MoveDown, Platform::Linux, &path);
        apply(&mut app, Action::MoveDown, Platform::Linux, &path);
        assert_eq!(app.cursor, 1); // clamped at bottom
        apply(&mut app, Action::MoveUp, Platform::Linux, &path);
        assert_eq!(app.cursor, 0);
        apply(&mut app, Action::MoveUp, Platform::Linux, &path);
        assert_eq!(app.cursor, 0); // clamped at top

        app.aliases.clear();
        app.store.history.clear();
        apply(&mut app, Action::MoveDown, Platform::Linux, &path);
        assert_eq!(app.cursor, 0); // empty list -> 0
    }

    #[test]
    fn palette_accept_prefills_arg_taking_commands() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        apply(&mut app, Action::OpenPalette, Platform::Linux, &path);
        assert_eq!(app.palette, Some(0));
        apply(&mut app, Action::PaletteAccept, Platform::Linux, &path);
        assert_eq!(app.palette, None, "accepting closes the palette");
        assert_eq!(app.input, ":add ");
    }

    #[test]
    fn palette_accept_runs_complete_commands() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);

        app.palette = Some(4); // :help
        apply(&mut app, Action::PaletteAccept, Platform::Linux, &path);
        assert_eq!(app.palette, None);
        let (ok, msg) = app.status.clone().expect(":help sets the usage status");
        assert!(ok && msg.contains(":add"));

        app.palette = Some(5); // /settings
        apply(&mut app, Action::PaletteAccept, Platform::Linux, &path);
        assert_eq!(app.palette, None);
        assert!(matches!(app.mode, Mode::Settings(_)));
    }

    #[test]
    fn palette_arrows_move_and_clamp() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        // Closed palette: the candidate cursor moves as before.
        apply(&mut app, Action::MoveDown, Platform::Linux, &path);
        assert_eq!(app.palette, None);

        apply(&mut app, Action::OpenPalette, Platform::Linux, &path);
        apply(&mut app, Action::MoveDown, Platform::Linux, &path);
        assert_eq!(app.palette, Some(1));
        apply(&mut app, Action::MoveUp, Platform::Linux, &path);
        apply(&mut app, Action::MoveUp, Platform::Linux, &path);
        assert_eq!(app.palette, Some(0), "clamped at the top");
        for _ in 0..commands::len() + 3 {
            apply(&mut app, Action::MoveDown, Platform::Linux, &path);
        }
        assert_eq!(
            app.palette,
            Some(commands::len() - 1),
            "clamped at the bottom"
        );
    }

    #[test]
    fn typing_closes_the_palette_and_inserts() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        apply(&mut app, Action::OpenPalette, Platform::Linux, &path);
        apply(&mut app, Action::InsertChar('b'), Platform::Linux, &path);
        assert_eq!(app.palette, None);
        assert_eq!(app.input, "b");
    }

    #[test]
    fn toggle_resets_status() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        app.status = Some((true, "hi".to_string()));
        apply(&mut app, Action::ToggleBar, Platform::Linux, &path);
        assert_eq!(app.visibility, Visibility::Hidden);
        assert_eq!(app.status, None);
        apply(&mut app, Action::Quit, Platform::Linux, &path);
        assert!(app.quit);
    }

    #[test]
    fn named_arg_execute_resolves_value_and_records_raw_text() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        type_str(&mut app, ":add t printf %s {input}", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        type_str(&mut app, ":arg t here cd /tmp", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        assert_eq!(app.status, Some((true, "arg added: t here".to_string())));

        app.input = "t here".to_string();
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert_eq!(app.status, Some((true, "t ok: cd /tmp".to_string())));
        assert_eq!(app.store.history.len(), 1);
        assert_eq!(
            app.store.history[0].input(),
            "here",
            "history keeps the raw text, so the shorthand replays"
        );

        type_str(&mut app, ":unarg t here", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        assert_eq!(app.status, Some((true, "arg removed: t here".to_string())));
        app.input = "t here".to_string();
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert_eq!(app.status, Some((true, "t ok: here".to_string())));
    }

    #[test]
    fn colon_arg_resolves_builtin_by_name() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        type_str(&mut app, ":arg br gh https://github.com", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        assert_eq!(app.status, Some((true, "arg added: br gh".to_string())));
        let def = alias::resolve(&app.aliases, "br").unwrap();
        assert_eq!(
            def.args.get("gh").map(String::as_str),
            Some("https://github.com")
        );
        assert_eq!(
            def.args.get("baidu").map(String::as_str),
            Some("https://www.baidu.com"),
            "override keeps br's registered builtin args"
        );
        assert_eq!(
            app.store.aliases.len(),
            1,
            "builtin materialized as override"
        );
        assert!(!app.store.aliases[0].builtin);
        // Re-registering an existing key reports an update, not an add.
        type_str(&mut app, ":arg br gh https://gitlab.com", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        assert_eq!(app.status, Some((true, "arg updated: br gh".to_string())));
    }

    #[test]
    fn slash_settings_opens_the_page_and_esc_returns() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        app.input = "/settings".to_string();
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert!(matches!(&app.mode, Mode::Settings(_)));
        assert!(app.input.is_empty(), "command line cleared on entry");

        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        apply_settings(&mut app, esc, &path);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn unknown_slash_command_keeps_input_and_points_at_settings() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        app.input = "/nope".to_string();
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.input, "/nope");
        assert!(app
            .status
            .as_ref()
            .is_some_and(|(ok, m)| !ok && m.contains("/settings")));
    }

    #[test]
    fn settings_page_delete_alias_persists() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        type_str(&mut app, ":add t printf %s {input}", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);

        app.input = "/settings".to_string();
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        // merged rows: br(0), cd(1), t(2)
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        apply_settings(&mut app, down, &path);
        apply_settings(&mut app, down, &path);
        apply_settings(
            &mut app,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
            &path,
        );
        assert!(app.store.aliases.is_empty(), "user alias deleted");
        assert!(
            storage::load(&path).aliases.is_empty(),
            "deletion persisted"
        );
        let Mode::Settings(st) = &app.mode else {
            panic!("still on the settings page");
        };
        assert!(st
            .status
            .as_ref()
            .is_some_and(|(ok, m)| *ok && m.contains('t')));
    }

    #[test]
    fn parse_still_recognizes_ops() {
        // guard: domain API used by submit_colon behaves as expected
        assert!(matches!(
            alias::parse_colon_cmd("del x").unwrap().unwrap(),
            AliasOp::Remove(_)
        ));
    }
}
