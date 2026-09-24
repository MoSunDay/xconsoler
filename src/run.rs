//! Enter handling: resolve the input line to `(alias, input)` and run it,
//! recording the outcome on the status line. Split out of [`crate::app`] so
//! the action router stays a thin dispatcher.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::alias::{self, AliasDef};
use crate::exec::{self, ExecOutcome};
use crate::history;
use crate::matcher::Candidate;
use crate::platform::Platform;
use crate::state::{self, App};
use crate::storage;

/// Enter on normal input: resolve an alias, run it, record history.
pub fn execute(app: &mut App, platform: Platform, path: &Path) {
    let trimmed = app.input.trim().to_string();
    if trimmed.starts_with('/') {
        return crate::app::submit_slash(app, &trimmed);
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let head = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim().to_string();

    // 1. "<alias> <args>": a resolving head wins; `rest` is the input, except
    //    when the cursor sits on one of that alias's named-arg rows (`br b`).
    // 2. Otherwise the selected candidate decides (alias => whole input,
    //    history => recorded alias + input). Errors surface as a status line.
    // The def is cloned so the borrow of `app` ends before we mutate the store.
    let target: Result<(AliasDef, String), String> = match alias::resolve(&app.aliases, head) {
        Some(def) => match arg_override(app, def, &rest) {
            Some(key) => Ok((def.clone(), key)),
            None => Ok((def.clone(), rest)),
        },
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
            let shown = if run_input.trim().is_empty() {
                def.name.clone()
            } else {
                run_input.clone()
            };
            crate::app::set_input(app, String::new());
            // The run succeeded either way; only persistence can still fail.
            app.status = Some(match storage::save(path, &app.store) {
                Ok(()) => (true, format!("{} ok: {}", def.name, shown)),
                Err(e) => (
                    false,
                    format!("{} ran, but saving history failed: {e}", def.name),
                ),
            });
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

/// Head resolved but `rest` is not an exact key: the highlighted named-arg row
/// decides, so Up/Down + Enter works for the `br b` picker flow.
fn arg_override(app: &App, def: &AliasDef, rest: &str) -> Option<String> {
    if def.args.contains_key(rest.trim()) {
        return None;
    }
    match state::selected(app) {
        // The arg KEY travels on as the input, so `exec::resolve_args` maps it
        // and history keeps the replayable shorthand (`br baidu`, not a URL).
        Some(Candidate::Arg { alias, key }) if alias.eq_ignore_ascii_case(&def.name) => Some(key),
        _ => None,
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

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;
    use crate::app::apply;
    use crate::state::Visibility;
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
    fn highlighted_arg_row_runs_for_a_typed_partial() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        type_str(&mut app, ":add t,tt printf %s {input}", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        type_str(&mut app, ":arg t baidu https://example.com/b", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        type_str(&mut app, ":arg t bing https://example.com/i", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);

        // "t b" lists both args: Enter runs the highlighted one, not the
        // literal partial.
        app.input = "t b".to_string();
        app.cursor = 0;
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert_eq!(
            app.status,
            Some((true, "t ok: https://example.com/b".to_string()))
        );
        // History keeps the raw key so the shorthand stays replayable.
        assert_eq!(app.store.history[0].input(), "baidu");

        // Moving the highlight down picks the other arg.
        app.input = "t b".to_string();
        apply(&mut app, Action::MoveDown, Platform::Linux, &path);
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert_eq!(
            app.status,
            Some((true, "t ok: https://example.com/i".to_string()))
        );
    }

    #[test]
    fn an_exactly_typed_arg_key_keeps_its_own_meaning() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        type_str(&mut app, ":add t,tt printf %s {input}", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        type_str(&mut app, ":arg t baidu https://example.com/b", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        type_str(&mut app, ":arg t bing https://example.com/i", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);

        app.input = "t baidu".to_string();
        let def = alias::resolve(&app.aliases, "t").expect("t resolves");
        // A cursor parked on another row must not change what `t baidu` does.
        app.cursor = 1;
        assert_eq!(arg_override(&app, def, "baidu"), None);
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert_eq!(
            app.status,
            Some((true, "t ok: https://example.com/b".to_string()))
        );
    }

    #[test]
    fn a_failed_history_save_is_reported() {
        let (mut app, dir, _good) = setup();
        shown(&mut app);
        // A path whose parent is a plain file: `storage::save` cannot write.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let bad = blocker.join("store.json");
        type_str(&mut app, ":add t,tt printf %s {input}", &bad);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &bad);
        app.input = "t hello".to_string();
        apply(&mut app, Action::Execute, Platform::Linux, &bad);
        let (ok, msg) = app.status.clone().expect("a status line");
        assert!(!ok, "the run succeeded but the save failed");
        assert!(msg.contains("saving history failed"), "got {msg}");
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
    fn a_command_that_fails_records_no_history() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        type_str(&mut app, ":add bad false", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        app.input = "bad".to_string();
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert!(
            matches!(app.status, Some((false, _))),
            "status: {:?}",
            app.status
        );
        assert!(
            app.store.history.is_empty(),
            "a failed run must not enter history: {:?}",
            app.store.history
        );
    }

    #[test]
    fn a_backgrounded_launch_that_never_started_records_no_history() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        type_str(
            &mut app,
            ":add bg nosuchbin_xconsoler_probe >/dev/null 2>&1 &",
            &path,
        );
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        app.input = "bg".to_string();
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert!(
            matches!(app.status, Some((false, _))),
            "a launch that never started must be judged a failure: {:?}",
            app.status
        );
        assert!(
            app.store.history.is_empty(),
            "status: {:?} history: {:?}",
            app.status,
            app.store.history
        );
        // The failure branch only sets the status, so the input survives for
        // a retry.
        assert_eq!(app.input, "bg", "input kept for retry: {:?}", app.input);
    }
}
