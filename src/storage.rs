//! Persistent store: user aliases and history entries (JSON on disk).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::alias::{self, AliasDef};
use crate::keyspec;

/// Hard cap on stored history entries. Only the newest entries are kept:
/// the store is trimmed after every load and every record, so the file
/// stays small and startup never pays for a long history.
pub const MAX_HISTORY: usize = 100;

/// Current on-disk schema version.
///
/// Version 5 base64-encodes every `"shortcuts"` value on disk (values can
/// carry tokens, URLs and paths), mirroring the `input_b64` history field;
/// [`load`] decodes them again, and older stores keep their plaintext
/// values until the next save encodes them. Version 4 seeded the `app`
/// alias (native application launcher), which older snapshots get appended
/// by [`migrate`]. Version 3 renamed the alias JSON keys to the UI
/// vocabulary (`"triggers"` for the trigger words, `"shortcuts"` for the
/// key → value map); the manual `AliasDef` deserializer normalizes legacy
/// keys while loading. Version 2 was the first full snapshot: stores older
/// than 2 carry *overrides* of the seeded defaults only, so [`load`] merges
/// the defaults back in for them; any store older than this constant is
/// bumped to it.
pub const SCHEMA_VERSION: u32 = 5;

/// First schema version whose files carry base64-encoded shortcut values;
/// stores at or above it are decoded on load (see [`accept`]). Kept apart
/// from [`SCHEMA_VERSION`] so later bumps still gate on the right version.
const SHORTCUTS_B64_VERSION: u32 = 5;

/// One recorded execution. The input is stored base64-encoded so arbitrary
/// text (quotes, newlines, unicode) survives the JSON roundtrip untouched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub alias: String,
    pub input_b64: String,
    pub ts: u64,
}

impl HistoryEntry {
    /// Build an entry, base64-encoding the plain input.
    pub fn new(alias: &str, input: &str, ts: u64) -> Self {
        HistoryEntry {
            alias: alias.to_string(),
            input_b64: encode_b64(input),
            ts,
        }
    }

    /// Decode the stored input.
    pub fn input(&self) -> String {
        decode_b64(&self.input_b64)
    }

    /// Dedup key: `alias` + NUL + plain input.
    pub fn key(&self) -> String {
        format!("{}\u{0}{}", self.alias, self.input())
    }
}

/// User preferences persisted alongside the aliases/history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// Wake-key spec string (`keyspec::DEFAULT_SPEC` when absent).
    pub wake_key: String,
    /// Command-palette key spec (`keyspec::DEFAULT_COMMAND_SPEC` when absent).
    #[serde(default = "default_command_key")]
    pub command_key: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            wake_key: keyspec::DEFAULT_SPEC.to_string(),
            command_key: default_command_key(),
        }
    }
}

/// Serde default for [`Config::command_key`]: the wake key's own default.
fn default_command_key() -> String {
    keyspec::DEFAULT_COMMAND_SPEC.to_string()
}

/// Persisted state. `aliases` holds exactly what the store carries: a fresh
/// store is seeded with [`alias::defaults`] (`br` and `cd`), and every entry,
/// seeded or not, is editable and deletable like any other. Older stores are
/// migrated on load (see [`load`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Store {
    /// `#[serde(default)]`: stores written before the config existed load
    /// with the default wake key.
    #[serde(default)]
    pub config: Config,
    pub aliases: Vec<AliasDef>,
    pub history: Vec<HistoryEntry>,
    /// `#[serde(default)]`: stores written before versioning load as `0` and
    /// are migrated by [`load`].
    #[serde(default)]
    pub version: u32,
    /// Runtime-only, never serialized: [`load`] sets this when the file
    /// declares a version newer than [`SCHEMA_VERSION`], and [`save`] then
    /// refuses to write, so an older build never downgrades a newer store.
    #[serde(skip)]
    pub from_newer_version: bool,
    /// Runtime-only, never serialized: [`load`] records here what it had to
    /// do to the file on disk (currently: move a corrupt store aside) so the
    /// TUI can show a one-time status line. [`crate::state::new`] takes it.
    #[serde(skip)]
    pub load_notice: Option<String>,
}

impl Default for Store {
    fn default() -> Self {
        Store {
            config: Config::default(),
            aliases: alias::defaults(),
            history: Vec::new(),
            version: SCHEMA_VERSION,
            from_newer_version: false,
            load_notice: None,
        }
    }
}

/// Validate a wake-key spec and persist its canonical form
/// (`keyspec::describe`) into the store.
pub fn set_wake_key(store: &mut Store, spec: &str) -> Result<(), String> {
    let parsed = keyspec::parse(spec)?;
    store.config.wake_key = keyspec::describe(&parsed);
    Ok(())
}

/// Validate a command-palette key spec and persist its canonical form
/// (`keyspec::describe`) into the store.
pub fn set_command_key(store: &mut Store, spec: &str) -> Result<(), String> {
    let parsed = keyspec::parse(spec)?;
    store.config.command_key = keyspec::describe(&parsed);
    Ok(())
}

/// Config directory for xconsoler: `$HOME/xconsoler` (or `./xconsoler` when
/// `HOME` is not set).
pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("xconsoler")
}

/// Default store location: `$HOME/xconsoler/store.json`.
pub fn default_path() -> PathBuf {
    config_dir().join("store.json")
}

/// Legacy store location used before the default moved under `$HOME`:
/// `<dirs::config_dir()>/xconsoler/store.json` (on Linux usually
/// `~/.config/xconsoler/store.json`). `None` when the config directory
/// cannot be determined.
fn legacy_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("xconsoler").join("store.json"))
}

/// Load the store.
///
/// A missing *default* `path` falls back once to the legacy location
/// ([`legacy_path`]) so an upgrade keeps the aliases from the old default;
/// what was found is best-effort written back to `path`. A custom `--store`
/// path never consults the legacy file, so isolated stores cannot pick up
/// the user's real aliases. Without a usable legacy file a fresh store
/// seeded with [`alias::defaults`] is written instead, so the next start
/// finds a real file. Corrupt JSON at `path` is moved aside to
/// `<path>.corrupt` and a fresh seeded store is written to `path` right
/// away; the legacy location is not consulted in that case. Stores older
/// than [`SCHEMA_VERSION`] are migrated in memory (see [`accept`]); stores
/// from a newer build are returned verbatim and marked `from_newer_version`.
pub fn load(path: &Path) -> Store {
    let legacy = legacy_fallback(path, legacy_path());
    load_with_legacy(path, legacy.as_deref())
}

/// Which legacy file a missing `path` may fall back to: only the default
/// store location does, and never the same file twice.
fn legacy_fallback(path: &Path, legacy: Option<PathBuf>) -> Option<PathBuf> {
    if path != default_path() {
        return None;
    }
    legacy.filter(|old| old.as_path() != path)
}

/// [`load`] without the process environment: `legacy` is the fallback tried
/// when `path` cannot be read, so the policy can be tested directly.
fn load_with_legacy(path: &Path, legacy: Option<&Path>) -> Store {
    let data = match fs::read_to_string(path) {
        Ok(d) => d,
        Err(_) => return read_fallback(path, legacy),
    };
    match serde_json::from_str::<Store>(&data) {
        Ok(store) => accept(store),
        Err(err) => match declared_version(&data) {
            // Valid JSON from a newer build whose shape this build cannot
            // parse: leave it in place instead of renaming it `.corrupt`,
            // and run on read-only defaults so the exit save cannot clobber
            // it either.
            Some(version) if version > SCHEMA_VERSION => {
                eprintln!(
                    "xconsoler: {} was written by a newer xconsoler (store version {}); \
                     leaving it untouched",
                    path.display(),
                    version
                );
                Store {
                    from_newer_version: true,
                    ..Store::default()
                }
            }
            _ => {
                eprintln!(
                    "xconsoler: {} could not be parsed ({err}); moving it aside",
                    path.display()
                );
                let aside = sibling_path(path, ".corrupt");
                let moved = fs::rename(path, &aside).is_ok();
                let store = Store {
                    load_notice: Some(if moved {
                        format!("corrupt store moved to {}", aside.display())
                    } else {
                        format!("store could not be parsed; kept at {}", path.display())
                    }),
                    ..Store::default()
                };
                let _ = save(path, &store);
                store
            }
        },
    }
}

/// Missing `path`: prefer a parseable store at the legacy location, else seed
/// a fresh one. Whatever is returned is best-effort persisted to `path`; a
/// corrupt legacy file is ignored and left untouched.
fn read_fallback(path: &Path, legacy: Option<&Path>) -> Store {
    if let Some(old) = legacy {
        if let Ok(data) = fs::read_to_string(old) {
            if let Ok(store) = serde_json::from_str::<Store>(&data) {
                let store = accept(store);
                if !store.from_newer_version {
                    let _ = save(path, &store);
                }
                return store;
            }
        }
    }
    let store = Store::default();
    let _ = save(path, &store);
    store
}

/// Shared version migration: stores older than version 2 hold only overrides
/// of the seeded defaults, so the defaults are merged back in first; version 4
/// seeded a new `app` alias, so older snapshots get it appended unless they
/// already define that name; every older version is then bumped to
/// [`SCHEMA_VERSION`]. Version 5 needs no in-memory transform here: shortcut
/// values are plaintext in memory at every version, and only the on-disk
/// encoding changed, which [`save`] applies. The alias deserializer has
/// already normalized legacy field names in memory.
fn migrate(store: &mut Store) {
    if store.version < 2 {
        store.aliases = merge_defaults(std::mem::take(&mut store.aliases));
    }
    if store.version < 4 {
        append_default_alias(&mut store.aliases, crate::launch::default_def());
    }
    if store.version < SCHEMA_VERSION {
        store.version = SCHEMA_VERSION;
    }
}

/// Backfill one newly seeded default (named `def`) into an old store unless an
/// alias of that name already exists; existing definitions are never touched.
fn append_default_alias(aliases: &mut Vec<AliasDef>, def: AliasDef) {
    let taken = aliases
        .iter()
        .any(|d| d.name.eq_ignore_ascii_case(&def.name));
    if !taken {
        aliases.push(def);
    }
}

/// Finish a parsed store: shortcut values are base64-decoded when the file
/// already uses the v5 encoding, older versions are migrated (seeded
/// defaults come first, same-name stored aliases replace them in place, and
/// any other stored names are appended) and history is trimmed. A store from
/// a newer build is returned verbatim and marked read-only. Freshly seeded
/// stores never pass through here, so their plaintext seeds survive until
/// the next save encodes them.
fn accept(mut store: Store) -> Store {
    if store.version > SCHEMA_VERSION {
        store.from_newer_version = true;
        return store;
    }
    if store.version >= SHORTCUTS_B64_VERSION {
        decode_shortcut_values(&mut store);
    }
    migrate(&mut store);
    trim_history(&mut store.history);
    store
}

/// Shortcut values are stored base64-encoded as of [`SHORTCUTS_B64_VERSION`];
/// decode them into the plaintext form every consumer works with.
/// [`decode_b64`] falls back to the raw string, so a hand-edited plaintext
/// value still loads unchanged.
fn decode_shortcut_values(store: &mut Store) {
    for def in &mut store.aliases {
        for value in def.shortcuts.values_mut() {
            *value = decode_b64(value);
        }
    }
}

/// `version` field of a file that failed the typed parse, when it is still
/// valid JSON. Lets [`load_with_legacy`] tell a future schema apart from
/// ordinary corruption.
fn declared_version(data: &str) -> Option<u32> {
    serde_json::from_str::<serde_json::Value>(data)
        .ok()?
        .get("version")?
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
}

/// Drop all but the newest [`MAX_HISTORY`] entries (history is newest first).
/// Applied on load and on record, so an oversized store shrinks the first
/// time it is read.
pub fn trim_history(history: &mut Vec<HistoryEntry>) {
    history.truncate(MAX_HISTORY);
}

/// Serialize pretty JSON and atomically replace `path` (tmp file + rename).
pub fn save(path: &Path, store: &Store) -> anyhow::Result<()> {
    if store.from_newer_version {
        anyhow::bail!(
            "refusing to overwrite {}: it was written by a newer xconsoler (store version {})",
            path.display(),
            store.version
        );
    }
    let json = serde_json::to_string_pretty(store)?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }
    let tmp = sibling_path(path, ".tmp");
    fs::write(&tmp, json.as_bytes())
        .with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("failed to move {} into place", tmp.display()))
}

/// Migration helper: seeded defaults first, same-name stored aliases replace
/// them in place, and any other stored names are appended at the end.
fn merge_defaults(stored: Vec<AliasDef>) -> Vec<AliasDef> {
    let mut merged = alias::defaults();
    for def in stored {
        match merged.iter().position(|d| d.name == def.name) {
            Some(i) => merged[i] = def,
            None => merged.push(def),
        }
    }
    merged
}

/// Base64-encode (standard alphabet) for storage.
pub fn encode_b64(s: &str) -> String {
    STANDARD.encode(s.as_bytes())
}

/// Base64-decode; on invalid input the original string is returned as-is.
pub fn decode_b64(s: &str) -> String {
    STANDARD
        .decode(s.as_bytes())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_else(|| s.to_string())
}

/// `#[serde(serialize_with)]` helper for the `AliasDef::shortcuts` field:
/// writes the map with every value base64-encoded (store schema v5+, see
/// [`SHORTCUTS_B64_VERSION`]); the keys are plain key specs and stay
/// readable. In memory the values are always plaintext.
pub fn serialize_shortcuts<S>(
    map: &BTreeMap<String, String>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let encoded: BTreeMap<&str, String> = map
        .iter()
        .map(|(key, value)| (key.as_str(), encode_b64(value)))
        .collect();
    encoded.serialize(serializer)
}

fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

#[cfg(test)]
mod migration_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod version_tests;
