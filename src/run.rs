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
        return crate::app::submit_slash(app, &trimmed, platform);
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let head = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim().to_string();

    // 1. "<alias> <shortcut>": a resolving head wins; `rest` is the input,
    //    except when the cursor sits on one of that alias's shortcut rows.
    // 2. Otherwise the selected candidate decides (shortcut => its key, mapped
    //    at run time; history => recorded alias + input). Errors surface as a
    //    status line.
    // The def is cloned so the borrow of `app` ends before we mutate the store.
    let target: Result<(AliasDef, String), String> = match alias::resolve(&app.aliases, head) {
        Some(def) => match shortcut_override(app, def, &rest) {
            Some(key) => Ok((def.clone(), key)),
            None => Ok((def.clone(), rest)),
        },
        None => selected_target(app),
    };
    let (def, input) = match target {
        Ok(t) => t,
        Err(msg) => {
            app.status = Some((false, msg));
            return;
        }
    };

    // Registered shortcuts (`br baidu`): the command sees the mapped value,
    // while history keeps the raw text so the shorthand stays replayable.
    let run_input = exec::resolve_shortcuts(&def, &input);
    // A native clipboard alias carries base64: only the text actually
    // published is decoded, while history keeps the input as typed.
    let payload = clipboard_payload(exec::uses_native_clipboard(&def, platform), &run_input);

    match exec::run_alias(&def, &payload, platform) {
        ExecOutcome::Success(_) => {
            history::record(&mut app.store, &def.name, &input, now_secs());
            // The status echoes the raw text, not the expanded value:
            // history records the raw key too, so neither restates it.
            let shown = if input.trim().is_empty() {
                def.name.clone()
            } else {
                input.clone()
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
        ExecOutcome::Started(msg) => {
            // Still running when the grace ran out: nothing proved success,
            // so nothing is recorded. Not a failure either -- the input
            // survives (it would otherwise be lost, since an unproven run is
            // not in the history) and the bar stays open to show why.
            app.status = Some((true, format!("{msg} (still running; not recorded)")));
        }
        ExecOutcome::Failure(msg) => {
            // Keep the input so it can be fixed and retried.
            app.status = Some((false, msg));
        }
    }
}

/// Head resolved but `rest` is not an exact key: the highlighted shortcut row
/// decides, so Up/Down + Enter works for the `br b` picker flow.
fn shortcut_override(app: &App, def: &AliasDef, rest: &str) -> Option<String> {
    if def.shortcuts.contains_key(rest.trim()) {
        return None;
    }
    match state::selected(app) {
        // The shortcut KEY travels on as the input, so
        // `exec::resolve_shortcuts` maps it and history keeps the replayable
        // shorthand (`br baidu`, not a URL).
        Some(Candidate::Shortcut { alias, key }) if alias.eq_ignore_ascii_case(&def.name) => {
            Some(key)
        }
        _ => None,
    }
}

/// Target resolution via the selected candidate. `Err` carries the message
/// for the status line (no match / dangling alias name).
fn selected_target(app: &App) -> Result<(AliasDef, String), String> {
    match state::selected(app) {
        // The shortcut KEY travels on as the input (mirroring
        // `shortcut_override`), so `exec::resolve_shortcuts` maps it and
        // history keeps the replayable shorthand (`br baidu`, not a URL).
        Some(Candidate::Shortcut { alias, key }) => match alias::resolve(&app.aliases, &alias) {
            Some(def) if def.shortcuts.contains_key(&key) => Ok((def.clone(), key)),
            Some(_) => Err(format!("shortcut not found: {key}")),
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

/// Text a run publishes to the native clipboard backend.
///
/// Native clipboard aliases (`cd`) are handed base64: the clipboard gets the
/// decoded text, while the input itself -- the base64 the user typed, or the
/// entry a replay picked -- is what the history records, so the stored form
/// stays base64 and a replay decodes it again the same way. `decode_b64`
/// returns invalid base64 (ordinary plain text) unchanged, so `cd hello`
/// keeps working. Every other alias publishes its resolved input verbatim.
fn clipboard_payload(native: bool, run_input: &str) -> String {
    if native {
        storage::decode_b64(run_input)
    } else {
        run_input.to_string()
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
        assert_eq!(reloaded.aliases.len(), 4, "br, cd, app and t round-trip");
        let t = reloaded.aliases.iter().find(|d| d.name == "t").unwrap();
        assert_eq!(t.triggers, vec!["tt".to_string()]);
    }

    /// The highlighted shortcut row runs for a typed `<alias> <partial>`,
    /// never the literal partial. History outranks those shortcut rows, so a
    /// partial that also matches a recorded run puts that run on row 0.
    #[test]
    fn highlighted_shortcut_row_runs_for_a_typed_partial() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        type_str(&mut app, ":add t,tt printf %s {input}", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        type_str(&mut app, ":arg t baidu https://example.com/b", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        type_str(&mut app, ":arg t bing https://example.com/i", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);

        // "t b" lists both shortcuts: Enter runs the highlighted one, not
        // the literal partial.
        app.input = "t b".to_string();
        app.cursor = 0;
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert_eq!(app.status, Some((true, "t ok: baidu".to_string())));
        // History keeps the raw key so the shorthand stays replayable.
        assert_eq!(app.store.history[0].input(), "baidu");

        // Retyping the partial now also matches the run just recorded.
        // History outranks the alias-scoped shortcut rows, and the `baidu`
        // key row resolves to the same target as that run - it deduplicates
        // away, so one move down reaches `bing`.
        app.input = "t b".to_string();
        assert_eq!(
            state::candidates(&app),
            vec![
                Candidate::History { idx: 0 },
                Candidate::Shortcut {
                    alias: "t".to_string(),
                    key: "bing".to_string()
                },
            ],
            "history first; the duplicate key row is gone"
        );
        apply(&mut app, Action::MoveDown, Platform::Linux, &path);
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert_eq!(app.status, Some((true, "t ok: bing".to_string())));
    }

    #[test]
    fn selected_shortcut_row_records_the_key_not_the_value() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        type_str(&mut app, ":add t,tt printf %s {input}", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        type_str(&mut app, ":arg t baidu https://example.com/b", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);

        // "baidu" does not resolve as a head (it is a shortcut key, not a
        // trigger): the highlighted row decides and runs the mapped value...
        app.input = "baidu".to_string();
        app.cursor = 0;
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert_eq!(app.status, Some((true, "t ok: baidu".to_string())));
        // ...while history keeps the shorthand, exactly like the
        // resolving-head picker path.
        assert_eq!(app.store.history[0].input(), "baidu");
    }

    #[test]
    fn an_exactly_typed_shortcut_key_keeps_its_own_meaning() {
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
        assert_eq!(shortcut_override(&app, def, "baidu"), None);
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert_eq!(app.status, Some((true, "t ok: baidu".to_string())));
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

    #[test]
    fn a_backgrounded_launch_that_never_proves_success_records_no_history() {
        let (mut app, _dir, path) = setup();
        shown(&mut app);
        type_str(&mut app, ":add bg sleep 3 >/dev/null 2>&1 &", &path);
        apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
        app.input = "bg".to_string();
        apply(&mut app, Action::Execute, Platform::Linux, &path);
        assert!(
            matches!(&app.status, Some((true, msg)) if msg.contains("not recorded")),
            "status: {:?}",
            app.status
        );
        assert!(
            app.store.history.is_empty(),
            "an unproven launch must not enter history: {:?}",
            app.store.history
        );
        // It is not in the history, so losing the input would lose the run.
        assert_eq!(app.input, "bg", "input kept: {:?}", app.input);
    }

    #[test]
    fn clipboard_payload_decodes_native_input_once() {
        // A native clipboard alias carries base64: the clipboard gets the
        // decoded text...
        assert_eq!(clipboard_payload(true, "aGVsbG8="), "hello");
        // ...while invalid base64 (ordinary plain text) falls back to itself.
        assert_eq!(clipboard_payload(true, "hello world"), "hello world");
        // A replay re-decodes the recorded input, so any valid base64 decodes.
        assert_eq!(clipboard_payload(true, "TWFu"), "Man");
    }

    #[test]
    fn clipboard_payload_leaves_other_aliases_untouched() {
        assert_eq!(
            clipboard_payload(false, "https://example.com"),
            "https://example.com"
        );
    }

    #[test]
    fn app_input_is_never_base64_decoded() {
        // `app` uses the `@native app` backend, not the clipboard one, so even
        // an application name that happens to be valid base64 reaches the
        // launcher verbatim (the decode would silently rename the target).
        let def = crate::launch::default_def();
        for platform in [Platform::Linux, Platform::Macos] {
            let native_clipboard = exec::uses_native_clipboard(&def, platform);
            assert!(!native_clipboard, "app is not the clipboard backend");
            assert_eq!(clipboard_payload(native_clipboard, "aGVsbG8="), "aGVsbG8=");
        }
    }
}
