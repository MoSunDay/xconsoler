//! RAII terminal lifecycle (simplified from the opencoder `terminal.rs`
//! pattern).
//!
//! Setup/teardown used to be the kind of thing done inline at the top and
//! bottom of a `run` function — which means teardown only ran on the happy
//! path. A panic (or an early `?`) unwound straight past it and left the
//! terminal in raw mode + alternate screen: to the user that looks like a
//! frozen program — last frame stuck, typing has no echo, Ctrl+C dead — and
//! needs a process kill plus `reset` to recover.
//!
//! [`TerminalGuard`] makes restoration an RAII invariant: `Drop` runs on
//! every exit path, and a panic hook restores *before* the previous hook
//! prints, so the message lands in a sane terminal instead of inside the
//! alternate screen.

use std::sync::Once;

use anyhow::Result;
use crossterm::cursor::{Hide, Show};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};

/// RAII handle holding the terminal in TUI mode (raw + alternate screen +
/// hidden cursor). Construct with [`TerminalGuard::enter`]; drop to restore.
pub struct TerminalGuard;

impl TerminalGuard {
    /// Put the terminal into TUI mode and install the panic hook. On any
    /// setup failure raw mode is rolled back so the process can exit into a
    /// usable shell.
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;
        if let Err(e) = execute!(std::io::stdout(), EnterAlternateScreen, Hide) {
            let _ = disable_raw_mode();
            return Err(e.into());
        }
        install_panic_hook();
        Ok(TerminalGuard)
    }

    /// Best-effort, idempotent restoration. Swallows its own errors so it is
    /// safe to call from a panic hook and from `Drop`.
    fn restore() {
        let _ = execute!(std::io::stdout(), Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        TerminalGuard::restore();
    }
}

/// Install the restore-then-chain panic hook exactly once per process.
///
/// The previous hook is captured and called *after* restoration: the default
/// hook prints the panic to stderr, which is only readable once the alternate
/// screen has been left and raw mode disabled. Host-installed hooks chained
/// through `prev` still run.
fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            TerminalGuard::restore();
            prev(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_is_a_unit_raii_token() {
        // No fields, no state: Drop is the whole contract. (enter() itself
        // needs a real terminal, so it is exercised manually, not here.)
        let _guard = TerminalGuard;
    }
}
