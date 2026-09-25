//! Binary entry point: terminal lifecycle, window fitting and the event loop
//! (with the ESC-tail guard so a split `ESC` + `d` still reads as `Alt+D`).
//! Command-line parsing and the fully headless `--set-*` / `--print-*`
//! helpers live in the `cli` module (see `src/cli.rs`).

use std::io::stdout;
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use std::time::Instant;

use anyhow::Result;
use crossterm::event::{self, Event, KeyEvent};
use crossterm::tty::IsTty;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use xconsoler::action;
use xconsoler::app;
use xconsoler::escguard::{self, EscGuard};
use xconsoler::fit;
use xconsoler::fit_tmux;
use xconsoler::keyspec::{self, KeySpec};
use xconsoler::platform;
use xconsoler::render;
use xconsoler::state::{self, App, Mode};
use xconsoler::storage;
use xconsoler::term::TerminalGuard;

mod cli;

use cli::{
    parse_args, run_print_app, run_print_bind, run_print_rows, run_set_command_key,
    run_set_wake_key, USAGE,
};

/// Tick cap: worst-case latency before a redraw / quit check.
const POLL_TIMEOUT: Duration = Duration::from_millis(250);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().skip(1).any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        std::process::exit(0);
    }
    // Detached child of the native clipboard backend (see src/clipboard.rs):
    // runs headless, never touches the terminal or the TUI.
    if args
        .iter()
        .skip(1)
        .any(|a| a == xconsoler::clipboard::SERVE_FLAG)
    {
        xconsoler::clipboard::serve();
        std::process::exit(0);
    }

    let cli = match parse_args(&args) {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("xconsoler: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    // Headless paths first: they must work without a terminal (SSH one-liners).
    if let Some(spec) = cli.set_wake_key.clone() {
        run_set_wake_key(&cli.store, &spec);
    }
    if let Some(spec) = cli.set_command_key.clone() {
        run_set_command_key(&cli.store, &spec);
    }
    if cli.print_bind {
        run_print_bind(&cli);
    }
    if cli.print_rows {
        run_print_rows(&cli.store);
    }
    if let Some(name) = cli.print_app.clone() {
        run_print_app(&name);
    }

    // zle widgets (zsh) run external commands with stdin redirected from
    // /dev/null: accept that as long as a controlling terminal exists --
    // crossterm falls back to /dev/tty itself when stdin is not a tty.
    if !std::io::stdin().is_tty()
        && std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .is_err()
    {
        eprintln!("xconsoler: stdin is not a terminal; run me in an interactive shell");
        std::process::exit(2);
    }
    // Validate before entering raw mode so errors stay readable.
    let wake_override = match cli.wake_key.as_deref().map(keyspec::parse) {
        Some(Ok(k)) => Some(k),
        Some(Err(e)) => {
            eprintln!("xconsoler: {e}");
            std::process::exit(2);
        }
        None => None,
    };
    let command_override = match cli.command_key.as_deref().map(keyspec::parse) {
        Some(Ok(k)) => Some(k),
        Some(Err(e)) => {
            eprintln!("xconsoler: {e}");
            std::process::exit(2);
        }
        None => None,
    };

    // The guard is dropped inside the closure, so the terminal is always
    // restored before the error is printed to stderr below.
    let result = (|| -> Result<()> {
        let _guard = TerminalGuard::enter()?;
        run(&cli.store, cli.summon, wake_override, command_override)
    })();
    if let Err(e) = result {
        eprintln!("xconsoler: {e:#}");
        std::process::exit(1);
    }
}

/// Whether the event loop hands `key` to the `/settings` page. The wake
/// hotkey stays global even while that page owns the mode: falling through
/// to `action::on_key` lets it hide (or quit, in summon mode) the bar from
/// any page instead of hitting the page's plain-char arms (Alt+D used to
/// delete the selection). The command key is deliberately *not* exempt - it
/// would open the palette over the settings page.
fn settings_page_owns(app: &App, key: &KeyEvent) -> bool {
    matches!(&app.mode, Mode::Settings(_)) && !keyspec::matches(&app.wake, key)
}

/// The event loop: draw (every iteration, hidden included), poll, dispatch.
/// `wake_override` / `command_override` replace the keys for this run only —
/// they never touch `app.store`, so the store saved at exit keeps the
/// configured keys.
fn run(
    path: &Path,
    summon: bool,
    wake_override: Option<KeySpec>,
    command_override: Option<KeySpec>,
) -> Result<()> {
    let pf = platform::current();
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut app: App = state::new(storage::load(path), summon);
    if let Some(k) = wake_override {
        app.wake = k;
    }
    if let Some(k) = command_override {
        app.command = k;
    }
    let mut guard = EscGuard::new();

    // Window auto-fit, resolved once: the bar keeps one session height
    // unless `XC_ROWS` pinned it or `XC_NO_FIT` opted out, so typing only
    // changes the candidate list's contents, never the window. Inside a tmux
    // pane the request travels through the passthrough envelope and targets
    // the outer window; the X11 fallback covers the terminals that ignore the
    // in-band resize escape.
    let env = |key: &str| std::env::var_os(key).and_then(|v| v.into_string().ok());
    let fit_on = fit::enabled(env("XC_ROWS").as_deref(), env("XC_NO_FIT").as_deref());
    let pane = if fit_on {
        fit_tmux::session(env("TMUX").as_deref(), env("TMUX_PANE").as_deref())
    } else {
        None
    };
    let fit = Fit {
        on: fit_on,
        wm: fit_on
            && pane.is_none()
            && fit::wm_resize_applies(
                env("DISPLAY").as_deref(),
                env("WAYLAND_DISPLAY").as_deref(),
                fit::which("xdotool", env("PATH").as_deref()).is_some(),
            ),
        pane,
    };
    // tmux 3.3+ forwards pane passthrough only while the window option
    // `allow-passthrough` is on: switch it on for this window, remembering
    // what to put back at exit.
    let passthrough = fit.pane.as_deref().and_then(fit_tmux::allow_passthrough);
    // Height the window had at startup (and the last one the user picked):
    // the size the fit restores on the way out.
    let mut baseline = crossterm::terminal::size()?.1;
    // The one height the normal bar asks for all session; `--print-rows` and
    // `scripts/xc-bar` compute the launch geometry with the same rule.
    let stable_rows = fit::stable_rows(&app);
    let mut pending: Option<Asked> = None;

    loop {
        fit_window(&fit, &mut pending, &app, stable_rows, Instant::now())?;
        terminal.draw(|f| render::draw(f, &app))?;
        if event::poll(POLL_TIMEOUT)? {
            match event::read()? {
                // A resize from outside - the user dragging the window, or the
                // terminal applying our own request - wins over the fit. A size
                // the user picked becomes the one restored at exit; our own
                // request is the one that reports the height we asked for.
                Event::Resize(_, rows) => {
                    if rows != fit::desired_rows(&app, stable_rows) {
                        baseline = rows;
                    }
                    fit_adopt(&mut pending);
                }
                Event::Key(key) => {
                    let mut events = guard.feed(key);
                    events.extend(resolve_held(&mut guard)?);
                    for key in events {
                        if settings_page_owns(&app, &key) {
                            // The settings page owns its keys; its pure transition
                            // returns the new state + store (applied in place).
                            app::apply_settings(&mut app, key, path);
                        } else {
                            // Key release/repeat is filtered inside `on_key`.
                            let act = action::on_key(&app, key);
                            app::apply(&mut app, act, pf, path);
                        }
                    }
                }
                // Mouse events and friends: fall through, redraw above.
                _ => {}
            }
        }
        if app.quit {
            break;
        }
    }
    fit_restore(&fit, baseline);
    if let Some(pane) = fit.pane.as_deref() {
        fit_tmux::restore_passthrough(pane, passthrough.as_deref());
    }
    storage::save(path, &app.store)?;
    Ok(())
}

/// How the window fit reaches its terminal, resolved once before the loop.
struct Fit {
    /// Fit enabled: not pinned via `XC_ROWS`, not opted out via `XC_NO_FIT`.
    on: bool,
    /// X11 fallback available: `xdotool` on `PATH`, an X display, no Wayland.
    /// Never inside tmux, where the hosting window cannot be proven to be ours
    /// and the passthrough envelope is the channel that gets out anyway.
    wm: bool,
    /// `Some(pane)` when the bar runs inside a tmux pane: the resize escape
    /// needs the DCS passthrough envelope and the outer window needs tmux's
    /// own chrome (the status line) added to the pane height.
    pane: Option<String>,
}

/// One height the fit is working on: what it wants, how many asks that
/// took, and when the last one went out - the input of [`fit::may_ask`].
/// `tries == fit::MAX_ASKS` parks the entry, whether because the window
/// arrived at the wanted height, because a resize from outside won, or
/// because the ask budget is spent.
#[derive(Clone, Copy, Debug)]
struct Asked {
    want: u16,
    tries: u8,
    at: Instant,
}

/// Move the terminal to the height the bar wants for the session. `pending`
/// is the
/// height the fit last asked for; it is repeated until the window reports that
/// height, because terminals apply the request asynchronously and a WM may not
/// have activated the window on the very first frames. [`fit::may_ask`] paces
/// the retries and caps them, so a terminal that ignores the mechanisms is not
/// asked once per tick forever. The request keeps the current width: only the
/// height is the bar's business. Best effort, a failing `xdotool` must never
/// take the bar down, and the X11 fallback only fires for a focused window
/// that provably belongs to this process tree.
fn fit_window(
    fit: &Fit,
    pending: &mut Option<Asked>,
    app: &App,
    stable_rows: u16,
    now: Instant,
) -> Result<()> {
    if !fit.on {
        return Ok(());
    }
    let want = fit::desired_rows(app, stable_rows);
    let mut asked = match *pending {
        Some(asked) if asked.want == want => asked,
        // A new wanted height: ask right away, whatever the last one cost.
        _ => Asked {
            want,
            tries: 0,
            at: now,
        },
    };
    if !fit::may_ask(asked.tries, now.duration_since(asked.at)) {
        *pending = Some(asked);
        return Ok(());
    }
    let (cols, rows) = crossterm::terminal::size()?;
    if cols == 0 {
        // The pty has no width yet (the far end has not reported its window
        // size). Resizing to zero columns is never what the bar wants, so ask
        // again on a later tick instead.
        return Ok(());
    }
    if rows == want {
        // Already there (or the terminal just honoured the request): stop.
        asked.tries = fit::MAX_ASKS;
        *pending = Some(asked);
        return Ok(());
    }
    let escape = match &fit.pane {
        // tmux swallows the escape in-band: hand it to the outer terminal
        // through the passthrough envelope, asking for the pane height plus
        // tmux's own chrome.
        Some(pane) => match fit_tmux::outer_request(pane, want) {
            Some(outer) => fit_tmux::passthrough(&fit::resize_escape(outer, cols)),
            // A split pane shares the window with its siblings: not ours to
            // resize, and not worth asking again until the wanted height
            // changes (the ask budget parks it).
            None => {
                asked.tries = fit::MAX_ASKS;
                *pending = Some(asked);
                return Ok(());
            }
        },
        None => fit::resize_escape(want, cols),
    };
    let mut out = stdout();
    out.write_all(escape.as_bytes())?;
    out.flush()?;
    if fit.wm && focused_window_is_ours() {
        let argv = fit::wm_resize_argv(want, cols);
        let _ = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .status();
    }
    asked.tries += 1;
    asked.at = now;
    *pending = Some(asked);
    Ok(())
}

/// Undo the fit on the way out: leave the window at the height it started
/// with, or at the one the user last picked if they resized it mid-run. Only
/// the height was ever the fit's business, so a window that never changed is
/// left alone, and a terminal that fails us at exit is not an error worth
/// reporting - the bar is quitting either way.
fn fit_restore(fit: &Fit, baseline: u16) {
    if !fit.on {
        return;
    }
    let Ok((cols, rows)) = crossterm::terminal::size() else {
        return;
    };
    // No width known (or never resized): leave the window exactly as it is.
    if cols == 0 || rows == baseline {
        return;
    }
    let escape = match &fit.pane {
        Some(pane) => match fit_tmux::outer_request(pane, baseline) {
            Some(outer) => fit_tmux::passthrough(&fit::resize_escape(outer, cols)),
            None => return,
        },
        None => fit::resize_escape(baseline, cols),
    };
    let mut out = stdout();
    let _ = out.write_all(escape.as_bytes());
    let _ = out.flush();
    if fit.wm && focused_window_is_ours() {
        let argv = fit::wm_resize_argv(baseline, cols);
        let _ = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .status();
    }
}

/// Pid of the X11 window that currently has input focus, via the chained
/// `xdotool getactivewindow getwindowpid`. `None` on any spawn failure,
/// non-zero exit (no active window, or no `_NET_WM_PID`) or a stdout that
/// does not parse as a pid.
#[cfg(unix)]
fn focused_window_pid() -> Option<u32> {
    let out = std::process::Command::new("xdotool")
        .args(["getactivewindow", "getwindowpid"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    std::str::from_utf8(&out.stdout).ok()?.trim().parse().ok()
}

/// Whether the focused X11 window belongs to this process tree. Unprovable
/// ownership (no pid, unreadable `/proc`, no ancestry link) is `false`: the
/// fallback must never resize a window that is not ours.
#[cfg(unix)]
fn focused_window_is_ours() -> bool {
    match focused_window_pid() {
        Some(pid) => fit::pid_in_ancestry(pid, std::process::id(), fit::parent_pid),
        None => false,
    }
}

#[cfg(not(unix))]
fn focused_window_is_ours() -> bool {
    false
}

/// A resize that came from outside (the user dragging the window, or the
/// terminal applying our own request) wins: park the counter so the bar stops
/// asking for the old height, and refit as soon as the candidate set wants a
/// different one.
fn fit_adopt(pending: &mut Option<Asked>) {
    if let Some(asked) = pending.as_mut() {
        asked.tries = fit::MAX_ASKS;
    }
}

/// After [`EscGuard::feed`] held a lone Esc, wait up to
/// [`escguard::HOLD_MS`] for the follow-up key: a plain char is merged into
/// `ALT+char` (the split-write Alt+D repair), any other key releases the
/// held Esc first, and a timeout flushes the Esc as a real Esc press.
fn resolve_held(guard: &mut EscGuard) -> Result<Vec<KeyEvent>> {
    let mut out = Vec::new();
    while guard.is_holding() {
        if event::poll(Duration::from_millis(escguard::HOLD_MS))? {
            if let Event::Key(key) = event::read()? {
                out.extend(guard.feed(key));
            } else {
                // Non-key event (resize/...): stop holding, redraw up top.
                out.extend(guard.flush());
            }
        } else {
            out.extend(guard.flush());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_key_stays_global_on_the_settings_page() {
        use crossterm::event::KeyCode;

        let mut app = state::new(storage::Store::default(), false);
        app.mode = Mode::Settings(Box::new(xconsoler::settings::new()));
        let d = |mods| KeyEvent::new(KeyCode::Char('d'), mods);

        // The wake key falls through to the launcher even on the page.
        assert!(!settings_page_owns(
            &app,
            &d(crossterm::event::KeyModifiers::ALT)
        ));
        // Plain keys (and any other combo) stay with the page.
        assert!(settings_page_owns(
            &app,
            &d(crossterm::event::KeyModifiers::NONE)
        ));
        // The command key is not exempt: opening the palette over the page
        // would leave the two screens fighting.
        app.command = keyspec::parse("ctrl+k").unwrap();
        assert!(settings_page_owns(
            &app,
            &KeyEvent::new(KeyCode::Char('k'), crossterm::event::KeyModifiers::CONTROL)
        ));
        // Normal mode never routes to the page.
        app.mode = Mode::Normal;
        assert!(!settings_page_owns(
            &app,
            &d(crossterm::event::KeyModifiers::NONE)
        ));
    }
}
