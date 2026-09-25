//! Window auto-fit: once the bar is open, the terminal window follows the
//! candidate set - the history rows on an empty bar, the ranked candidates
//! while typing, the command palette while it is open, a full screen for
//! `/settings`.
//!
//! `scripts/xc-bar` picks the *launch* geometry from the stored history, so
//! the first frame is already right; this module keeps it right afterwards.
//! Two mechanisms, both best effort and both fired at most once per height
//! change (a terminal that ignores the request would otherwise get one per
//! event-loop tick):
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
//! Only a summoned bar resizes anything: a plain `xconsoler` run is a guest
//! in whatever terminal the user started it from. `XC_ROWS` (a pinned
//! height) and `XC_NO_FIT` turn the whole thing off.

use crate::commands;
use crate::state::{self, App, Mode, Visibility};

/// How many event-loop ticks a height request is repeated. Terminals apply
/// the request asynchronously and a window manager may not have activated the
/// window on the very first frame, so one shot is not enough; a terminal that
/// ignores both mechanisms stops being asked after this many tries instead of
/// once per tick forever.
pub const MAX_TRIES: u8 = 3;

/// Rows the `/settings` page asks for. It is a form, not a bar, so it gets a
/// fixed comfortable height instead of one derived from a candidate count.
pub const SETTINGS_ROWS: u16 = 24;

/// Rows the window should have for `app` right now.
///
/// The launcher reuses [`state::bar_rows`] - box, list frame, one row per
/// entry, status row, capped by `state::MAX_BAR_ROWS` - so the summoned
/// height and the fitted height can never disagree about the geometry.
pub fn desired_rows(app: &App) -> u16 {
    if matches!(app.mode, Mode::Settings(_)) {
        return SETTINGS_ROWS;
    }
    if app.visibility != Visibility::Shown {
        // A hidden bar is just the bare box; leave the window alone.
        return state::bar_rows(0);
    }
    // While the palette is open it replaces the candidate list, so its
    // commands are what need room.
    let items = match app.palette {
        Some(_) => commands::len(),
        None => state::candidates(app).len(),
    };
    state::bar_rows(items)
}

/// Whether this run may resize its own window: summoned (it owns that
/// window), no `XC_ROWS` pin to respect, and not opted out via `XC_NO_FIT`.
pub fn enabled(summon: bool, rows_pin: Option<&str>, no_fit: Option<&str>) -> bool {
    summon && rows_pin.is_none_or(|v| v.trim().is_empty()) && !truthy(no_fit)
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
        let mut def = alias::defaults().remove(0); // br (builtin)
        def.shortcuts.clear();
        def.shortcuts
            .insert("baidu".to_string(), "https://www.baidu.com".to_string());
        def.shortcuts
            .insert("bing".to_string(), "https://www.bing.com".to_string());
        vec![def]
    }

    #[test]
    fn rows_follow_the_history_on_an_empty_bar() {
        assert_eq!(desired_rows(&app(0)), 4, "box + status");
        assert_eq!(desired_rows(&app(1)), 7, "+ list frame + 1 row");
        assert_eq!(desired_rows(&app(3)), 9);
        assert_eq!(desired_rows(&app(10)), state::MAX_BAR_ROWS);
        // Same rule as the launch geometry: --print-rows and the fitted
        // height agree by construction.
        assert_eq!(desired_rows(&app(3)), state::bar_rows(3));
    }

    #[test]
    fn rows_follow_the_typed_candidates() {
        let mut a = app(1); // history: `br baidu 0`
        a.input = "br b".to_string();
        // History first, then the alias's `baidu`/`bing` rows.
        assert_eq!(state::candidates(&a).len(), 3);
        assert_eq!(desired_rows(&a), 9);
        a.input = "zzz".to_string();
        assert_eq!(state::candidates(&a).len(), 0);
        assert_eq!(desired_rows(&a), 4, "no matches: back to the bare box");
    }

    #[test]
    fn palette_and_settings_get_their_own_heights() {
        let mut a = app(0);
        a.palette = Some(0);
        assert_eq!(desired_rows(&a), state::bar_rows(commands::len()));
        assert!(desired_rows(&a) > 4);
        a.palette = None;
        a.mode = Mode::Settings(Box::new(crate::settings::new()));
        assert_eq!(desired_rows(&a), SETTINGS_ROWS);
    }

    #[test]
    fn hidden_bar_keeps_the_bare_box() {
        let mut a = app(3);
        a.visibility = Visibility::Hidden;
        assert_eq!(desired_rows(&a), 4);
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
    fn only_an_unpinned_summoned_bar_may_resize() {
        assert!(enabled(true, None, None));
        assert!(!enabled(false, None, None), "a plain run is a guest");
        assert!(!enabled(true, Some("9"), None), "XC_ROWS wins");
        assert!(enabled(true, Some("  "), None), "an empty pin is no pin");
        assert!(!enabled(true, None, Some("1")), "XC_NO_FIT=1");
        assert!(!enabled(true, None, Some("yes")));
        assert!(
            enabled(true, None, Some("0")),
            "XC_NO_FIT=0 is not an opt-out"
        );
        assert!(enabled(true, None, Some("")));
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
