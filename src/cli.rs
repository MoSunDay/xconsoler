//! Command-line parsing and the fully headless subcommands (`--set-*`,
//! `--print-*`). Split out of `main.rs` so the entry point stays within the
//! size budget: everything here must work without a terminal (SSH one-liners).

use std::path::{Path, PathBuf};

use anyhow::Result;

use xconsoler::keyspec::{self, KeySpec};
use xconsoler::launch;
use xconsoler::platform;
use xconsoler::state;
use xconsoler::storage;

pub const USAGE: &str = "\
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
    --print-rows          print the desktop bar height in rows for the store,
                          exit (used by scripts/xc-bar)
    --print-app <name>    resolve <name> against the installed applications and
                          print the match (label, source, exec), exit; nothing
                          is launched and no TUI starts
    --shell <bash|zsh>    shell flavour for --print-bind (default: bash)
    --store <path>        store.json location (default: $HOME/xconsoler/store.json)
    -h, --help            show this help";

/// Parsed command line (plain data).
#[derive(Debug)]
pub struct Cli {
    pub store: PathBuf,
    pub summon: bool,
    pub wake_key: Option<String>,
    pub set_wake_key: Option<String>,
    pub command_key: Option<String>,
    pub set_command_key: Option<String>,
    pub print_bind: bool,
    pub print_rows: bool,
    pub print_app: Option<String>,
    pub shell: String,
}

/// Parse argv (skipping argv[0]); `-h/--help` is handled by the caller.
pub fn parse_args(args: &[String]) -> Result<Cli, String> {
    let mut cli = Cli {
        store: storage::default_path(),
        summon: false,
        wake_key: None,
        set_wake_key: None,
        command_key: None,
        set_command_key: None,
        print_bind: false,
        print_rows: false,
        print_app: None,
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
            "--print-rows" => cli.print_rows = true,
            "--print-app" => cli.print_app = Some(next_value(args, &mut i, "--print-app")?),
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
pub fn next_value(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

/// `--set-wake-key`: validate, persist, print confirmation + a reminder to
/// re-generate the binding. Fully headless; always exits.
pub fn run_set_wake_key(path: &Path, spec: &str) -> ! {
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
pub fn run_set_command_key(path: &Path, spec: &str) -> ! {
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

/// `--print-rows`: desktop bar height for the stored history, one integer.
///
/// Same rule as [`fit::stable_rows`]: room for the history an empty bar
/// lists and never less than the typed candidate set, so the launch geometry
/// is also the height the bar keeps for the session. Fully headless; always
/// exits.
pub fn run_print_rows(path: &Path) -> ! {
    let store = storage::load(path);
    let visible = store.history.len().max(state::CANDIDATE_LIMIT);
    println!("{}", state::bar_rows(visible));
    std::process::exit(0);
}

/// `--print-app`: resolve a name against the installed applications and print
/// the match (nothing is launched). Read-only and headless; an empty,
/// unmatched or ambiguous name exits 2 so scripts can branch.
pub fn run_print_app(name: &str) -> ! {
    let name = name.trim();
    if name.is_empty() {
        eprintln!("xconsoler: --print-app requires a non-empty name");
        std::process::exit(2);
    }
    match launch::resolve(name, platform::current()) {
        launch::Match::One(target) => {
            println!("{}", launch::describe(&target));
            if let Some(exec) = target.exec.as_deref() {
                println!("exec: {exec}");
            }
            std::process::exit(0);
        }
        launch::Match::Ambiguous(labels) => {
            eprintln!("xconsoler: {}", launch::ambiguous_message(name, &labels));
            for label in &labels {
                eprintln!("  {label}");
            }
            std::process::exit(2);
        }
        launch::Match::None => {
            eprintln!("xconsoler: no app matches: {name}");
            std::process::exit(2);
        }
    }
}

/// `--print-bind`: key = `--wake-key` override, else the stored config.
/// Fully headless; always exits.
pub fn run_print_bind(cli: &Cli) -> ! {
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
pub fn build_bind_lines(shell: &str, spec: &KeySpec) -> Result<String, String> {
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
        assert!(!cli.print_rows);
        assert_eq!(cli.print_app, None);
        assert_eq!(cli.shell, "bash");
    }

    #[test]
    fn parse_args_all_flags() {
        let cli = parse_args(&args(&[
            "--summon",
            "--wake-key",
            "ctrl+g",
            "--print-bind",
            "--print-rows",
            "--print-app",
            "wechat",
            "--shell",
            "zsh",
            "--store",
            "/tmp/s.json",
        ]))
        .unwrap();
        assert!(cli.summon);
        assert_eq!(cli.wake_key.as_deref(), Some("ctrl+g"));
        assert!(cli.print_bind);
        assert!(cli.print_rows);
        assert_eq!(cli.print_app.as_deref(), Some("wechat"));
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
        assert!(parse_args(&args(&["--print-app"])).is_err());
        assert!(parse_args(&args(&["--shell", "fish"])).is_err());
        assert!(parse_args(&args(&["--shell"])).is_err());
        assert!(parse_args(&args(&["--bogus"])).is_err());
    }
}
