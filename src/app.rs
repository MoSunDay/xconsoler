//! Action application and execution: the only place `App` is mutated.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::action::Action;
use crate::alias::{self, AliasDef, AliasOp};
use crate::exec::{self, ExecOutcome};
use crate::history;
use crate::matcher::Candidate;
use crate::platform::Platform;
use crate::state::{self, App, Visibility};
use crate::storage;

/// Colon-command help shown for `:help` / unknown sub-commands.
const COLON_HELP: &str = "\":add name[,short] <linux-cmd> // <macos-cmd>\" \u{b7} \":del name\"";

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

    match exec::run_alias(&def, &input, platform) {
        ExecOutcome::Success(_) => {
            history::record(&mut app.store, &def.name, &input, now_secs());
            let _ = storage::save(path, &app.store);
            let shown = if input.trim().is_empty() {
                def.name.clone()
            } else {
                input.clone()
            };
            set_input(app, String::new());
            app.status = Some((true, format!("{} ok: {}", def.name, shown)));
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
    use crate::alias::AliasOp;
    use crate::storage::Store;
    use tempfile::TempDir;

    fn setup() -> (App, TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("store.json");
        (state::new(Store::default()), dir, path)
    }

    fn shown(app: &mut App) {
        apply(
            app,
            Action::ToggleBar,
            Platform::Linux,
            Path::new("/dev/null"),
        );
    }

    fn type_str(app: &mut App, s: &str, path: &Path) {
        for c in s.chars() {
            apply(app, Action::InsertChar(c), Platform::Linux, path);
        }
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
    fn parse_still_recognizes_ops() {
        // guard: domain API used by submit_colon behaves as expected
        assert!(matches!(
            alias::parse_colon_cmd("del x").unwrap().unwrap(),
            AliasOp::Remove(_)
        ));
    }
}
