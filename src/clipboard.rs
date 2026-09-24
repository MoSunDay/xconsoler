//! Native clipboard backend for the built-in `cd` alias.
//!
//! Alias templates equal to [`TEMPLATE`] are dispatched here by
//! [`crate::exec::run_alias`] instead of `sh -c`, so the feature needs no
//! external tools (`xclip` / `wl-copy` / `xsel` / `pbcopy`) and no system
//! clipboard libraries: the `arboard` crate talks to X11 through pure-Rust
//! `x11rb` (XWayland sessions included) and to the macOS pasteboard natively.
//!
//! Lifetime of the copied text:
//!
//! * macOS - the pasteboard server owns the content; nothing to hold.
//! * Linux/X11 - the selection owner must stay alive to serve paste
//!   requests. `copy` validates and publishes in-process, then spawns a
//!   detached child ([`SERVE_FLAG`]) that re-claims the selection and
//!   serves it for up to [`SERVE_SECS`], so the text survives xconsoler
//!   exiting (summon mode) even without a clipboard manager.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Marker template handled by this module instead of the shell.
pub const TEMPLATE: &str = "@native clipboard";

/// Hidden argv flag of the detached server child.
pub const SERVE_FLAG: &str = "--internal-clipboard-serve";

/// How long the detached child keeps serving the X11 selection.
const SERVE_SECS: u64 = 3600;

/// True when an alias template selects the native clipboard backend.
pub fn is_native(template: &str) -> bool {
    template.trim() == TEMPLATE
}

/// Copy `text` to the system clipboard without external tools.
///
/// The in-process copy runs first so connection errors surface in the
/// status line; on Linux a detached child is then spawned to keep owning
/// the X11 selection after this process exits.
pub fn copy(text: &str) -> Result<(), String> {
    set_text(text).map_err(describe)?;
    #[cfg(target_os = "linux")]
    let _ = spawn_server(text);
    Ok(())
}

/// Entry point of the detached server child. Reads the payload from stdin,
/// claims the selection, then parks until [`SERVE_SECS`] elapse. Detached
/// children have no terminal, so failures stay silent by design.
pub fn serve() {
    let mut text = String::new();
    let _ = std::io::stdin().read_to_string(&mut text);
    let _ = hold(&text);
}

/// Publish `text` through arboard; drops the owner right after (on Linux
/// the detached child re-claims it, see [`copy`]).
fn set_text(text: &str) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    cb.set_text(text.to_string()).map_err(|e| e.to_string())
}

/// Claim the selection and keep the owner alive while parking.
fn hold(text: &str) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    cb.set_text(text.to_string()).map_err(|e| e.to_string())?;
    for _ in 0..SERVE_SECS {
        std::thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

/// Decorate arboard errors with a hint when no display is reachable at all.
fn describe(err: String) -> String {
    let headless =
        std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none();
    let msg = format!("clipboard: {err}");
    if headless {
        format!(
            "{msg} (no display found; X11/XWayland required, \
             or override with `:add cd <cmd>`)"
        )
    } else {
        msg
    }
}

/// Spawn the detached re-serve child (Linux only). Errors are ignored:
/// the in-process copy already succeeded and keeps serving for as long
/// as xconsoler itself lives.
#[cfg(target_os = "linux")]
fn spawn_server(text: &str) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;

    let exe = std::env::current_exe()?;
    let mut child = Command::new(exe)
        .arg(SERVE_FLAG)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Own session/process group: outlives xconsoler, no job control.
        .process_group(0)
        .spawn()?;
    // Snippets are short, so the write cannot fill the pipe buffer.
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(text.as_bytes());
    }
    drop(child); // leaked on purpose: the child must outlive this process
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_matches_exactly_and_tolerates_padding() {
        assert!(is_native(TEMPLATE));
        assert!(is_native("  @native clipboard  "));
        assert!(!is_native("@native clipboard extra"));
        assert!(!is_native("wl-copy @stdin"));
        assert!(!is_native(""));
    }

    #[test]
    fn flag_and_template_are_hidden_from_users() {
        // The flag must not collide with the public CLI surface.
        assert!(SERVE_FLAG.starts_with("--internal-"));
        assert_eq!(TEMPLATE, "@native clipboard");
    }

    #[test]
    fn describe_prefixes_error() {
        // No env mutation here (tests run in parallel): only check shape.
        let msg = describe("boom".to_string());
        assert!(msg.starts_with("clipboard: boom"), "{msg}");
    }
}
