//! Disk cache for the Linux desktop-entry scan.
//!
//! The bar is a per-Enter process - summon, run one command, exit - so a
//! scan can never be amortized in memory: every launch would pay the full
//! walk of the application directories (plus a `TryExec` probe per entry)
//! again. Instead the scan result is written to [`cache_path`] once and
//! served on every later launch. The fingerprint stamps each directory
//! with its mtime and the name, size and mtime of every `.desktop` file,
//! so installing, updating or removing an application changes it; [`load`]
//! answers `None` on any difference or damage, which sends the caller back
//! to a full scan. A stale or corrupt cache therefore heals itself on the
//! next run - the worst a broken file can cost is one slow scan.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::Entry;

/// Cache schema version. A file stamped with any other version is a miss,
/// so a format change never serves incompatible entries.
const VERSION: u32 = 1;

/// One `.desktop` file as the fingerprint sees it: its name, its mtime as
/// `(secs, nanos)` since the UNIX epoch (`None` when it cannot be stat'ed)
/// and its size in bytes.
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
struct FileStamp {
    name: String,
    mtime: Option<(u64, u32)>,
    len: u64,
}

/// One scanned directory: the directory, its mtime (which moves whenever a
/// file is created or removed inside it) and its `.desktop` files sorted by
/// name - the order the scan itself visits them, so both sides of the
/// comparison stamp the directory identically.
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub(super) struct DirStamp {
    dir: PathBuf,
    mtime: Option<(u64, u32)>,
    files: Vec<FileStamp>,
}

/// The cache document: the schema version, the fingerprint the entries were
/// scanned under, and the entries ready to serve without touching a desktop
/// file (keys precomputed, `TryExec` probes already passed).
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
struct Cache {
    version: u32,
    fingerprint: Vec<DirStamp>,
    entries: Vec<Entry>,
}

/// Stamp `dirs` for a cache check, one [`DirStamp`] per directory. A missing
/// directory keeps its place with `None` mtime and no files, so it still
/// differs from a directory that existed when the cache was written.
pub(super) fn fingerprint(dirs: &[PathBuf]) -> Vec<DirStamp> {
    dirs.iter()
        .map(|dir| DirStamp {
            dir: dir.clone(),
            mtime: stat(dir).0,
            files: desktop_stamps(dir),
        })
        .collect()
}

/// The directory's `.desktop` files stamped and sorted by name, mirroring
/// the scan's path sort so stamp order equals scan order. Anything else in
/// the directory (icons, `*.directory`, editor backups) is invisible to the
/// fingerprint.
fn desktop_stamps(dir: &Path) -> Vec<FileStamp> {
    let Ok(reader) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut stamps: Vec<FileStamp> = reader
        .filter_map(|item| item.ok())
        .filter(|item| item.path().extension().is_some_and(|ext| ext == "desktop"))
        .map(|item| {
            let (mtime, len) = stat(&item.path());
            FileStamp {
                name: item.file_name().to_string_lossy().into_owned(),
                mtime,
                len,
            }
        })
        .collect();
    stamps.sort_by(|a, b| a.name.cmp(&b.name));
    stamps
}

/// `(mtime, len)` of `path`: the mtime as `(secs, nanos)` since the UNIX
/// epoch, `None` when the path cannot be stat'ed; the length then reads as
/// `0`. One helper serves files and directories alike.
fn stat(path: &Path) -> (Option<(u64, u32)>, u64) {
    let Ok(meta) = fs::metadata(path) else {
        return (None, 0);
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|age| (age.as_secs(), age.subsec_nanos()));
    (mtime, meta.len())
}

/// The cached `entries` when `path` holds a readable, well-formed cache at
/// the current [`VERSION`] stamped with exactly `stamps`; `None` on any
/// other outcome (missing file, corrupt JSON, foreign version, stale
/// fingerprint). Silent by design: a miss is an everyday event, not an
/// error worth a message.
pub(super) fn load(path: &Path, stamps: &[DirStamp]) -> Option<Vec<Entry>> {
    let text = fs::read_to_string(path).ok()?;
    let cache: Cache = serde_json::from_str(&text).ok()?;
    (cache.version == VERSION && cache.fingerprint == stamps).then_some(cache.entries)
}

/// Write the scan result best-effort: serialize, drop it on the sibling
/// `<path>.tmp`, rename over `path` - the same atomic move the store uses.
/// Errors are ignored, the cache only ever speeds things up, and the tmp
/// file is removed when the rename fails. An empty first scan writes
/// nothing, so a machine without applications stays cache-file-free.
pub(super) fn store(path: &Path, stamps: &[DirStamp], entries: &[Entry]) {
    if entries.is_empty() && !path.exists() {
        return;
    }
    let cache = Cache {
        version: VERSION,
        fingerprint: stamps.to_vec(),
        entries: entries.to_vec(),
    };
    let Ok(json) = serde_json::to_string(&cache) else {
        return;
    };
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        let _ = fs::create_dir_all(parent);
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    if fs::write(&tmp, json.as_bytes()).is_err() {
        return;
    }
    if fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

/// Cache file location: `<config_dir>/app-scan.json`, sitting next to
/// `store.json` in the xconsoler config directory.
pub(super) fn cache_path() -> PathBuf {
    crate::storage::config_dir().join("app-scan.json")
}

#[cfg(test)]
mod tests;
