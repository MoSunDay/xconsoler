//! Action application and execution: the only place `App` is mutated.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::KeyEvent;

use crate::action::Action;
use crate::alias::{self, AliasDef, AliasOp};
use crate::exec::{self, ExecOutcome};
use crate::history;
use crate::matcher::Candidate;
use crate::platform::Platform;
use crate::settings::{self, Effect};
use crate::state::{self, App, Mode, Visibility};
use crate::storage;

/// Colon-command help shown for `:help` / unknown sub-commands.
const COLON_HELP: &str = "\":add name[,short] <linux-cmd> // <macos-cmd>\" \u{b7} \":del name\" \u{b7} \":arg name key value\" \u{b7} \":unarg name key\"";

/// Apply an action in place. `path` is the store.json location used for saves.
pub fn apply(app: &mut App, action: Action, platform: Platform, path: &Path) {
    match action {
        Action::Nop => {}
        Action::Quit => app.quit = true,
        Action::ToggleBar => {
            app.visibility = match app.visibility {
                Visibility::Hidden => Visibility::Shown,
                Visibility::Shown => Visibility::Hidden,
            };
            app.status = None;
        }
        Action::Execute => execute(app, platform, path),
        Action::SubmitColon => submit_colon(app, path),
        Action::InsertChar(c) => {
            let mut s = app.input.clone();
            s.push(c);
            set_input(app, s);
        }
        Action::Backspace => {
            let mut s = app.input.clone();
            s.pop();
            set_input(app, s);
        }
        Action::ClearInput => set_input(app, String::new()),
        Action::MoveUp => move_sel(app, -1),
        Action::MoveDown => move_sel(app, 1),
    }
}

/// Enter on normal input: resolve an alias, run it, record history.
pub fn execute(app: &mut App, platform: Platform, path: &Path) {
    let trimmed = app.input.trim().to_string();
    if trimmed.starts_with('/') {
        return submit_slash(app, &trimmed);
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let head = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim().to_string();

    // 1. "<alias> <args>": a resolving head wins, args become the input.
    // 2. Otherwise the selected candidate decides (alias => whole input,
    //    history => recorded alias + input). Errors surface as a status line.
    // The def is cloned so the borrow of `app` ends before we mutate the store.
    let target: Result<(AliasDef, String), String> = match alias::resolve(&app.aliases, head) {
        Some(def) => Ok((def.clone(), rest)),
        None => selected_target(app, &trimmed),
    };
    let (def, input) = match target {
        Ok(t) => t,
        Err(msg) => {
            app.status = Some((false, msg));
            return;
        }
    };

    // Named args (`br baidu`): the command sees the mapped value, while
    // history keeps the raw text so the shorthand stays replayable.
    let run_input = exec::resolve_args(&def, &input);

    match exec::run_alias(&def, &run_input, platform) {
        ExecOutcome::Success(_) => {
            history::record(&mut app.store, &def.name, &input, now_secs());
            let _ = storage::save(path, &app.store);
            let shown = if run_input.trim().is_empty() {
                def.name.clone()
            } else {
                run_input.clone()
            };
            set_input(app, String::new());
            app.status = Some((true, format!("{} ok: {}", def.name, shown)));
            // Desktop-launcher semantics: in summon mode a successful run
            // dismisses the bar (Spotlight-style); the hotkey resummons it.
            if app.summon {
                app.quit = true;
            }
        }
        ExecOutcome::Failure(msg) => {
            // Keep the input so it can be fixed and retried.
            app.status = Some((false, msg));
        }
    }
}

/// Target resolution via the selected candidate. `Err` carries the message
/// for the status line (no match / dangling alias name).
fn selected_target(app: &App, trimmed: &str) -> Result<(AliasDef, String), String> {
    match state::selected(app) {
        Some(Candidate::Alias { name }) => match alias::resolve(&app.aliases, &name) {
            Some(def) => Ok((def.clone(), trimmed.to_string())),
            None => Err(format!("alias not found: {name}")),
        },
        Some(Candidate::Arg { alias, key }) => match alias::resolve(&app.aliases, &alias) {
            Some(def) => match def.args.get(&key) {
                Some(value) => Ok((def.clone(), value.clone())),
                None => Err(format!("named arg not found: {key}")),
            },
            None => Err(format!("alias not found: {alias}")),
        },
        Some(Candidate::History { idx }) => match app.store.history.get(idx) {
            Some(entry) => {
                let input = entry.input();
                match alias::resolve(&app.aliases, &entry.alias) {
                    Some(def) => Ok((def.clone(), input)),
                    None => Err(format!("alias not found: {}", entry.alias)),
                }
            }
            None => Err("no match".to_string()),
        },
        None => Err("no match".to_string()),
    }
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
        Ok(Some(AliasOp::Add(def))) => {
            let label = alias::label(&def);
            match app.store.aliases.iter().position(|d| d.name == def.name) {
                Some(i) => app.store.aliases[i] = def,
                None => app.store.aliases.push(def),
            }
            rebuild_aliases(app);
            let _ = storage::save(path, &app.store);
            set_input(app, String::new());
            app.status = Some((true, format!("alias added: {label}")));
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
                    let _ = storage::save(path, &app.store);
                    set_input(app, String::new());
                    app.status = Some((true, format!("alias removed: {name}")));
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
                        let _ = storage::save(path, &app.store);
                        set_input(app, String::new());
                        let verb = if prev.is_some() { "arg updated" } else { "arg added" };
                        app.status = Some((true, format!("{verb}: {cname} {key}")));
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
                        let _ = storage::save(path, &app.store);
                        set_input(app, String::new());
                        app.status = Some((true, format!("arg removed: {cname} {key}")));
                    }
                    Err(e) => app.status = Some((false, e)),
                },
            }
        }
    }
}

/// Submit a `/` command. Only `/settings` exists today; unknown ones keep
/// the input (the help block above the bar lists what is available).
fn submit_slash(app: &mut App, line: &str) {
    if line == "/settings" {
        set_input(app, String::new());
        app.mode = Mode::Settings(Box::new(settings::new()));
    } else {
        app.status = Some((false, format!("unknown command: {line} \u{b7} try /settings")));
    }
}

/// Route one key event to the settings page and apply its effect. The store
/// returned by the pure transition replaces `app.store` wholesale.
pub fn apply_settings(app: &mut App, key: KeyEvent, path: &Path) {
    let Mode::Settings(st) = std::mem::replace(&mut app.mode, Mode::Normal) else {
        return;
    };
    let (st, store, effect) = settings::handle_key(&st, &app.store, key);
    app.store = store;
    match effect {
        Effect::None => app.mode = Mode::Settings(Box::new(st)),
        Effect::Back => {}
        Effect::Save => {
            rebuild_aliases(app);
            let _ = storage::save(path, &app.store);
            app.mode = Mode::Settings(Box::new(st));
        }
        Effect::Quit => app.quit = true,
    }
}

/// Replace the input; resets cursor and transient status.
fn set_input(app: &mut App, s: String) {
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

/// Recompute the effective alias list after a store mutation.
fn rebuild_aliases(app: &mut App) {
    app.aliases = storage::merge_aliases(&app.store.aliases);
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use crate::alias::AliasOp;
    use crate::storage::Store;
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
    fn summon_mode_quits_after_successful_execute() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        app.summon = true;
        type_str(&mut app, ":add t,tt printf %s {input}", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        assert!(!app.quit, "colon commands keep the bar open");

        type_str(&mut app, "t hello", &path);
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert!(app.quit, "summon bar auto-dismisses on success");
        assert_eq!(app.store.history.len(), 1);

        // Failure keeps the bar open so the input can be fixed and retried.
        let (mut app2, _dir, path2) = setup();
        shown(&mut app2);
        app2.summon = true;
        app2.input = "t ".to_string();
        apply(&mut app2, Action::Execute, Platform::Linux, &path2);
        assert!(!app2.quit);
    }

    #[test]
    fn execute_via_add_records_history_and_saves_b64() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        type_str(
            &mut app,
            ":add t,tt printf %s {input} // printf %s {input}",
            &path,
        );
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        assert_eq!(app.status, Some((true, "alias added: t (tt)".to_string())));
        assert!(app.input.is_empty());
        assert_eq!(app.aliases.iter().filter(|d| d.name == "t").count(), 1);

        app.input = "t hello".to_string();
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert_eq!(app.status, Some((true, "t ok: hello".to_string())));
        assert_eq!(app.store.history.len(), 1);
        assert!(app.input.is_empty());
        assert_eq!(app.store.history[0].alias, "t");

        let reloaded = storage::load(&path);
        assert_eq!(reloaded.history.len(), 1);
        assert_eq!(reloaded.history[0].input(), "hello");
        assert_ne!(reloaded.history[0].input_b64, "hello"); // stored base64
        assert_eq!(reloaded.aliases.len(), 1);
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
    fn execute_without_match_reports_error() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        app.aliases.clear();
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert_eq!(app.status, Some((false, "no match".to_string())));
    }

    #[test]
    fn execute_failure_keeps_input() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        type_str(&mut app, ":add t printf %s {input}", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        app.input = "t".to_string(); // head resolves, empty {input}
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert_eq!(app.status, Some((false, "input required".to_string())));
        assert_eq!(app.input, "t");
        assert!(app.store.history.is_empty());
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
        app.input = ":del browser".to_string();
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
        // default candidates: 0 history + 2 aliases = 2 rows
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
    fn colon_arg_resolves_alias_by_shortcut() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        type_str(&mut app, ":arg br baidu https://www.baidu.com", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        assert_eq!(app.status, Some((true, "arg added: browser baidu".to_string())));
        let def = alias::resolve(&app.aliases, "br").unwrap();
        assert_eq!(
            def.args.get("baidu").map(String::as_str),
            Some("https://www.baidu.com")
        );
        assert_eq!(app.store.aliases.len(), 1, "builtin materialized as override");
        assert!(!app.store.aliases[0].builtin);
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
        // merged rows: browser(0), clipboard(1), t(2)
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        apply_settings(&mut app, down, &path);
        apply_settings(&mut app, down, &path);
        apply_settings(
            &mut app,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
            &path,
        );
        assert!(app.store.aliases.is_empty(), "user alias deleted");
        assert!(storage::load(&path).aliases.is_empty(), "deletion persisted");
        let Mode::Settings(st) = &app.mode else {
            panic!("still on the settings page");
        };
        assert!(st.status.as_ref().is_some_and(|(ok, m)| *ok && m.contains('t')));
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
