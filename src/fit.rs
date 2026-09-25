//! Window auto-fit: the bar's frame is sized once per session and then stays
//! put - the height carries the stored history (what the empty bar lists) and
//! is never smaller than the typed candidate set, so typing a query only
//! changes the list's *contents*, never the window around it. The command
//! palette keeps one fixed height for its whole open session (its fuzzy
//! filter must not resize the frame per keystroke); only `/settings`, a full
//! page rather than a bar, asks for a different window.
//!
//! `scripts/xc-bar` picks the *launch* geometry with the same rule (see
//! `--print-rows`), so the first frame already has the session height; this
//! module pins it there afterwards.
//! Two mechanisms, both best effort, and a retry policy ([`may_ask`]) that
//! covers a window which is still being mapped or focused - where the first
//! request goes nowhere - without asking a terminal that ignores the request
//! on every event-loop tick forever:
//!
//! * [`resize_escape`] - xterm's `CSI 8 ; rows ; cols t`. Honouring it is a
//!   terminal policy: xterm needs `allowWindowOps`, kitty/wezterm/contour do
//!   it out of the box, VTE (gnome-terminal) and alacritty ignore window ops
//!   altogether.
//! * [`wm_resize_argv`] - on X11 with `xdotool` around, resize the focused
//!   window in *character cells*: `windowsize --usehints` converts rows/cols
//!   through the terminal's own WM size hints, which covers exactly the
//!   terminals that ignore the escape. "The focused window" is not
//!   necessarily ours though, so this path is guarded: [`pid_in_ancestry`]
//!   must prove the focused window's pid sits above us on the parent chain
//!   (the hosting terminal is our ancestor, not our child) before `xdotool`
//!   is allowed to touch it. If ownership cannot be proven, the fallback is
//!   skipped; the escape above still runs.
//!
//! Every run fits its window: a plain `xconsoler` shrinks the terminal it was
//! started from, a summoned bar the window it owns, and the size the window had
//! at startup comes back on the way out (the event loop keeps that baseline).
//! `XC_ROWS` (a pinned height) and `XC_NO_FIT` turn the whole thing off; in a
//! tmux pane the request takes the passthrough envelope ([`crate::fit_tmux`]).

use std::time::Duration;

use crate::commands;
use crate::state::{self, App, Mode};

/// How many event-loop ticks a height request is repeated. Terminals apply
/// the request asynchronously and a window manager may not have activated the
/// window on the very first frame, so one shot is not enough; a terminal that
/// ignores both mechanisms stops being asked after this many tries instead of
/// once per tick forever.
pub const MAX_TRIES: u8 = 3;

/// Pace of the retries after [`MAX_TRIES`]: a window that is not mapped or
/// focused yet answers within a second or two, while a terminal that never
/// answers only costs one request every this often.
pub const RETRY_AFTER: Duration = Duration::from_secs(2);

/// Total asks spent on one height, quick tries included. Past this the fit
/// gives up on that height until the candidate set wants a different one.
pub const MAX_ASKS: u8 = 8;

/// Rows the `/settings` page asks for. It is a form, not a bar, so it gets a
/// fixed comfortable height instead of one derived from a candidate count.
pub const SETTINGS_ROWS: u16 = 24;

/// Rows the normal bar keeps for the whole session: room for the stored
/// history an empty bar lists, and never less than the typed candidate set,
/// so neither typing nor replaying changes the frame's height.
pub fn stable_rows(app: &App) -> u16 {
    state::bar_rows(app.store.history.len().max(state::CANDIDATE_LIMIT))
}

/// Rows the window should have for `app` right now.
///
/// The launcher reuses [`state::bar_rows`] - box, list frame, one row per
/// entry, status row, capped by `state::MAX_BAR_ROWS` - so the summoned
/// height and the fitted height can never disagree about the geometry.
/// Normal and hidden modes keep the session's [`stable_rows`]; the palette
/// keeps one fixed height of its own, because its fuzzy filter changes the
/// visible rows, not the window, and `/settings` is the one page that asks
/// for its full height.
pub fn desired_rows(app: &App, stable: u16) -> u16 {
    if matches!(app.mode, Mode::Settings(_)) {
        return SETTINGS_ROWS;
    }
    if app.palette.is_some() {
        return stable.max(state::bar_rows(commands::len()));
    }
    stable
}

/// Whether the fit may ask for its height again: `tries` asks went out, the
/// last one `since` ago. The first [`MAX_TRIES`] go back to back (the terminal
/// applies a request asynchronously), then [`RETRY_AFTER`] spaces them out
/// until [`MAX_ASKS`] gives up on this height. Pure; the caller owns the
/// counters.
pub fn may_ask(tries: u8, since: Duration) -> bool {
    tries < MAX_TRIES || (tries < MAX_ASKS && since >= RETRY_AFTER)
}

/// Whether this run may resize its own window: no `XC_ROWS` pin to respect
/// and not opted out via `XC_NO_FIT`. A summoned bar and a plain run both fit;
/// only the window they resize differs.
pub fn enabled(rows_pin: Option<&str>, no_fit: Option<&str>) -> bool {
    rows_pin.is_none_or(|v| v.trim().is_empty()) && !truthy(no_fit)
}

/// Whether the X11 fallback applies: `xdotool` on `PATH`, an X display, and
/// no Wayland session (where the fallback would hit the wrong window at
/// best, and there is no active-window concept at worst).
pub fn wm_resize_applies(display: Option<&str>, wayland: Option<&str>, xdotool: bool) -> bool {
    xdotool && truthy(display) && !truthy(wayland)
}

/// The parent pid of `pid`, read from the `PPid:` line of
/// `/proc/<pid>/status`. `None` on any error (no `/proc`, unreadable or
/// malformed file), which the ownership guard treats as "not provable".
#[cfg(unix)]
pub fn parent_pid(pid: u32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("PPid:"))
        .and_then(|value| value.trim().parse().ok())
}

/// Whether `target` is `self_pid` or one of its ancestors, asked through
/// `parent_of`: the walk starts at `self_pid` and climbs, because a window
/// owned by the hosting terminal is our *parent*, not our child. Capped at
/// eight links and stopped by a missing parent, so cycles or a garbage chain
/// cannot hang the caller; a target not reached within the cap counts as not
/// ours. Pure: the only I/O is whatever `parent_of` does.
pub fn pid_in_ancestry(target: u32, self_pid: u32, parent_of: impl Fn(u32) -> Option<u32>) -> bool {
    const MAX_LINKS: u8 = 8;
    let mut current = self_pid;
    for _ in 0..MAX_LINKS {
        if current == target {
            return true;
        }
        current = match parent_of(current) {
            Some(parent) => parent,
            None => return false,
        };
    }
    current == target
}

/// xterm's resize-window sequence, keeping the current width: only the
/// height is the bar's business.
pub fn resize_escape(rows: u16, cols: u16) -> String {
    format!("\x1b[8;{rows};{cols}t")
}

/// argv for the X11 fallback: resize the focused window to `cols` x `rows`
/// character cells. `argv[0]` is the program to spawn.
pub fn wm_resize_argv(rows: u16, cols: u16) -> Vec<String> {
    ["xdotool", "getactivewindow", "windowsize", "--usehints"]
        .into_iter()
        .map(str::to_string)
        .chain([cols.to_string(), rows.to_string()])
        .collect()
}

/// `Some(name)` when an executable of that name sits on `PATH`.
pub fn which(name: &str, path: Option<&str>) -> Option<String> {
    let path = path?;
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|p| is_executable(p))
        .map(|p| p.to_string_lossy().into_owned())
}

/// A non-empty value that is not an explicit "off" (`0`, `false`, `no`).
fn truthy(value: Option<&str>) -> bool {
    match value.map(str::trim) {
        None | Some("") => false,
        Some(v) => !matches!(v, "0" | "false" | "no"),
    }
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alias::{self, AliasDef};
    use crate::state::Visibility;
    use crate::storage::{HistoryEntry, Store};

    fn app(history: usize) -> App {
        let store = Store {
            aliases: with_shortcuts(),
            history: (0..history)
                .map(|i| HistoryEntry::new("br", &format!("baidu {i}"), 1_700 + i as u64))
                .collect(),
            ..Store::default()
        };
        let mut app = state::new(store, true);
        app.aliases = with_shortcuts();
        app
    }

    fn with_shortcuts() -> Vec<AliasDef> {
        let mut def = alias::defaults().remove(0); // br (seeded default)
        def.shortcuts.clear();
        def.shortcuts
            .insert("baidu".to_string(), "https://www.baidu.com".to_string());
        def.shortcuts
            .insert("bing".to_string(), "https://www.bing.com".to_string());
        vec![def]
    }

    #[test]
    fn stable_rows_keeps_room_for_history_and_typed_candidates() {
        // Room for CANDIDATE_LIMIT typed candidates even with no history...
        assert_eq!(
            stable_rows(&app(0)),
            state::bar_rows(state::CANDIDATE_LIMIT)
        );
        assert_eq!(
            stable_rows(&app(3)),
            state::bar_rows(state::CANDIDATE_LIMIT)
        );
        // ...and for the stored history once it is the larger side.
        assert_eq!(stable_rows(&app(6)), state::bar_rows(6));
        assert_eq!(stable_rows(&app(10)), state::MAX_BAR_ROWS);
        // The launch geometry (`--print-rows`) uses the same rule, so the
        // first frame already has this height.
    }

    #[test]
    fn desired_rows_ignores_the_live_candidates() {
        let mut a = app(1); // history: `br baidu 0`
        let stable = stable_rows(&a);
        a.input = "br b".to_string();
        // History first, then the alias's `baidu`/`bing` rows.
        assert_eq!(state::candidates(&a).len(), 3);
        assert_eq!(desired_rows(&a, stable), stable);
        a.input = "zzz".to_string();
        assert_eq!(state::candidates(&a).len(), 0);
        assert_eq!(desired_rows(&a, stable), stable, "no matches: same frame");
    }

    #[test]
    fn palette_height_is_fixed_and_settings_gets_its_page() {
        let mut a = app(0);
        let stable = stable_rows(&a);
        a.palette = Some(0);
        let palette = desired_rows(&a, stable);
        assert_eq!(palette, stable.max(state::bar_rows(commands::len())));
        // The fuzzy filter shrinks the visible rows, never the window.
        a.input = "/zz".to_string();
        assert_eq!(desired_rows(&a, stable), palette);
        a.palette = None;
        assert_eq!(desired_rows(&a, stable), stable);
        a.mode = Mode::Settings(Box::new(crate::settings::new()));
        assert_eq!(desired_rows(&a, stable), SETTINGS_ROWS);
    }

    #[test]
    fn hidden_bar_keeps_the_session_height() {
        let mut a = app(3);
        let stable = stable_rows(&a);
        a.visibility = Visibility::Hidden;
        assert_eq!(desired_rows(&a, stable), stable);
    }

    #[test]
    #[allow(clippy::assertions_on_constants)] // runtime guard on the constant
    fn tries_are_bounded() {
        assert!(MAX_TRIES > 1, "one shot loses the WM activation race");
        assert!(MAX_TRIES < 10, "an ignoring terminal must not be spammed");
    }

    #[test]
    fn escape_carries_rows_and_keeps_the_width() {
        assert_eq!(resize_escape(9, 52), "\x1b[8;9;52t");
    }

    #[test]
    fn asks_are_spaced_after_the_quick_tries() {
        assert!(may_ask(0, Duration::ZERO));
        assert!(
            may_ask(MAX_TRIES - 1, Duration::ZERO),
            "quick tries are back to back"
        );
        assert!(
            !may_ask(MAX_TRIES, Duration::from_millis(1)),
            "then they slow down"
        );
        assert!(
            may_ask(MAX_TRIES, RETRY_AFTER),
            "a window mapped late still answers"
        );
        assert!(may_ask(MAX_ASKS - 1, RETRY_AFTER));
        assert!(
            !may_ask(MAX_ASKS, RETRY_AFTER),
            "a terminal that ignores us is done"
        );
        assert!(!may_ask(MAX_ASKS, Duration::from_secs(600)));
    }

    #[test]
    fn an_unpinned_bar_may_resize() {
        assert!(enabled(None, None));
        assert!(!enabled(Some("9"), None), "XC_ROWS wins");
        assert!(enabled(Some("  "), None), "an empty pin is no pin");
        assert!(!enabled(None, Some("1")), "XC_NO_FIT=1");
        assert!(!enabled(None, Some("yes")));
        assert!(enabled(None, Some("0")), "XC_NO_FIT=0 is not an opt-out");
        assert!(enabled(None, Some("")));
    }

    #[test]
    fn x11_fallback_needs_xdotool_display_and_no_wayland() {
        assert!(wm_resize_applies(Some(":0"), None, true));
        assert!(!wm_resize_applies(Some(":0"), None, false), "no xdotool");
        assert!(!wm_resize_applies(None, None, true), "headless");
        assert!(
            !wm_resize_applies(Some(":0"), Some("wayland-0"), true),
            "Wayland owns its windows"
        );
    }

    #[test]
    fn pid_ancestry_accepts_self_parent_and_grandparent() {
        use std::collections::HashMap;
        // The bar (300) runs inside a terminal (200) that was started from a
        // session leader (100): both are above us on the parent chain.
        let parents: HashMap<u32, u32> = [(300, 200), (200, 100)].into_iter().collect();
        let parent_of = |pid| parents.get(&pid).copied();
        assert!(pid_in_ancestry(300, 300, parent_of), "self");
        assert!(pid_in_ancestry(200, 300, parent_of), "direct parent");
        assert!(pid_in_ancestry(100, 300, parent_of), "grandparent");
        assert!(!pid_in_ancestry(400, 300, parent_of), "stranger");
        assert!(
            !pid_in_ancestry(200, 100, parent_of),
            "children are not ancestors"
        );
    }

    #[test]
    fn pid_ancestry_survives_cycles_and_missing_parents() {
        use std::collections::HashMap;
        let cycle: HashMap<u32, u32> = [(1, 2), (2, 1)].into_iter().collect();
        let parent_of = |pid| cycle.get(&pid).copied();
        assert!(
            !pid_in_ancestry(99, 1, parent_of),
            "cycle never reaches the target"
        );
        assert!(
            !pid_in_ancestry(99, 7, |_| None),
            "missing parent stops at once"
        );
        assert!(!pid_in_ancestry(99, 7, |_| Some(7)), "self-loop terminates");
    }

    #[test]
    fn pid_ancestry_caps_the_walk_at_eight_links() {
        // parent(i) = i - 1: target 0 needs exactly 8 links from self 8.
        let parent_of = |pid: u32| if pid == 0 { None } else { Some(pid - 1) };
        assert!(
            pid_in_ancestry(0, 8, parent_of),
            "8 links are within the cap"
        );
        assert!(!pid_in_ancestry(0, 9, parent_of), "9 links are too far");
    }

    #[cfg(unix)]
    #[test]
    fn parent_pid_reads_our_own_status() {
        if !std::path::Path::new("/proc/self/status").exists() {
            return; // no procfs: the guard degrades to "not provable"
        }
        // The test process always has a parent; the parse must find it.
        assert!(parent_pid(std::process::id()).is_some());
        assert_eq!(parent_pid(u32::MAX), None, "no such process");
    }

    #[test]
    fn wm_argv_resizes_in_character_cells() {
        assert_eq!(
            wm_resize_argv(9, 52),
            [
                "xdotool",
                "getactivewindow",
                "windowsize",
                "--usehints",
                "52",
                "9"
            ]
            .map(str::to_string)
        );
    }

    #[test]
    fn which_finds_only_executables() {
        let dir = std::env::temp_dir().join("xc-fit-which-test");
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("xc-fake-tool");
        std::fs::write(&bin, "#!/bin/sh\nexit 0\n").unwrap();
        let path = dir.to_string_lossy().into_owned();
        assert_eq!(
            which("xc-fake-tool", Some(&path)),
            None,
            "not executable yet"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(
                which("xc-fake-tool", Some(&path)).as_deref(),
                Some(bin.to_str().unwrap())
            );
        }
        assert_eq!(which("xc-fake-tool", None), None, "no PATH");
        assert_eq!(which("definitely-not-installed-xyz", Some(&path)), None);
        std::fs::remove_file(&bin).ok();
    }
}
