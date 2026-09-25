//! Finding the installed applications: XDG directories, Flatpak/Snap exports
//! and macOS bundles.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::{field, flag, launchable, parse, Entry};
use crate::platform::Platform;

/// Applications installed for `platform`, in menu precedence order.
///
/// Linux: the XDG application directories, then Flatpak and Snap exports.
/// The first directory carrying an id wins, and `Hidden=true` masks that id
/// everywhere - the desktop spec's way to suppress a system entry, so a
/// user's own copy shadows `/usr/share` without editing it. Entries that
/// cannot run or cannot be read are skipped silently: one malformed file
/// must never break the launcher.
pub fn scan(platform: Platform) -> Vec<Entry> {
    match platform {
        Platform::Linux => scan_dirs(&linux_dirs()),
        Platform::Macos => scan_macos(),
    }
}

/// True when `program` names an executable file on `$PATH`. Spawning
/// `which`/`command -v` would add a process per lookup and answer a
/// different question (the user's shell aliases and functions).
pub fn on_path(program: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| is_executable(&dir.join(program)))
}

/// Linux scan over explicit directories, in precedence order. Kept separate
/// from the environment lookup so tests can drive it with temp dirs.
fn scan_dirs(dirs: &[PathBuf]) -> Vec<Entry> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut entries = Vec::new();
    for dir in dirs {
        let Ok(reader) = fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = reader
            .filter_map(|item| item.ok())
            .map(|item| item.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "desktop"))
            .collect();
        paths.sort(); // reproducible "first wins" within one directory
        for path in paths {
            let Some(id) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                continue;
            };
            // First directory wins; a Hidden file takes the id out of the
            // list for good, the way the desktop spec masks lower entries.
            if !seen.insert(id.clone()) {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            if flag(&text, "Hidden") || !launchable(&text) {
                continue;
            }
            if let Some(binary) = field(&text, "TryExec").filter(|b| !b.is_empty()) {
                if !on_path(&binary) {
                    continue;
                }
            }
            entries.push(parse(&text, &id, &path));
        }
    }
    entries
}

/// The XDG application directories for the current environment, in
/// precedence order, followed by the Flatpak and Snap export directories.
fn linux_dirs() -> Vec<PathBuf> {
    let home = dirs::home_dir();
    let mut dirs = Vec::new();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".local/share")));
    if let Some(dir) = data_home {
        dirs.push(dir.join("applications"));
    }
    match std::env::var_os("XDG_DATA_DIRS").filter(|dirs| !dirs.is_empty()) {
        Some(data_dirs) => {
            dirs.extend(std::env::split_paths(&data_dirs).map(|dir| dir.join("applications")));
        }
        None => {
            dirs.extend(
                ["/usr/local/share", "/usr/share"]
                    .map(|dir| PathBuf::from(dir).join("applications")),
            );
        }
    }
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    if let Some(home) = &home {
        dirs.push(home.join(".local/share/flatpak/exports/share/applications"));
    }
    dirs.push(PathBuf::from("/var/lib/snapd/desktop/applications"));
    dirs
}

/// macOS scan: `*.app` bundles one and two levels under the application
/// directories. Bundles carry no desktop file, so only the name is filled in.
fn scan_macos() -> Vec<Entry> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Applications"));
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut entries = Vec::new();
    for root in roots {
        for bundle in app_bundles(&root) {
            let Some(id) = bundle.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                continue;
            };
            if !seen.insert(id.clone()) {
                continue;
            }
            let name = id.clone();
            entries.push(Entry {
                id,
                path: bundle,
                name: name.clone(),
                names: vec![name],
                exec: None,
                keywords: Vec::new(),
                no_display: false,
            });
        }
    }
    entries
}

/// `*.app` bundles directly under `root` and one directory deeper
/// (`/Applications/Dev/X.app`).
fn app_bundles(root: &Path) -> Vec<PathBuf> {
    let mut bundles = Vec::new();
    let Ok(children) = fs::read_dir(root) else {
        return bundles;
    };
    for child in children.flatten() {
        let path = child.path();
        if is_app(&path) {
            bundles.push(path);
        } else if path.is_dir() {
            if let Ok(grandchildren) = fs::read_dir(&path) {
                bundles.extend(
                    grandchildren
                        .flatten()
                        .map(|item| item.path())
                        .filter(|path| is_app(path)),
                );
            }
        }
    }
    bundles.sort();
    bundles
}

fn is_app(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "app")
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.is_file()
        && path
            .metadata()
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(test)]
mod tests;
