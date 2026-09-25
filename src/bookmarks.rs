//! Chrome/Chromium bookmark import: locate the browser's `Bookmarks` file,
//! parse its JSON tree into URL entries, and turn those into concrete
//! shortcut rows (`key => url`) on an existing alias.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::alias::AliasDef;

/// Hard cap on entries imported in one run.
pub const MAX_IMPORT: usize = 300;

/// Serialises tests that touch `XC_CHROME_BOOKMARKS` (process-global env).
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One bookmark worth importing.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Bookmark title; blank titles fall back to the URL host for the key.
    pub title: String,
    /// `http`/`https` URL, never blank.
    pub url: String,
    /// Normalized parent-folder chain (`"Dev/Tools"`); empty at the top level.
    pub folder: String,
}

/// Locate this machine's Chrome/Chromium `Bookmarks` file.
///
/// `XC_CHROME_BOOKMARKS` wins when set and non-empty, even when the path does
/// not exist (the caller then reports a clear read error). Otherwise the
/// usual profile dirs are scanned: `$XDG_CONFIG_HOME/google-chrome`,
/// `~/.config/google-chrome`, `~/.config/chromium`, the Chromium snap and
/// flatpak trees, plus the macOS app-support profiles. `Default` profiles win
/// over the rest, which are tried alphabetically.
pub fn find_file() -> Option<PathBuf> {
    match std::env::var_os("XC_CHROME_BOOKMARKS") {
        Some(v) if !v.is_empty() => Some(PathBuf::from(v)),
        _ => profile_bases().iter().flat_map(|b| profiles_in(b)).next(),
    }
}

/// Baseline profile dirs in scan order; the macOS entries only on macOS.
fn profile_bases() -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            bases.push(PathBuf::from(xdg).join("google-chrome"));
        }
    }
    if let Some(home) = dirs::home_dir() {
        bases.push(home.join(".config").join("google-chrome"));
        bases.push(home.join(".config").join("chromium"));
        bases.push(home.join("snap/chromium/common/chromium"));
        bases.push(home.join(".var/app/org.chromium.Chromium/config/chromium"));
        if cfg!(target_os = "macos") {
            bases.push(home.join("Library/Application Support/Google/Chrome"));
            bases.push(home.join("Library/Application Support/Chromium"));
        }
    }
    bases
}

/// Existing `<base>/<profile>/Bookmarks` files: `Default` first, then the
/// remaining profiles alphabetically.
fn profiles_in(base: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(base) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort_by_key(|n| (n != "Default", n.clone()));
    names
        .into_iter()
        .map(|n| base.join(n).join("Bookmarks"))
        .filter(|p| p.is_file())
        .collect()
}

/// Read and parse a Chrome `Bookmarks` file.
pub fn load(path: &Path) -> Result<Vec<Entry>, String> {
    let json = fs::read_to_string(path)
        .map_err(|e| format!("cannot read bookmarks {}: {e}", path.display()))?;
    parse(&json)
}

/// Parse the Chrome bookmark tree into entries, in tree order.
///
/// Walks `roots` -> each root -> `children` recursively. `type == "url"`
/// nodes become entries, `type == "folder"` nodes recurse with their name
/// pushed onto the folder chain, anything else is ignored. Blank and
/// non-http(s) URLs are skipped, duplicates by URL keep the first
/// occurrence, and at most [`MAX_IMPORT`] entries are returned.
pub fn parse(json: &str) -> Result<Vec<Entry>, String> {
    let root: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("invalid bookmarks json: {e}"))?;
    let mut out = Vec::new();
    if let Some(roots) = root.get("roots").and_then(|v| v.as_object()) {
        for node in roots.values() {
            walk(node, "", &mut out);
        }
    }
    let mut seen = BTreeSet::new();
    out.retain(|e| seen.insert(e.url.clone()));
    out.truncate(MAX_IMPORT);
    Ok(out)
}

/// Recurse into a folder's children, extending `folder` for sub-folders.
fn walk(node: &serde_json::Value, folder: &str, out: &mut Vec<Entry>) {
    let Some(children) = node.get("children").and_then(|v| v.as_array()) else {
        return;
    };
    for child in children {
        let kind = child.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let name = child.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if kind == "url" {
            let url = child
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if is_http(url) {
                out.push(Entry {
                    title: name.to_string(),
                    url: url.to_string(),
                    folder: folder.to_string(),
                });
            }
        } else if kind == "folder" {
            walk(child, &join_folder(folder, name.trim()), out);
        }
    }
}

/// True for `http`/`https` URLs, case-insensitively.
fn is_http(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Parent chain + `name`, skipping blanks; levels are joined with `/`.
fn join_folder(parent: &str, name: &str) -> String {
    if name.is_empty() {
        parent.to_string()
    } else if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// Fresh `key => url` pairs for `entries`, skipping URLs that are already
/// registered in `existing` — a re-run of the same export adds nothing and
/// never overwrites. Keys never collide with `existing` keys or with keys
/// picked earlier in the same run.
///
/// Naming order per entry: the plain slug of the title, then `<folder>-<key>`
/// (folder chain slugified) when that is free, then `<key>-2`, `<key>-3`, ...
pub fn plan(existing: &BTreeMap<String, String>, entries: &[Entry]) -> Vec<(String, String)> {
    let mut taken: BTreeSet<String> = existing.keys().cloned().collect();
    let mut seen: BTreeSet<&str> = existing.values().map(String::as_str).collect();
    let mut fresh = Vec::new();
    for entry in entries {
        if !seen.insert(entry.url.as_str()) {
            continue; // already imported (or listed twice): leave it alone
        }
        let key = base_key(&entry.title, &entry.url);
        let folder = slug(&entry.folder);
        let candidate = if !taken.contains(&key) {
            key.clone()
        } else if !folder.is_empty() && !taken.contains(&format!("{folder}-{key}")) {
            format!("{folder}-{key}")
        } else {
            numbered(&key, &taken)
        };
        taken.insert(candidate.clone());
        fresh.push((candidate, entry.url.clone()));
    }
    fresh
}

/// First free `<key>-<n>` with `n` counting up from 2.
fn numbered(key: &str, taken: &BTreeSet<String>) -> String {
    (2..)
        .map(|n| format!("{key}-{n}"))
        .find(|c| !taken.contains(c))
        .unwrap()
}

/// Preferred key for an entry: title slug, else the URL host without a
/// leading `www.`, else the literal `bookmark`.
fn base_key(title: &str, url: &str) -> String {
    let from_title = slug(title);
    if !from_title.is_empty() {
        return from_title;
    }
    let host = host_of(url);
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let from_host = slug(host);
    if from_host.is_empty() {
        "bookmark".to_string()
    } else {
        from_host
    }
}

/// Lowercase ASCII slug: runs of characters outside `[a-z0-9._-]` become a
/// single `-`, which is also trimmed from both ends.
fn slug(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-') {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with(['-', '.']) {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Host part of an http(s) URL: authority up to the path, without the
/// `user@` prefix or `:port` suffix ("" when there is no authority).
fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or("");
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = rest[..end].rsplit('@').next().unwrap_or("");
    host.split(':').next().unwrap_or(host).to_string()
}

/// Merge `fresh` pairs into alias `target` in `aliases`. Existing keys are
/// left untouched; returns how many pairs were actually added. `Err` when
/// `target` does not exist.
pub fn merge_into(
    aliases: &mut [AliasDef],
    target: &str,
    fresh: &[(String, String)],
) -> Result<usize, String> {
    let Some(idx) = aliases.iter().position(|d| d.name == target) else {
        return Err(format!("alias not found: {target}"));
    };
    let def = &mut aliases[idx];
    let mut added = 0;
    for (key, value) in fresh {
        if !def.shortcuts.contains_key(key) {
            def.shortcuts.insert(key.clone(), value.clone());
            added += 1;
        }
    }
    Ok(added)
}

/// Load `path`, plan against `target`'s current keys, and merge the result.
/// The entry point app code and tests share; [`find_file`] only locates the
/// file, so tests can inject a fixture path directly.
pub fn plan_and_merge(
    aliases: &mut [AliasDef],
    target: &str,
    path: &Path,
) -> Result<usize, String> {
    let entries = load(path)?;
    let existing = aliases
        .iter()
        .find(|d| d.name == target)
        .map(|d| d.shortcuts.clone())
        .ok_or_else(|| format!("alias not found: {target}"))?;
    merge_into(aliases, target, &plan(&existing, &entries))
}

#[cfg(test)]
mod tests;
