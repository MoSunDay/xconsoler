//! Binary entry point: arg parsing, terminal lifecycle, event loop.

use std::io::stdout;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use xconsoler::action;
use xconsoler::app;
use xconsoler::platform;
use xconsoler::render;
use xconsoler::state::{self, App};
use xconsoler::storage;
use xconsoler::term::TerminalGuard;

const USAGE: &str = "\
xconsoler — long-bar keyboard launcher (Alt+D to wake)

USAGE:
    xconsoler [--store <path>]

OPTIONS:
    --store <path>    store.json location (default: <config_dir>/xconsoler/store.json)
    -h, --help        show this help";

/// Tick cap: worst-case latency before a redraw / quit check.
const POLL_TIMEOUT: Duration = Duration::from_millis(250);

fn main() {
    let path = store_path(std::env::args().collect());
    // The guard is dropped inside the closure, so the terminal is always
    // restored before the error is printed to stderr below.
    let result = (|| -> Result<()> {
        let _guard = TerminalGuard::enter()?;
        run(&path)
    })();
    if let Err(e) = result {
        eprintln!("xconsoler: {e:#}");
        std::process::exit(1);
    }
}

/// Resolve the store path from argv; `-h`/`--help` prints usage and exits.
fn store_path(args: Vec<String>) -> PathBuf {
    let mut path = storage::default_path();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--store" => match args.get(i + 1) {
                Some(p) => {
                    path = PathBuf::from(p);
                    i += 2;
                }
                None => {
                    eprintln!("--store requires a path\n\n{USAGE}");
                    std::process::exit(2);
                }
            },
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument: {other}\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }
    path
}

/// The event loop: draw (every iteration, hidden included), poll, dispatch.
fn run(path: &std::path::Path) -> Result<()> {
    let pf = platform::current();
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut app: App = state::new(storage::load(path));

    loop {
        terminal.draw(|f| render::draw(f, &app))?;
        if event::poll(POLL_TIMEOUT)? {
            if let Event::Key(key) = event::read()? {
                // Key release/repeat is filtered inside `on_key`.
                let act = action::on_key(&app, key);
                app::apply(&mut app, act, pf, path);
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
