//! Binary entry point: arg parsing, headless helpers (`--print-bind`,
//! `--set-wake-key`), terminal lifecycle, and the event loop (with the
//! ESC-tail guard so a split `ESC` + `d` still reads as `Alt+D`).

use std::io::stdout;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyEvent};
use crossterm::tty::IsTty;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use xconsoler::action;
use xconsoler::app;
use xconsoler::escguard::{self, EscGuard};
use xconsoler::keyspec::{self, KeySpec};
use xconsoler::platform;
use xconsoler::render;
use xconsoler::state::{self, App, Mode};
use xconsoler::storage;
use xconsoler::term::TerminalGuard;

const USAGE: &str = "\
xconsoler — long-bar keyboard launcher (alt+d to wake)

USAGE:
    xconsoler [OPTIONS]

OPTIONS:
    --summon              shell-keybind mode: starts shown, wake key / Esc quits
    --wake-key <spec>     wake key for this run only (e.g. alt+d, ctrl+g);
                          also feeds --print-bind; not persisted
    --set-wake-key <spec> save the wake key to the store, print a reminder to
                          re-generate the shell binding, then exit (no TUI)
    --command-key <spec>  command-palette key for this run only
                          (default: the wake key); not persisted
    --set-command-key <spec>
                          save the command-palette key to the store
                          (default: the wake key), then exit (no TUI)
    --print-bind          print the shell binding line for the wake key, exit
    --shell <bash|zsh>    shell flavour for --print-bind (default: bash)
    --store <path>        store.json location (default: <config_dir>/xconsoler/store.json)
    -h, --help            show this help";

/// Tick cap: worst-case latency before a redraw / quit check.
const POLL_TIMEOUT: Duration = Duration::from_millis(250);

/// Parsed command line (plain data).
#[derive(Debug)]
struct Cli {
    store: PathBuf,
    summon: bool,
    wake_key: Option<String>,
    set_wake_key: Option<String>,
    command_key: Option<String>,
    set_command_key: Option<String>,
    print_bind: bool,
    shell: String,
}

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

/// Parse argv (skipping argv[0]); `-h/--help` is handled by the caller.
fn parse_args(args: &[String]) -> Result<Cli, String> {
    let mut cli = Cli {
        store: storage::default_path(),
        summon: false,
        wake_key: None,
        set_wake_key: None,
        command_key: None,
        set_command_key: None,
        print_bind: false,
        shell: "bash".to_string(),
    };
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--store" => cli.store = PathBuf::from(next_value(args, &mut i, "--store")?),
            "--summon" => cli.summon = true,
            "--wake-key" => cli.wake_key = Some(next_value(args, &mut i, "--wake-key")?),
            "--set-wake-key" => {
                cli.set_wake_key = Some(next_value(args, &mut i, "--set-wake-key")?)
            }
            "--command-key" => cli.command_key = Some(next_value(args, &mut i, "--command-key")?),
            "--set-command-key" => {
                cli.set_command_key = Some(next_value(args, &mut i, "--set-command-key")?)
            }
            "--print-bind" => cli.print_bind = true,
            "--shell" => {
                let shell = next_value(args, &mut i, "--shell")?;
                if shell != "bash" && shell != "zsh" {
                    return Err(format!(
                        "unsupported shell {shell:?} (use \"bash\" or \"zsh\")"
                    ));
                }
                cli.shell = shell;
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        i += 1;
    }
    Ok(cli)
}

/// Consume the value following a flag at index `i` (advancing `i`).
fn next_value(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

/// `--set-wake-key`: validate, persist, print confirmation + a reminder to
/// re-generate the binding. Fully headless; always exits.
fn run_set_wake_key(path: &Path, spec: &str) -> ! {
    let mut store = storage::load(path);
    if let Err(e) = storage::set_wake_key(&mut store, spec) {
        eprintln!("xconsoler: {e}");
        std::process::exit(2);
    }
    if let Err(e) = storage::save(path, &store) {
        eprintln!("xconsoler: {e:#}");
        std::process::exit(1);
    }
    println!("wake key saved: {}", store.config.wake_key);
    println!("re-generate your shell binding and reload your rc file, e.g.:");
    println!("  xconsoler --print-bind --shell bash");
    std::process::exit(0);
}

/// `--set-command-key`: validate, persist, print confirmation. Fully
/// headless; always exits.
fn run_set_command_key(path: &Path, spec: &str) -> ! {
    let mut store = storage::load(path);
    if let Err(e) = storage::set_command_key(&mut store, spec) {
        eprintln!("xconsoler: {e}");
        std::process::exit(2);
    }
    if let Err(e) = storage::save(path, &store) {
        eprintln!("xconsoler: {e:#}");
        std::process::exit(1);
    }
    println!("command key saved: {}", store.config.command_key);
    if store.config.command_key == store.config.wake_key {
        println!(
            "note: this equals the wake key - while hidden / in summon mode the \
             wake key keeps priority"
        );
    }
    std::process::exit(0);
}

/// `--print-bind`: key = `--wake-key` override, else the stored config.
/// Fully headless; always exits.
fn run_print_bind(cli: &Cli) -> ! {
    let store = storage::load(&cli.store);
    let spec = cli.wake_key.as_deref().unwrap_or(&store.config.wake_key);
    let key = match keyspec::parse(spec) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("xconsoler: {e}");
            std::process::exit(2);
        }
    };
    match build_bind_lines(&cli.shell, &key) {
        Ok(lines) => {
            print!("{lines}");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("xconsoler: {e}");
            std::process::exit(2);
        }
    }
}

/// Shell binding text for a wake key (`bash` / `zsh`), newline-terminated.
/// The `\ed` / `\C-g` sequences are the literal readline spellings produced
/// by [`keyspec::readline_seq`].
fn build_bind_lines(shell: &str, spec: &KeySpec) -> Result<String, String> {
    let seq = keyspec::readline_seq(spec);
    match shell {
        "bash" => Ok(format!(
            "bind -x '\"{seq}\": \"xconsoler --summon\"' 2>/dev/null || true\n"
        )),
        "zsh" => Ok(format!(
            "xconsoler_invoke() {{ zle -I; xconsoler --summon \"$@\" </dev/tty; zle reset-prompt }}\n\
             zle -N xconsoler_invoke\n\
             bindkey '{seq}' xconsoler_invoke\n"
        )),
        other => Err(format!("unsupported shell {other:?} (use \"bash\" or \"zsh\")")),
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

    loop {
        terminal.draw(|f| render::draw(f, &app))?;
        if event::poll(POLL_TIMEOUT)? {
            if let Event::Key(key) = event::read()? {
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
            // Event::Resize (and mouse events): fall through, redraw above.
        }
        if app.quit {
            break;
        }
    }
    storage::save(path, &app.store)?;
    Ok(())
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

    fn args(list: &[&str]) -> Vec<String> {
        std::iter::once("xconsoler".to_string())
            .chain(list.iter().map(|s| s.to_string()))
            .collect()
    }

    #[test]
    fn bash_bind_line_for_alt_d() {
        let k = keyspec::parse("alt+d").unwrap();
        assert_eq!(
            build_bind_lines("bash", &k).unwrap(),
            "bind -x '\"\\ed\": \"xconsoler --summon\"' 2>/dev/null || true\n"
        );
    }

    #[test]
    fn bash_bind_line_for_ctrl_g() {
        let k = keyspec::parse("ctrl+g").unwrap();
        assert_eq!(
            build_bind_lines("bash", &k).unwrap(),
            "bind -x '\"\\C-g\": \"xconsoler --summon\"' 2>/dev/null || true\n"
        );
    }

    #[test]
    fn zsh_bind_lines() {
        let k = keyspec::parse("alt+d").unwrap();
        assert_eq!(
            build_bind_lines("zsh", &k).unwrap(),
            "xconsoler_invoke() { zle -I; xconsoler --summon \"$@\" </dev/tty; zle reset-prompt }\n\
             zle -N xconsoler_invoke\n\
             bindkey '\\ed' xconsoler_invoke\n"
        );
    }

    #[test]
    fn unsupported_shell_is_rejected() {
        let k = keyspec::parse("alt+d").unwrap();
        assert!(build_bind_lines("fish", &k).is_err());
    }

    #[test]
    fn parse_args_defaults() {
        let cli = parse_args(&args(&[])).unwrap();
        assert_eq!(cli.store, storage::default_path());
        assert!(!cli.summon);
        assert_eq!(cli.wake_key, None);
        assert_eq!(cli.set_wake_key, None);
        assert_eq!(cli.command_key, None);
        assert_eq!(cli.set_command_key, None);
        assert!(!cli.print_bind);
        assert_eq!(cli.shell, "bash");
    }

    #[test]
    fn parse_args_all_flags() {
        let cli = parse_args(&args(&[
            "--summon",
            "--wake-key",
            "ctrl+g",
            "--print-bind",
            "--shell",
            "zsh",
            "--store",
            "/tmp/s.json",
        ]))
        .unwrap();
        assert!(cli.summon);
        assert_eq!(cli.wake_key.as_deref(), Some("ctrl+g"));
        assert!(cli.print_bind);
        assert_eq!(cli.shell, "zsh");
        assert_eq!(cli.store, PathBuf::from("/tmp/s.json"));
    }

    #[test]
    fn parse_args_set_wake_key() {
        let cli = parse_args(&args(&["--set-wake-key", "alt+j"])).unwrap();
        assert_eq!(cli.set_wake_key.as_deref(), Some("alt+j"));
    }

    #[test]
    fn parse_args_command_keys() {
        let cli = parse_args(&args(&[
            "--command-key",
            "ctrl+k",
            "--set-command-key",
            "alt+j",
        ]))
        .unwrap();
        assert_eq!(cli.command_key.as_deref(), Some("ctrl+k"));
        assert_eq!(cli.set_command_key.as_deref(), Some("alt+j"));
    }

    #[test]
    fn parse_args_rejects_bad_input() {
        assert!(parse_args(&args(&["--store"])).is_err());
        assert!(parse_args(&args(&["--wake-key"])).is_err());
        assert!(parse_args(&args(&["--set-wake-key"])).is_err());
        assert!(parse_args(&args(&["--command-key"])).is_err());
        assert!(parse_args(&args(&["--set-command-key"])).is_err());
        assert!(parse_args(&args(&["--shell", "fish"])).is_err());
        assert!(parse_args(&args(&["--shell"])).is_err());
        assert!(parse_args(&args(&["--bogus"])).is_err());
    }

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
