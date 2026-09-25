//! Installed applications: reading desktop entries and matching a name.
//!
//! The `app` alias resolves its input against what the desktop menu would
//! show. Reading the files is the cheap half ([`scan`]); [`keys`] is what
//! makes a name reachable the way a launcher user types it (initials, no
//! punctuation, pinyin), and [`score`]/[`best`] keep the ranking predictable
//! so the caller can either launch one match or report the ambiguity.

use std::path::{Path, PathBuf};

use super::pinyin;

mod scan;

pub use scan::{on_path, scan};

/// One installed application, as read from disk.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Desktop id: file name minus ".desktop" (`org.gnome.Nautilus`).
    pub id: String,
    /// Full path to the `.desktop` file (the `.app` bundle on macOS).
    pub path: PathBuf,
    /// Display label: `Name[<locale>]` when present, else `Name`, else the id.
    pub name: String,
    /// The plain name and the translations that fit the user's locale.
    pub names: Vec<String>,
    /// Raw `Exec=` value (`/usr/bin/code %F`), field codes included.
    pub exec: Option<String>,
    /// `Keywords` values (plain and the user's locale), split on `;`.
    pub keywords: Vec<String>,
    /// `NoDisplay=true` or `Hidden=true`.
    pub no_display: bool,
}

/// Parse one `.desktop` file into an [`Entry`]. Unknown and malformed lines
/// are ignored, so a partially understood file still answers to the keys it
/// does carry.
pub fn parse(text: &str, id: &str, path: &Path) -> Entry {
    parse_with_locale(text, id, path, locale().as_deref())
}

/// [`parse`] with an explicit locale, so the process-global environment
/// lookup stays out of the unit tests.
///
/// Only the plain name and the translations for the user's locale are kept:
/// a file lists ninety-odd locales, and each of them would otherwise answer
/// in its own spelling (the Japanese `詳細` spells `xx`) and hide the app the
/// user actually meant.
fn parse_with_locale(text: &str, id: &str, path: &Path, locale: Option<&str>) -> Entry {
    let mut localized: Option<String> = None;
    let mut language: Option<String> = None;
    let mut plain: Option<String> = None;
    let mut keywords: Vec<String> = Vec::new();
    let mut exec: Option<String> = None;
    let mut no_display = false;
    for line in entry_lines(text) {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if key == "Name" {
            plain = Some(value.to_string());
        } else if let Some(tag) = key.strip_prefix("Name[").and_then(|k| k.strip_suffix(']')) {
            if Some(tag) == locale {
                localized = Some(value.to_string());
            } else if language.is_none() && wanted_tag(locale, tag) {
                language = Some(value.to_string());
            }
        } else if key == "Keywords" || key.starts_with("Keywords[") {
            let wanted = key
                .strip_prefix("Keywords[")
                .and_then(|k| k.strip_suffix(']'))
                .is_none_or(|tag| wanted_tag(locale, tag));
            if wanted {
                for word in value.split(';').map(str::trim).filter(|w| !w.is_empty()) {
                    keywords.push(word.to_string());
                }
            }
        } else if key == "Exec" {
            exec = Some(value.to_string());
        } else if key == "NoDisplay" || key == "Hidden" {
            no_display |= value.eq_ignore_ascii_case("true");
        }
    }
    let localized = non_empty(localized);
    let language = non_empty(language);
    let plain = non_empty(plain);
    let label = localized.or(language).or(plain.clone());
    let mut names: Vec<String> = Vec::new();
    if let Some(label) = label.clone() {
        push_unique(&mut names, &label);
    }
    if let Some(plain) = plain {
        push_unique(&mut names, &plain);
    }
    Entry {
        id: id.to_string(),
        path: path.to_path_buf(),
        name: label.unwrap_or_else(|| id.to_string()),
        names,
        exec,
        keywords,
        no_display,
    }
}

/// Lowercase match keys for `entry`, deduped, one-character keys dropped.
///
/// The id (whole and dot-separated, so `org.gnome.Nautilus` answers to
/// `nautilus`), each name (whole, word by word, and with separators removed,
/// so `google chrome` also spells `googlechrome`), pinyin acronyms (`wjglq`
/// for `wen jian guan li qi`) and the executable's base name all point at the
/// entry: whatever the user calls the app, it is reachable. Keys shorter than
/// two characters are noise - the full name is already a key.
pub fn keys(entry: &Entry) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    let id = entry.id.to_lowercase();
    push_key(&mut keys, id.clone());
    for segment in id.split('.').map(str::trim).filter(|s| !s.is_empty()) {
        push_key(&mut keys, segment.to_string());
    }
    for name in &entry.names {
        let lower = name.to_lowercase();
        push_key(&mut keys, lower.clone());
        push_key(
            &mut keys,
            lower.chars().filter(|c| c.is_alphanumeric()).collect(),
        );
        for word in words(&lower) {
            push_key(&mut keys, word.to_string());
            for acronym in pinyin::acronyms(word) {
                push_key(&mut keys, acronym);
            }
        }
        for acronym in pinyin::acronyms(&lower) {
            push_key(&mut keys, acronym);
        }
    }
    if let Some(word) = entry
        .exec
        .as_deref()
        .and_then(|e| e.split_whitespace().next())
    {
        if let Some(base) = Path::new(word).file_name() {
            push_key(&mut keys, base.to_string_lossy().to_lowercase());
        }
    }
    for keyword in &entry.keywords {
        let lower = keyword.to_lowercase();
        push_key(&mut keys, lower.clone());
        for word in words(&lower) {
            push_key(&mut keys, word.to_string());
        }
    }
    keys
}

/// Match quality of `token` against `entry`: 5 whole name, 4 exact key,
/// 3 prefix in either direction, 2 substring, 0 no match.
///
/// The whole name is the strongest signal there is, and the one that keeps a
/// name from ending in an ambiguity it does not deserve: `app 文件管理器`
/// must pick the entry *called* that, not one that merely carries the words
/// as a key. The length floors keep one- and two-letter fragments from
/// matching half the machine: a two-letter token only prefixes, and a
/// substring needs three.
pub fn score(entry: &Entry, token: &str) -> u8 {
    let token = normalize(token);
    if token.is_empty() {
        return 0;
    }
    let whole_name = normalize(&entry.name);
    if whole_name == token || entry.names.iter().any(|name| normalize(name) == token) {
        return 5;
    }
    keys(entry)
        .iter()
        .map(|key| key_score(key, &token))
        .max()
        .unwrap_or(0)
}

/// Top-scoring entries for `token`, visible ones first, then label, then id.
///
/// The order is stable, so "first match wins" never depends on directory
/// listing order. An empty result means nothing matched.
pub fn best<'a>(entries: &'a [Entry], token: &str) -> Vec<&'a Entry> {
    let token = normalize(token);
    if token.is_empty() {
        return Vec::new();
    }
    let mut matches: Vec<(u8, &'a Entry)> = entries
        .iter()
        .map(|entry| (score(entry, &token), entry))
        .filter(|(score, _)| *score > 0)
        .collect();
    let Some(top) = matches.iter().map(|(score, _)| *score).max() else {
        return Vec::new();
    };
    matches.retain(|(score, _)| *score == top);
    matches.sort_by(|(_, a), (_, b)| {
        a.no_display
            .cmp(&b.no_display)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
    matches.into_iter().map(|(_, entry)| entry).collect()
}

/// Keys of the `[Desktop Entry]` group only: desktop actions and KDE extras
/// carry their own `Name`/`Exec` that must never win over the main one.
fn entry_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut in_entry = true;
    text.lines().filter_map(move |line| {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_entry = trimmed == "[Desktop Entry]";
            return None;
        }
        (in_entry && !trimmed.is_empty() && !trimmed.starts_with('#')).then_some(trimmed)
    })
}

/// First value of `key` in the `[Desktop Entry]` group.
pub(super) fn field(text: &str, key: &str) -> Option<String> {
    for line in entry_lines(text) {
        if let Some((name, value)) = line.split_once('=') {
            if name.trim() == key {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

/// Boolean desktop key (`Hidden`, `NoDisplay`, `DBusActivatable`).
pub(super) fn flag(text: &str, key: &str) -> bool {
    field(text, key).is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

/// True when the entry can run: an `Exec=` command, or DBus activation (some
/// GNOME apps ship no command and are started through their service file).
pub(super) fn launchable(text: &str) -> bool {
    let exec = field(text, "Exec").unwrap_or_default();
    !exec.trim().is_empty() || flag(text, "DBusActivatable")
}

/// Locale tag for `Name[<locale>]` lookup: `LC_ALL` wins over `LANG`, and
/// `zh_CN.UTF-8` is reduced to `zh_CN`.
fn locale() -> Option<String> {
    ["LC_ALL", "LANG"].iter().find_map(|var| {
        std::env::var(var)
            .ok()
            .and_then(|value| normalize_locale(&value))
    })
}

/// Whether a `Name[tag]` translation belongs to the user's locale: the exact
/// tag, or its bare language (`zh_CN` also accepts a lone `Name[zh]`).
fn wanted_tag(locale: Option<&str>, tag: &str) -> bool {
    locale.is_some_and(|l| l == tag || l.split('_').next() == Some(tag))
}

/// Treat an empty assignment (`Name[zh_CN]=`) as absent.
fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

/// Strip codeset and modifier off an environment locale value.
fn normalize_locale(value: &str) -> Option<String> {
    let base = value
        .trim()
        .split(['.', '@'])
        .next()
        .unwrap_or("")
        .to_string();
    if base.is_empty() || base == "C" || base == "POSIX" {
        None
    } else {
        Some(base)
    }
}

/// Fold input and keys the same way: lowercase, trimmed, whitespace runs
/// collapsed to single spaces.
fn normalize(input: &str) -> String {
    input
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn key_score(key: &str, token: &str) -> u8 {
    if key == token {
        return 4;
    }
    if key.starts_with(token) && token.chars().count() >= 2 {
        return 3;
    }
    if token.starts_with(key) && key.chars().count() >= 3 {
        return 3;
    }
    if key.contains(token) && token.chars().count() >= 3 {
        return 2;
    }
    0
}

/// Words of `text`, split on every non-alphanumeric character (spaces,
/// punctuation, dashes), so `Foo-Bar 2` spells `foo`, `bar` and `2`.
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
}

fn push_key(keys: &mut Vec<String>, key: String) {
    if key.chars().count() >= 2 && !keys.contains(&key) {
        keys.push(key);
    }
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !value.is_empty() && !values.iter().any(|v| v == value) {
        values.push(value.to_string());
    }
}

#[cfg(test)]
mod tests;
